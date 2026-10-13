//! `tracing_subscriber::Layer` implementation.

use core::cell::RefCell;
use core::fmt::{self, Write as _};
use core::ops::Range;

use tracing_core::field::{Field as TracingField, Visit};
use tracing_core::span::{Attributes, Id, Record as SpanRecord};
use tracing_core::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use crate::level::Level;
use crate::logger::Logger;
use crate::record::{Field, Metadata};
use crate::value::{OwnedField, Value, write_error_chain, write_hex};

/// A `tracing` layer that writes events through a [`Logger`].
///
/// * Event levels map one to one; the event's target, file and line are
///   preserved.
/// * The `message` field becomes the record message; every other event
///   field becomes a record field. Integers, floats and booleans keep
///   their type; strings, `Debug` values and errors are rendered to
///   text.
/// * Fields of the spans enclosing the event (root first) are attached
///   as context. A field on an inner span shadows the same field on an
///   outer span, and event fields shadow span fields.
/// * The logger's filter is applied per event. The layer does not
///   implement `Layer::enabled`, because returning `false` there would
///   disable the event for every other layer too; combine it with
///   `tracing-subscriber`'s own filters if you need callsite-level
///   filtering.
///
/// Event fields are collected into a reused thread-local buffer, so an
/// event does not allocate in steady state; each span's fields are
/// stored once, when the span is created or recorded.
///
/// The layer needs a subscriber that stores spans, such as
/// `tracing_subscriber::Registry` (enable `tracing-subscriber`'s
/// `registry` feature in your own `Cargo.toml`).
///
/// # Examples
///
/// ```
/// use log_io::compat::TracingLayer;
/// use log_io::{Level, Logger};
/// use tracing_subscriber::layer::SubscriberExt;
///
/// let logger = Logger::builder().level(Level::Info).stdout_json().build();
/// let subscriber = tracing_subscriber::registry().with(TracingLayer::new(logger));
///
/// tracing::subscriber::with_default(subscriber, || {
///     let span = tracing::info_span!("request", id = 7);
///     let _entered = span.enter();
///     tracing::info!(status = 200, "handled");
/// });
/// ```
#[derive(Debug, Clone)]
pub struct TracingLayer {
    logger: Logger,
}

impl TracingLayer {
    /// Wrap `logger`.
    #[must_use]
    pub const fn new(logger: Logger) -> Self {
        Self { logger }
    }

    /// The wrapped logger.
    #[must_use]
    pub const fn logger(&self) -> &Logger {
        &self.logger
    }
}

const fn from_tracing_level(level: tracing_core::Level) -> Level {
    match level {
        tracing_core::Level::ERROR => Level::Error,
        tracing_core::Level::WARN => Level::Warn,
        tracing_core::Level::INFO => Level::Info,
        tracing_core::Level::DEBUG => Level::Debug,
        tracing_core::Level::TRACE => Level::Trace,
    }
}

/// Owned fields stored in a span's extensions.
#[derive(Default)]
struct SpanFields(Vec<OwnedField>);

impl SpanFields {
    fn set(&mut self, key: &'static str, value: Value<'_>) {
        let owned = OwnedField::new(key, value);
        if let Some(existing) = self.0.iter_mut().find(|f| &*f.key == key) {
            *existing = owned;
        } else {
            self.0.push(owned);
        }
    }
}

impl Visit for SpanFields {
    fn record_f64(&mut self, field: &TracingField, value: f64) {
        self.set(field.name(), Value::F64(value));
    }
    fn record_i64(&mut self, field: &TracingField, value: i64) {
        self.set(field.name(), Value::I64(value));
    }
    fn record_u64(&mut self, field: &TracingField, value: u64) {
        self.set(field.name(), Value::U64(value));
    }
    fn record_i128(&mut self, field: &TracingField, value: i128) {
        self.set(field.name(), Value::I128(value));
    }
    fn record_u128(&mut self, field: &TracingField, value: u128) {
        self.set(field.name(), Value::U128(value));
    }
    fn record_bool(&mut self, field: &TracingField, value: bool) {
        self.set(field.name(), Value::Bool(value));
    }
    fn record_str(&mut self, field: &TracingField, value: &str) {
        self.set(field.name(), Value::Str(value));
    }
    fn record_bytes(&mut self, field: &TracingField, value: &[u8]) {
        self.set(field.name(), Value::Bytes(value));
    }
    fn record_error(&mut self, field: &TracingField, value: &(dyn std::error::Error + 'static)) {
        self.set(field.name(), Value::Error(value));
    }
    fn record_debug(&mut self, field: &TracingField, value: &dyn fmt::Debug) {
        self.set(field.name(), Value::Debug(value));
    }
}

/// Where a collected value lives.
#[derive(Clone)]
enum Slot {
    /// A value with no borrowed data.
    Scalar(Value<'static>),
    /// Text in the arena.
    Text(Range<usize>),
}

/// Key of a collected field.
#[derive(Clone)]
enum Key {
    Static(&'static str),
    Text(Range<usize>),
}

/// Event fields and span fields for one event, with all text in a
/// single reusable buffer.
struct Arena {
    text: String,
    message: Option<Range<usize>>,
    event: Vec<(Key, Slot)>,
    /// Span fields, root span first.
    spans: Vec<(Key, Slot)>,
    /// Span fields as collected, leaf span first, with the start index
    /// of each span's block. Reordered into `spans` afterwards; walking
    /// the scope leaf-first avoids the allocation in `Scope::from_root`.
    scratch: Vec<(Key, Slot)>,
    blocks: Vec<usize>,
}

impl Arena {
    const fn new() -> Self {
        Self {
            text: String::new(),
            message: None,
            event: Vec::new(),
            spans: Vec::new(),
            scratch: Vec::new(),
            blocks: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.text.clear();
        self.message = None;
        self.event.clear();
        self.spans.clear();
        self.scratch.clear();
        self.blocks.clear();
    }

    /// Move the collected span blocks into `spans`, root span first.
    fn order_spans_root_first(&mut self) {
        let mut end = self.scratch.len();
        for &start in self.blocks.iter().rev() {
            if let Some(block) = self.scratch.get(start..end) {
                self.spans.extend_from_slice(block);
            }
            end = start;
        }
    }

    fn push_text(&mut self, f: impl FnOnce(&mut String) -> fmt::Result) -> Range<usize> {
        let start = self.text.len();
        // If a user `Debug` / `Display` impl fails, keep what it wrote.
        let _partial_text_is_kept = f(&mut self.text);
        start..self.text.len()
    }

    fn text(&self, range: &Range<usize>) -> &str {
        self.text.get(range.clone()).unwrap_or("")
    }

    fn key(&self, key: &Key) -> &str {
        match key {
            Key::Static(s) => s,
            Key::Text(r) => self.text(r),
        }
    }

    fn value(&self, slot: &Slot) -> Value<'_> {
        match slot {
            Slot::Scalar(v) => *v,
            Slot::Text(r) => Value::Str(self.text(r)),
        }
    }

    fn add_span_field(&mut self, field: &OwnedField) {
        let key = Key::Text(self.push_text(|t| t.write_str(&field.key)));
        let slot = match field.as_field().value {
            Value::Str(s) => Slot::Text(self.push_text(|t| t.write_str(s))),
            Value::Bytes(b) => Slot::Text(self.push_text(|t| write_hex(t, b))),
            Value::Null => Slot::Scalar(Value::Null),
            Value::Bool(b) => Slot::Scalar(Value::Bool(b)),
            Value::I64(n) => Slot::Scalar(Value::I64(n)),
            Value::U64(n) => Slot::Scalar(Value::U64(n)),
            Value::I128(n) => Slot::Scalar(Value::I128(n)),
            Value::U128(n) => Slot::Scalar(Value::U128(n)),
            Value::F64(n) => Slot::Scalar(Value::F64(n)),
            Value::Char(c) => Slot::Scalar(Value::Char(c)),
            other => Slot::Text(self.push_text(|t| write!(t, "{other}"))),
        };
        self.scratch.push((key, slot));
    }
}

/// Collects one event's fields into an [`Arena`].
struct EventVisitor<'a>(&'a mut Arena);

impl EventVisitor<'_> {
    fn scalar(&mut self, field: &TracingField, value: Value<'static>) {
        self.0
            .event
            .push((Key::Static(field.name()), Slot::Scalar(value)));
    }

    fn text(&mut self, field: &TracingField, f: impl FnOnce(&mut String) -> fmt::Result) {
        let range = self.0.push_text(f);
        if field.name() == "message" {
            self.0.message = Some(range);
        } else {
            self.0
                .event
                .push((Key::Static(field.name()), Slot::Text(range)));
        }
    }
}

impl Visit for EventVisitor<'_> {
    fn record_f64(&mut self, field: &TracingField, value: f64) {
        self.scalar(field, Value::F64(value));
    }
    fn record_i64(&mut self, field: &TracingField, value: i64) {
        self.scalar(field, Value::I64(value));
    }
    fn record_u64(&mut self, field: &TracingField, value: u64) {
        self.scalar(field, Value::U64(value));
    }
    fn record_i128(&mut self, field: &TracingField, value: i128) {
        self.scalar(field, Value::I128(value));
    }
    fn record_u128(&mut self, field: &TracingField, value: u128) {
        self.scalar(field, Value::U128(value));
    }
    fn record_bool(&mut self, field: &TracingField, value: bool) {
        self.scalar(field, Value::Bool(value));
    }
    fn record_str(&mut self, field: &TracingField, value: &str) {
        self.text(field, |t| t.write_str(value));
    }
    fn record_bytes(&mut self, field: &TracingField, value: &[u8]) {
        self.text(field, |t| write_hex(t, value));
    }
    fn record_error(&mut self, field: &TracingField, value: &(dyn std::error::Error + 'static)) {
        self.text(field, |t| write_error_chain(t, value));
    }
    fn record_debug(&mut self, field: &TracingField, value: &dyn fmt::Debug) {
        self.text(field, |t| write!(t, "{value:?}"));
    }
}

thread_local! {
    static ARENA: RefCell<Arena> = const { RefCell::new(Arena::new()) };
}

/// Arena capacity kept between events; larger buffers are released.
const MAX_RETAINED_TEXT: usize = 64 * 1024;

impl<S> Layer<S> for TracingLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut fields = SpanFields::default();
        attrs.record(&mut fields);
        span.extensions_mut().insert(fields);
    }

    fn on_record(&self, id: &Id, values: &SpanRecord<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        if let Some(fields) = extensions.get_mut::<SpanFields>() {
            values.record(fields);
        } else {
            let mut fields = SpanFields::default();
            values.record(&mut fields);
            extensions.insert(fields);
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = from_tracing_level(*meta.level());
        let target = meta.target();
        if !self.logger.enabled(target, level) {
            return;
        }
        let pooled = ARENA.try_with(|cell| {
            let mut arena = cell.try_borrow_mut().ok()?;
            self.emit(&mut arena, event, &ctx, level);
            if arena.text.capacity() > MAX_RETAINED_TEXT {
                arena.text = String::new();
            }
            Some(())
        });
        if !matches!(pooled, Ok(Some(()))) {
            // Reentrant event from inside a sink, or thread teardown.
            self.emit(&mut Arena::new(), event, &ctx, level);
        }
    }
}

impl TracingLayer {
    fn emit<S>(&self, arena: &mut Arena, event: &Event<'_>, ctx: &Context<'_, S>, level: Level)
    where
        S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        arena.clear();
        event.record(&mut EventVisitor(arena));
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope {
                let extensions = span.extensions();
                if let Some(fields) = extensions.get::<SpanFields>() {
                    arena.blocks.push(arena.scratch.len());
                    for field in &fields.0 {
                        arena.add_span_field(field);
                    }
                }
            }
            arena.order_spans_root_first();
        }

        let meta = event.metadata();
        let mut metadata = Metadata::new(level, meta.target());
        if let (Some(file), Some(line)) = (meta.file(), meta.line()) {
            metadata = metadata.with_location(file, line);
        }
        let arena = &*arena;
        let message = arena.message.as_ref().map_or("", |r| arena.text(r));
        with_fields(arena, &arena.event, |fields| {
            with_fields(arena, &arena.spans, |spans| {
                // Failures were already passed to the error handler.
                let _reported = self.logger.emit_unchecked(metadata, message, fields, spans);
            });
        });
    }
}

/// Materialize collected fields as `Field`s: on the stack for up to 16,
/// on the heap beyond that.
fn with_fields(arena: &Arena, items: &[(Key, Slot)], f: impl FnOnce(&[Field<'_>])) {
    const EMPTY: Field<'static> = Field {
        key: "",
        value: Value::Null,
    };
    if items.len() <= 16 {
        let mut buf = [EMPTY; 16];
        for (slot, (key, value)) in buf.iter_mut().zip(items) {
            *slot = Field {
                key: arena.key(key),
                value: arena.value(value),
            };
        }
        f(&buf[..items.len()]);
    } else {
        let fields: Vec<Field<'_>> = items
            .iter()
            .map(|(key, value)| Field {
                key: arena.key(key),
                value: arena.value(value),
            })
            .collect();
        f(&fields);
    }
}

#[cfg(all(test, feature = "logfmt"))]
mod tests {
    use super::*;
    use crate::format::LogfmtFormat;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    #[derive(Default, Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Capture {
        fn take(&self) -> String {
            String::from_utf8(std::mem::take(&mut *self.0.lock().unwrap())).unwrap()
        }
    }

    fn layer(level: Level) -> (TracingLayer, Capture) {
        let out = Capture::default();
        let logger = Logger::builder()
            .level(level)
            .no_timestamps()
            .no_context()
            .writer(out.clone(), LogfmtFormat::new())
            .build();
        (TracingLayer::new(logger), out)
    }

    #[test]
    fn test_tracing_layer_event_fields_and_message() {
        let (layer, out) = layer(Level::Trace);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            let err = std::io::Error::other("reset");
            tracing::warn!(
                target: "app::net",
                status = 503_u16,
                retry = true,
                ratio = 0.25,
                peer = %"10.0.0.1",
                tags = ?["a", "b"],
                big = u128::MAX,
                error = &err as &(dyn std::error::Error + 'static),
                "upstream {} failed", "db"
            );
        });
        let text = out.take();
        assert!(
            text.starts_with("level=warn target=app::net message=\"upstream db failed\" file="),
            "{text}"
        );
        assert!(
            text.contains(" status=503 retry=true ratio=0.25 peer=10.0.0.1 "),
            "{text}"
        );
        assert!(text.contains("tags=\"[\\\"a\\\", \\\"b\\\"]\""), "{text}");
        assert!(text.contains(&format!("big={}", u128::MAX)), "{text}");
        assert!(text.contains("error=reset"), "{text}");
    }

    #[test]
    fn test_tracing_layer_attaches_span_fields_with_shadowing() {
        let (layer, out) = layer(Level::Trace);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            let outer = tracing::info_span!(
                "request",
                id = 7_u64,
                user = "alice",
                tenant = tracing::field::Empty
            );
            let _o = outer.enter();
            let _span = outer.record("tenant", "acme");
            let inner = tracing::debug_span!("db", user = "svc", bytes = tracing::field::Empty);
            let _i = inner.enter();
            let _span = inner.record("bytes", &b"\x01\xff"[..]);
            tracing::info!(user = "event", "query");
            tracing::info!("no user field on the event");
        });
        let text = out.take();
        let mut lines = text.lines();
        let first = lines.next().unwrap_or_default();
        let second = lines.next().unwrap_or_default();
        assert!(
            second.contains(" user=svc"),
            "inner span shadows outer: {second}"
        );
        assert!(
            second.find(" id=7").unwrap() < second.find(" user=svc").unwrap(),
            "root span fields come first: {second}"
        );
        let text = first.to_owned();
        assert!(text.contains(" id=7 "), "{text}");
        assert!(text.contains(" tenant=acme "), "{text}");
        assert!(text.contains(" bytes=01ff"), "{text}");
        assert_eq!(text.matches("user=").count(), 1, "{text}");
        assert!(text.contains(" user=event"), "{text}");
    }

    #[test]
    fn test_tracing_layer_applies_logger_filter() {
        let (layer, out) = layer(Level::Warn);
        assert_eq!(layer.logger().min_level(), Level::Warn);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("dropped");
            tracing::error!("kept");
            tracing::trace!("dropped too");
            tracing::debug!("dropped as well");
        });
        let text = out.take();
        assert!(!text.contains("dropped"), "{text}");
        assert!(text.contains("message=kept"), "{text}");
    }

    #[test]
    fn test_tracing_layer_many_fields_spill_to_heap() {
        let (layer, out) = layer(Level::Trace);
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                a0 = 0,
                a1 = 1,
                a2 = 2,
                a3 = 3,
                a4 = 4,
                a5 = 5,
                a6 = 6,
                a7 = 7,
                a8 = 8,
                a9 = 9,
                a10 = 10,
                a11 = 11,
                a12 = 12,
                a13 = 13,
                a14 = 14,
                a15 = 15,
                a16 = 16,
                a17 = 17,
                "wide"
            );
        });
        let text = out.take();
        for i in 0..18 {
            assert!(text.contains(&format!(" a{i}={i}")), "a{i}: {text}");
        }
    }

    #[test]
    fn test_tracing_level_mapping() {
        assert_eq!(from_tracing_level(tracing_core::Level::TRACE), Level::Trace);
        assert_eq!(from_tracing_level(tracing_core::Level::DEBUG), Level::Debug);
        assert_eq!(from_tracing_level(tracing_core::Level::INFO), Level::Info);
        assert_eq!(from_tracing_level(tracing_core::Level::WARN), Level::Warn);
        assert_eq!(from_tracing_level(tracing_core::Level::ERROR), Level::Error);
    }
}

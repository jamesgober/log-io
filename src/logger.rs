//! The [`Logger`]: filter, assemble, dispatch.
//!
//! A `Logger` bundles a [`Filter`], one or more [`Sink`]s, optional
//! timestamping, thread-local context capture, default fields,
//! redaction, and the flush policy. Cloning a `Logger` clones an `Arc`;
//! all clones share the same sinks.

use core::cell::{Cell, RefCell};
use core::fmt;
use std::sync::Arc;

use crate::context;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::level::Level;
use crate::record::{Field, Metadata, Record};
use crate::sink::{Sink, trim_buffer};
use crate::time;
use crate::value::{OwnedField, Value};

use crate::builder::LoggerBuilder;

/// Callback invoked for every sink error, flush error, and message
/// formatting error the logger encounters.
///
/// The handler runs on the thread that logged the record. If the
/// handler itself logs through a logger whose sink keeps failing, the
/// nested failures are not reported again (a per-thread guard prevents
/// unbounded recursion), but the nested records are still dispatched.
///
/// # Examples
///
#[cfg_attr(all(feature = "logfmt"), doc = "```")]
#[cfg_attr(not(all(feature = "logfmt")), doc = "```ignore")]
/// use std::sync::Arc;
/// use log_io::{ErrorHandler, Logger};
///
/// let handler: ErrorHandler = Arc::new(|err| eprintln!("logging failed: {err}"));
/// let logger = Logger::builder().stderr_logfmt().on_error(handler).build();
/// # let _ = logger;
/// ```
pub type ErrorHandler = Arc<dyn Fn(&Error) + Send + Sync>;

/// Value written in place of a redacted field.
pub const REDACTED: &str = "[REDACTED]";

/// The configured logging pipeline.
///
/// Build one with [`Logger::builder`]. A `Logger` is cheap to clone and
/// `Send + Sync`; share it by cloning or install it process-wide with
/// [`crate::init`].
///
/// # Record assembly
///
/// For each admitted record the logger:
///
/// 1. stamps the wall-clock time (unless disabled);
/// 2. collects ambient fields: default fields, then the thread's
///    context ([`crate::context`]), then any extra context supplied by
///    an integration (for example `tracing` span fields);
/// 3. removes duplicate keys so every key appears once. Precedence is
///    call-site fields > extra context > thread context > defaults;
/// 4. replaces the values of redacted keys with [`REDACTED`];
/// 5. hands the record to every sink in installation order. A failing
///    sink does not stop the others; each failure goes to the error
///    handler;
/// 6. flushes every sink if the record is at or above the flush level
///    (default [`Level::Error`]).
///
/// Steps 2-4 use stack buffers for up to 16 fields per group and do not
/// allocate in that case.
///
/// # Shutdown
///
/// When the last clone is dropped, every sink is shut down (flushed,
/// background threads drained and joined) and failures are reported
/// to the error handler. A logger stored in a `static` is never
/// dropped: keep the [`crate::FlushGuard`] from [`crate::init`] or
/// [`Logger::flush_guard`] alive, or call [`Logger::shutdown`].
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// use log_io::{Field, Level, Logger};
///
/// let logger = Logger::builder()
///     .level(Level::Info)
///     .with_default_field("service", "billing")
///     .stdout_json()
///     .build();
///
/// logger.log(Level::Info, "ready", &[Field::new("port", 8443_u16)]);
/// log_io::warn!(logger, "slow query took {} ms", 1280, table = "invoices");
/// ```
#[derive(Clone)]
pub struct Logger {
    inner: Arc<LoggerInner>,
}

// `Logger` keeps working after a panic anywhere in the pipeline: sink
// locks recover from poisoning and no logger state is left half-updated
// by an unwind, so observing it after `catch_unwind` is sound.
impl std::panic::UnwindSafe for Logger {}
impl std::panic::RefUnwindSafe for Logger {}

pub(crate) struct LoggerInner {
    pub(crate) filter: Filter,
    pub(crate) sinks: Box<[Box<dyn Sink>]>,
    pub(crate) defaults: Box<[OwnedField]>,
    pub(crate) default_target: Box<str>,
    pub(crate) capture_timestamps: bool,
    pub(crate) capture_context: bool,
    pub(crate) flush_level: Level,
    pub(crate) on_error: Option<ErrorHandler>,
    /// Lower-cased keys whose values are replaced by [`REDACTED`].
    pub(crate) redact: Box<[Box<str>]>,
}

impl Drop for LoggerInner {
    fn drop(&mut self) {
        for sink in &self.sinks {
            if let Err(e) = sink.shutdown() {
                report_with(self.on_error.as_ref(), &e);
            }
        }
    }
}

thread_local! {
    /// Depth of `dispatch` calls on this thread. Read by the panic
    /// hook so it never re-enters a sink that the panicking frame may
    /// still hold locked.
    static DISPATCH_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// Set while an error handler runs on this thread.
    static IN_HANDLER: Cell<bool> = const { Cell::new(false) };
    /// Reusable buffer for rendering `format_args!` messages.
    static MESSAGE_BUF: RefCell<String> = const { RefCell::new(String::new()) };
}

/// `true` while this thread is inside a logger dispatch (a sink or
/// format is running).
pub(crate) fn in_dispatch() -> bool {
    DISPATCH_DEPTH.try_with(Cell::get).unwrap_or(0) > 0
}

struct DepthGuard;

impl DepthGuard {
    fn enter() -> Self {
        let _tls_may_be_gone = DISPATCH_DEPTH.try_with(|d| d.set(d.get().saturating_add(1)));
        Self
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        let _tls_may_be_gone = DISPATCH_DEPTH.try_with(|d| d.set(d.get().saturating_sub(1)));
    }
}

fn report_with(handler: Option<&ErrorHandler>, error: &Error) {
    let Some(handler) = handler else {
        return;
    };
    let _tls_may_be_gone = IN_HANDLER.try_with(|busy| {
        if busy.get() {
            return;
        }
        busy.set(true);
        struct Reset<'a>(&'a Cell<bool>);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _reset = Reset(busy);
        handler(error);
    });
}

impl Logger {
    /// Start configuring a logger. See [`LoggerBuilder`] for defaults.
    pub fn builder() -> LoggerBuilder {
        LoggerBuilder::new()
    }

    pub(crate) fn from_inner(inner: LoggerInner) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    /// The filter threshold that applies to `target`.
    #[must_use]
    pub fn threshold_for(&self, target: &str) -> Level {
        self.inner.filter.threshold_for(target)
    }

    /// Returns `true` if a record with this target and level would pass
    /// the filter. The macros call this before evaluating any field or
    /// formatting the message.
    #[inline]
    #[must_use]
    pub fn enabled(&self, target: &str, level: Level) -> bool {
        self.inner.filter.is_enabled(target, level)
    }

    /// The lowest severity the filter admits for any target. A record
    /// below it is rejected without looking at its target.
    #[must_use]
    pub fn min_level(&self) -> Level {
        self.inner.filter.min_level()
    }

    /// The filter in use.
    #[must_use]
    pub fn filter(&self) -> &Filter {
        &self.inner.filter
    }

    /// Number of installed sinks.
    #[must_use]
    pub fn sink_count(&self) -> usize {
        self.inner.sinks.len()
    }

    /// The target used by [`Logger::log`] (empty unless set with
    /// [`LoggerBuilder::default_target`]).
    #[must_use]
    pub fn default_target(&self) -> &str {
        &self.inner.default_target
    }

    /// Emit a record with the logger's default target.
    ///
    /// Errors are passed to the error handler, not returned. Note that
    /// per-target filter rules only apply to this record if they match
    /// the default target; the macros use the calling module's path
    /// instead.
    #[inline]
    pub fn log(&self, level: Level, message: &str, fields: &[Field<'_>]) {
        let _reported_to_handler = self.try_log(level, &self.inner.default_target, message, fields);
    }

    /// Emit a record with an explicit target.
    ///
    /// # Errors
    ///
    /// Returns the first sink or flush error. Every sink still receives
    /// the record, and every error is also passed to the error handler.
    #[inline]
    pub fn try_log(
        &self,
        level: Level,
        target: &str,
        message: &str,
        fields: &[Field<'_>],
    ) -> Result<()> {
        self.try_emit(Metadata::new(level, target), message, fields)
    }

    /// Lowest-level entry point: emit a record with caller-supplied
    /// metadata (for example a source location). The filter uses
    /// `metadata.target` and `metadata.level`; a timestamp is added if
    /// timestamps are enabled and `metadata` has none.
    ///
    /// # Errors
    ///
    /// Same as [`Logger::try_log`].
    #[inline]
    pub fn try_emit(
        &self,
        metadata: Metadata<'_>,
        message: &str,
        fields: &[Field<'_>],
    ) -> Result<()> {
        if !self.enabled(metadata.target, metadata.level) {
            return Ok(());
        }
        self.emit_unchecked(metadata, message, fields, &[])
    }

    /// Flush every sink. Records at or above the flush level are already
    /// flushed as they are written; call this before a planned exit.
    ///
    /// # Errors
    ///
    /// Returns the first error. Every sink is still flushed, and every
    /// error is also passed to the error handler.
    pub fn flush(&self) -> Result<()> {
        self.for_each_sink(|sink| sink.flush())
    }

    /// Flush every sink and stop background workers, draining their
    /// queues first. The logger stays usable afterwards: sinks with a
    /// background thread write later records synchronously.
    ///
    /// # Errors
    ///
    /// Same as [`Logger::flush`].
    pub fn shutdown(&self) -> Result<()> {
        self.for_each_sink(|sink| sink.shutdown())
    }

    /// A guard that calls [`Logger::shutdown`] when dropped. Keep it
    /// alive in `main` when the logger outlives `main` (for example in
    /// a `static`).
    pub fn flush_guard(&self) -> crate::FlushGuard {
        crate::FlushGuard::new(self.clone())
    }

    fn for_each_sink(&self, op: impl Fn(&dyn Sink) -> Result<()>) -> Result<()> {
        let _depth = DepthGuard::enter();
        let mut first = None;
        for sink in &self.inner.sinks {
            if let Err(e) = op(&**sink) {
                self.report(&e);
                if first.is_none() {
                    first = Some(e);
                }
            }
        }
        first.map_or(Ok(()), Err)
    }

    pub(crate) fn report(&self, error: &Error) {
        report_with(self.inner.on_error.as_ref(), error);
    }

    /// Emit a message given as format arguments, without re-checking
    /// the filter. Renders into a reused thread-local buffer, so it
    /// does not allocate in steady state; a literal with no arguments
    /// is used as-is.
    pub(crate) fn emit_fmt(
        &self,
        metadata: Metadata<'_>,
        args: fmt::Arguments<'_>,
        fields: &[Field<'_>],
        extra: &[Field<'_>],
    ) {
        if let Some(message) = args.as_str() {
            let _reported = self.emit_unchecked(metadata, message, fields, extra);
            return;
        }
        let pooled = MESSAGE_BUF.try_with(|cell| {
            let mut buf = cell.try_borrow_mut().ok()?;
            self.emit_rendered(&mut buf, metadata, args, fields, extra);
            trim_buffer(&mut buf);
            Some(())
        });
        if !matches!(pooled, Ok(Some(()))) {
            // Reentrant call from inside a sink, or thread teardown.
            self.emit_rendered(&mut String::new(), metadata, args, fields, extra);
        }
    }

    fn emit_rendered(
        &self,
        buf: &mut String,
        metadata: Metadata<'_>,
        args: fmt::Arguments<'_>,
        fields: &[Field<'_>],
        extra: &[Field<'_>],
    ) {
        buf.clear();
        let rendered = fmt::write(buf, args);
        let _reported = self.emit_unchecked(metadata, buf, fields, extra);
        if rendered.is_err() {
            // A `Display` impl in the arguments failed; the record was
            // emitted with whatever text was produced before the error.
            self.report(&Error::Format(fmt::Error));
        }
    }

    /// Assemble and dispatch without consulting the filter. `extra` is
    /// ambient context supplied by an integration; it shadows thread
    /// context and defaults.
    pub(crate) fn emit_unchecked(
        &self,
        mut metadata: Metadata<'_>,
        message: &str,
        fields: &[Field<'_>],
        extra: &[Field<'_>],
    ) -> Result<()> {
        if self.inner.capture_timestamps && metadata.timestamp_unix_nanos.is_none() {
            metadata.timestamp_unix_nanos = Some(time::now_unix_nanos());
        }
        if self.inner.capture_context {
            context::with_snapshot(|ctx| self.assemble(metadata, message, fields, ctx, extra))
        } else {
            self.assemble(metadata, message, fields, &[], extra)
        }
    }

    fn assemble(
        &self,
        metadata: Metadata<'_>,
        message: &str,
        fields: &[Field<'_>],
        ctx: &[Field<'_>],
        extra: &[Field<'_>],
    ) -> Result<()> {
        let redacting = !self.inner.redact.is_empty();
        if self.inner.defaults.is_empty()
            && extra.is_empty()
            && !redacting
            && (ctx.is_empty() || !any_key_in(ctx, fields))
        {
            return self.dispatch(&Record::with_context(metadata, message, fields, ctx));
        }

        // Common case: default fields only (no thread or extra context,
        // no redaction). Defaults have unique keys by construction, so
        // only collisions with call fields need checking.
        let defaults = &self.inner.defaults;
        if ctx.is_empty() && extra.is_empty() && !redacting && defaults.len() <= INLINE_FIELDS {
            let mut buf = [EMPTY_FIELD; INLINE_FIELDS];
            let mut len = 0;
            for owned in defaults {
                if !fields.iter().any(|f| f.key == &*owned.key) {
                    buf[len] = owned.as_field();
                    len += 1;
                }
            }
            return self.dispatch(&Record::with_context(
                metadata,
                message,
                fields,
                &buf[..len],
            ));
        }

        if redacting && fields.iter().any(|f| self.is_redacted(f.key)) {
            let mut call = FieldBuf::new();
            for f in fields {
                call.push(self.redacted(*f));
            }
            self.merge_ambient(metadata, message, call.as_slice(), ctx, extra)
        } else {
            self.merge_ambient(metadata, message, fields, ctx, extra)
        }
    }

    /// Merge defaults, thread context and extra context into one
    /// de-duplicated, redacted slice, then dispatch.
    fn merge_ambient(
        &self,
        metadata: Metadata<'_>,
        message: &str,
        fields: &[Field<'_>],
        ctx: &[Field<'_>],
        extra: &[Field<'_>],
    ) -> Result<()> {
        // Highest precedence first; each layer newest first. Reversed
        // at the end so the output keeps defaults, context, extra order.
        let mut ambient = FieldBuf::new();
        for layer in [extra, ctx] {
            for f in layer.iter().rev() {
                self.push_ambient(&mut ambient, *f, fields);
            }
        }
        for owned in self.inner.defaults.iter().rev() {
            self.push_ambient(&mut ambient, owned.as_field(), fields);
        }
        ambient.reverse();
        self.dispatch(&Record::with_context(
            metadata,
            message,
            fields,
            ambient.as_slice(),
        ))
    }

    #[inline]
    fn push_ambient<'a>(&self, ambient: &mut FieldBuf<'a>, field: Field<'a>, call: &[Field<'_>]) {
        let shadowed = call.iter().any(|c| c.key == field.key)
            || ambient.as_slice().iter().any(|a| a.key == field.key);
        if !shadowed {
            ambient.push(self.redacted(field));
        }
    }

    #[inline]
    fn redacted<'a>(&self, field: Field<'a>) -> Field<'a> {
        if self.is_redacted(field.key) {
            Field {
                key: field.key,
                value: Value::Str(REDACTED),
            }
        } else {
            field
        }
    }

    /// A key is redacted when it, or its last dot-separated segment,
    /// equals a configured key (ASCII case-insensitive).
    fn is_redacted(&self, key: &str) -> bool {
        if self.inner.redact.is_empty() {
            return false;
        }
        let last = key.rsplit('.').next().unwrap_or(key);
        self.inner
            .redact
            .iter()
            .any(|r| r.eq_ignore_ascii_case(key) || r.eq_ignore_ascii_case(last))
    }

    fn dispatch(&self, record: &Record<'_>) -> Result<()> {
        let _depth = DepthGuard::enter();
        let mut first = None;
        for sink in &self.inner.sinks {
            if let Err(e) = sink.write_record(record) {
                self.report(&e);
                if first.is_none() {
                    first = Some(e);
                }
            }
        }
        if record.metadata.level >= self.inner.flush_level {
            for sink in &self.inner.sinks {
                if let Err(e) = sink.request_flush() {
                    self.report(&e);
                    if first.is_none() {
                        first = Some(e);
                    }
                }
            }
        }
        first.map_or(Ok(()), Err)
    }
}

fn any_key_in(a: &[Field<'_>], b: &[Field<'_>]) -> bool {
    !b.is_empty() && a.iter().any(|x| b.iter().any(|y| x.key == y.key))
}

impl fmt::Debug for Logger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Logger")
            .field("filter", &self.inner.filter)
            .field("sinks", &self.inner.sinks.len())
            .field("default_fields", &self.inner.defaults.len())
            .field("default_target", &self.inner.default_target)
            .field("capture_timestamps", &self.inner.capture_timestamps)
            .field("capture_context", &self.inner.capture_context)
            .field("flush_level", &self.inner.flush_level)
            .field("redacted_keys", &self.inner.redact.len())
            .finish_non_exhaustive()
    }
}

/// Fields kept on the stack before spilling to the heap.
const INLINE_FIELDS: usize = 16;

const EMPTY_FIELD: Field<'static> = Field {
    key: "",
    value: Value::Null,
};

/// Up to [`INLINE_FIELDS`] fields on the stack, spilling to the heap
/// beyond that.
struct FieldBuf<'a> {
    stack: [Field<'a>; INLINE_FIELDS],
    len: usize,
    heap: Vec<Field<'a>>,
}

impl<'a> FieldBuf<'a> {
    fn new() -> Self {
        Self {
            stack: [EMPTY_FIELD; INLINE_FIELDS],
            len: 0,
            heap: Vec::new(),
        }
    }

    #[inline]
    fn push(&mut self, field: Field<'a>) {
        if !self.heap.is_empty() {
            self.heap.push(field);
        } else if let Some(slot) = self.stack.get_mut(self.len) {
            *slot = field;
            self.len += 1;
        } else {
            self.heap.reserve(self.len * 2);
            self.heap.extend_from_slice(&self.stack[..self.len]);
            self.heap.push(field);
        }
    }

    #[inline]
    fn as_slice(&self) -> &[Field<'a>] {
        if self.heap.is_empty() {
            &self.stack[..self.len]
        } else {
            &self.heap
        }
    }

    fn reverse(&mut self) {
        if self.heap.is_empty() {
            self.stack[..self.len].reverse();
        } else {
            self.heap.reverse();
        }
    }
}

#[cfg(all(test, feature = "logfmt", feature = "json"))]
mod tests {
    use super::*;
    use crate::format::{JsonFormat, LogfmtFormat};
    use std::io::Write;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default, Clone)]
    struct MemBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for MemBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl MemBuf {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn quiet() -> LoggerBuilder {
        Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .no_context()
    }

    struct Failing(AtomicUsize);
    impl Sink for Failing {
        fn write_record(&self, _record: &Record<'_>) -> Result<()> {
            let _n = self.0.fetch_add(1, Ordering::Relaxed);
            Err(Error::Configuration("boom"))
        }
        fn flush(&self) -> Result<()> {
            Err(Error::Configuration("flush boom"))
        }
    }

    #[test]
    fn test_logger_enabled_and_min_level() {
        let logger = Logger::builder()
            .level(Level::Warn)
            .target_level("loud", Level::Trace)
            .with_sink(crate::sink::NullSink::new())
            .build();
        assert!(logger.enabled("loud", Level::Trace));
        assert!(!logger.enabled("other", Level::Info));
        assert_eq!(logger.min_level(), Level::Trace);
        assert_eq!(logger.threshold_for("loud::x"), Level::Trace);
        assert_eq!(logger.filter().default_level(), Level::Warn);
        assert_eq!(logger.sink_count(), 1);
    }

    #[test]
    fn test_logger_log_writes_through_logfmt() {
        let buf = MemBuf::default();
        let logger = quiet().writer(buf.clone(), LogfmtFormat::new()).build();
        logger.log(Level::Info, "hi", &[Field::new("k", 1_u64)]);
        assert_eq!(buf.text(), "level=info target=\"\" message=hi k=1\n");
    }

    #[test]
    fn test_logger_default_target_is_used_by_log() {
        // The audit's R11: `log` with a per-target filter.
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .filter_directive("myapp=debug")
            .default_target("myapp")
            .no_timestamps()
            .writer(buf.clone(), JsonFormat::new())
            .build();
        assert_eq!(logger.default_target(), "myapp");
        logger.log(Level::Error, "kept", &[]);
        assert!(
            buf.text().contains("\"target\":\"myapp\""),
            "{}",
            buf.text()
        );
    }

    #[test]
    fn test_logger_context_is_spliced_in() {
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .writer(buf.clone(), LogfmtFormat::new())
            .build();
        context::clear();
        let guard = context::with_trace_id("tx-42");
        logger.log(Level::Info, "hello", &[]);
        drop(guard);
        assert!(buf.text().contains("trace_id=tx-42"), "{}", buf.text());
    }

    #[test]
    fn test_logger_duplicate_keys_follow_precedence() {
        // The audit's R06: defaults < context < call fields, one key each.
        context::clear();
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .with_default_field("user", "default")
            .with_default_field("service", "api")
            .with_default_field("region", "eu")
            .writer(buf.clone(), JsonFormat::new())
            .build();
        let _a = context::with_field("user", "outer");
        let _b = context::with_field("user", "inner");
        let _c = context::with_field("region", "us");
        logger.log(Level::Error, "m", &[Field::new("region", "call")]);
        let out = buf.text();
        assert_eq!(
            out,
            "{\"level\":\"error\",\"target\":\"\",\"message\":\"m\",\"service\":\"api\",\
             \"user\":\"inner\",\"region\":\"call\"}\n"
        );
        context::clear();
    }

    #[test]
    fn test_logger_context_vs_call_dedup_without_defaults() {
        context::clear();
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .writer(buf.clone(), LogfmtFormat::new())
            .build();
        let _t = context::with_field("tenant", "ctx");
        logger.log(Level::Info, "m", &[Field::new("tenant", "call")]);
        assert_eq!(buf.text(), "level=info target=\"\" message=m tenant=call\n");
        context::clear();
    }

    #[test]
    fn test_logger_extra_context_shadows_thread_context() {
        context::clear();
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .writer(buf.clone(), LogfmtFormat::new())
            .build();
        let _t = context::with_field("span", "thread");
        let extra = [Field::new("span", "extra"), Field::new("depth", 2_u8)];
        logger.emit_fmt(
            Metadata::new(Level::Info, "t"),
            format_args!("m"),
            &[],
            &extra,
        );
        assert_eq!(
            buf.text(),
            "level=info target=t message=m span=extra depth=2\n"
        );
        context::clear();
    }

    #[test]
    fn test_logger_redacts_keys_everywhere() {
        context::clear();
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .redact_common_secrets()
            .redact_key("Session_ID")
            .with_default_field("api_key", "k-123")
            .writer(buf.clone(), LogfmtFormat::new())
            .build();
        let _c = context::with_field("authorization", "Bearer x");
        logger.log(
            Level::Info,
            "login",
            &[
                Field::new("user", "alice"),
                Field::new("db.PASSWORD", "hunter2"),
                Field::new("session_id", "s1"),
            ],
        );
        let out = buf.text();
        for secret in ["k-123", "Bearer", "hunter2", "s1"] {
            assert!(!out.contains(secret), "{secret} leaked: {out}");
        }
        assert!(out.contains("user=alice"), "{out}");
        assert_eq!(out.matches(REDACTED).count(), 4, "{out}");
        context::clear();
    }

    #[test]
    fn test_logger_redaction_never_evaluates_lazy_values() {
        struct Explodes;
        impl fmt::Display for Explodes {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                Err(fmt::Error)
            }
        }
        let buf = MemBuf::default();
        let logger = quiet()
            .redact_key("token")
            .writer(buf.clone(), JsonFormat::new())
            .build();
        let result = logger.try_log(
            Level::Info,
            "t",
            "m",
            &[Field::new("token", Value::Display(&Explodes))],
        );
        assert!(result.is_ok());
        assert!(buf.text().contains("\"token\":\"[REDACTED]\""));
    }

    #[test]
    fn test_logger_more_than_sixteen_ambient_fields_spill_correctly() {
        context::clear();
        let buf = MemBuf::default();
        let mut builder = Logger::builder().level(Level::Trace).no_timestamps();
        let keys: Vec<String> = (0..20).map(|i| format!("d{i}")).collect();
        for (i, k) in keys.iter().enumerate() {
            builder = builder.with_default_field(k.clone(), i as u32);
        }
        let logger = builder.writer(buf.clone(), JsonFormat::new()).build();
        let _c = context::with_field("c", 1_u8);
        logger.log(Level::Info, "ok", &[Field::new("d0", "call")]);
        let out = buf.text();
        assert!(out.contains("\"d0\":\"call\""), "{out}");
        assert_eq!(out.matches("\"d0\"").count(), 1, "{out}");
        for (i, k) in keys.iter().enumerate().skip(1) {
            assert!(out.contains(&format!("\"{k}\":{i}")), "missing {k}: {out}");
        }
        assert!(out.contains("\"c\":1"));
        context::clear();
    }

    #[test]
    fn test_logger_dispatch_continues_after_sink_error_and_reports_each() {
        // The audit's R09: the handler sees every failing sink.
        let failing = Arc::new(Failing(AtomicUsize::new(0)));
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = Arc::clone(&calls);
        let buf = MemBuf::default();
        let logger = quiet()
            .flush_on(Level::Off)
            .with_sink(Arc::clone(&failing))
            .with_sink(Failing(AtomicUsize::new(0)))
            .writer(buf.clone(), JsonFormat::new())
            .on_error(Arc::new(move |_| {
                let _n = calls2.fetch_add(1, Ordering::Relaxed);
            }))
            .build();
        let result = logger.try_log(Level::Info, "t", "m", &[]);
        assert!(matches!(result, Err(Error::Configuration("boom"))));
        assert!(buf.text().contains("\"message\":\"m\""));
        assert_eq!(failing.0.load(Ordering::Relaxed), 1);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            2,
            "one call per failing sink"
        );
    }

    #[test]
    fn test_logger_flush_on_level_flushes_and_reports() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = Arc::clone(&calls);
        let logger = quiet()
            .flush_on(Level::Warn)
            .with_sink(Failing(AtomicUsize::new(0)))
            .on_error(Arc::new(move |_| {
                let _n = calls2.fetch_add(1, Ordering::Relaxed);
            }))
            .build();
        logger.log(Level::Info, "below", &[]);
        assert_eq!(calls.load(Ordering::Relaxed), 1, "write error only");
        logger.log(Level::Warn, "at", &[]);
        assert_eq!(calls.load(Ordering::Relaxed), 3, "write and flush errors");
        assert!(logger.flush().is_err());
        assert!(logger.shutdown().is_err());
    }

    #[test]
    fn test_logger_on_error_recursion_is_bounded() {
        // The audit's LOG-015: a handler that logs through the same
        // failing logger used to recurse until the stack overflowed.
        use std::sync::OnceLock;
        static SELF: OnceLock<Logger> = OnceLock::new();
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let logger = quiet()
            .with_sink(Failing(AtomicUsize::new(0)))
            .on_error(Arc::new(|_| {
                let _n = CALLS.fetch_add(1, Ordering::Relaxed);
                if let Some(l) = SELF.get() {
                    l.log(Level::Error, "sink failed", &[]);
                }
            }))
            .build();
        assert!(SELF.set(logger).is_ok());
        if let Some(l) = SELF.get() {
            l.log(Level::Error, "first", &[]);
        }
        // write error of the outer record (1) + flush error (1); the
        // nested record's failures are not re-reported.
        assert_eq!(CALLS.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_logger_drop_shuts_sinks_down() {
        struct CountShutdown(Arc<AtomicUsize>);
        impl Sink for CountShutdown {
            fn write_record(&self, _: &Record<'_>) -> Result<()> {
                Ok(())
            }
            fn flush(&self) -> Result<()> {
                Ok(())
            }
            fn shutdown(&self) -> Result<()> {
                let _n = self.0.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        let logger = quiet().with_sink(CountShutdown(Arc::clone(&count))).build();
        let clone = logger.clone();
        drop(logger);
        assert_eq!(count.load(Ordering::Relaxed), 0, "a clone is still alive");
        drop(clone);
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_logger_emit_fmt_renders_arguments_and_reports_format_errors() {
        struct Broken;
        impl fmt::Display for Broken {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("partial")?;
                Err(fmt::Error)
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = Arc::clone(&calls);
        let buf = MemBuf::default();
        let logger = quiet()
            .writer(buf.clone(), LogfmtFormat::new())
            .on_error(Arc::new(move |e| {
                assert!(matches!(e, Error::Format(_)));
                let _n = calls2.fetch_add(1, Ordering::Relaxed);
            }))
            .build();
        let n = 7;
        logger.emit_fmt(
            Metadata::new(Level::Info, "t"),
            format_args!("n={n}"),
            &[],
            &[],
        );
        logger.emit_fmt(
            Metadata::new(Level::Info, "t"),
            format_args!("{}", Broken),
            &[],
            &[],
        );
        let out = buf.text();
        assert!(out.contains("message=\"n=7\""), "{out}");
        assert!(out.contains("message=partial"), "{out}");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_logger_timestamps_are_added_when_enabled() {
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .no_context()
            .writer(buf.clone(), LogfmtFormat::new())
            .build();
        logger.log(Level::Info, "t", &[]);
        assert!(buf.text().starts_with("timestamp="), "{}", buf.text());
        let explicit = Metadata::new(Level::Info, "x").with_timestamp(5);
        logger.try_emit(explicit, "m", &[]).unwrap();
        assert!(buf.text().contains("timestamp=5 "), "{}", buf.text());
    }

    #[test]
    fn test_logger_debug_lists_configuration() {
        let logger = quiet().with_sink(crate::sink::NullSink::new()).build();
        let text = format!("{logger:?}");
        assert!(text.contains("flush_level: Error"), "{text}");
        assert!(text.contains("sinks: 1"), "{text}");
    }

    #[test]
    fn test_logger_is_unwind_safe_and_thread_safe() {
        fn assert_traits<T: Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe>() {}
        assert_traits::<Logger>();
    }

    #[test]
    fn test_field_buf_spills_and_reverses() {
        let mut buf = FieldBuf::new();
        for i in 0..20_u64 {
            buf.push(Field::new("k", i));
        }
        assert_eq!(buf.as_slice().len(), 20);
        buf.reverse();
        assert_eq!(buf.as_slice()[0].value, Value::U64(19));
        let mut small = FieldBuf::new();
        small.push(Field::new("a", 1_u8));
        small.push(Field::new("b", 2_u8));
        small.reverse();
        assert_eq!(small.as_slice()[0].key, "b");
    }
}

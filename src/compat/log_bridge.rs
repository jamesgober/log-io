//! `log::Log` implementation.

use core::fmt;

use log::kv::{self, Key, VisitSource, VisitValue};

use crate::level::Level;
use crate::logger::Logger;
use crate::record::{Field, Metadata};
use crate::value::Value;

/// Routes records from the [`log`] facade into a [`Logger`].
///
/// * Levels map one to one (`log::Level::Warn` becomes
///   [`Level::Warn`], and so on).
/// * The record's target, file and line are preserved.
/// * The message's format arguments are rendered into a reused
///   thread-local buffer (no allocation in steady state).
/// * Structured key-values (`log::info!(user = "alice"; "login")`)
///   become fields. Strings, integers, floats, booleans and chars keep
///   their type; anything else is rendered through its `Display`
///   implementation when a sink serializes the record. Up to 16
///   key-values are collected without allocating.
///
/// # Examples
///
/// ```
/// use log_io::compat::LogBridge;
/// use log_io::{Level, Logger};
///
/// let logger = Logger::builder().level(Level::Info).stdout_logfmt().build();
/// LogBridge::new(logger).install()?;
///
/// log::info!(user = "alice", attempts = 3; "login succeeded");
/// # Ok::<(), log::SetLoggerError>(())
/// ```
#[derive(Debug, Clone)]
pub struct LogBridge {
    logger: Logger,
}

impl LogBridge {
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

    /// Install as the `log` crate's global logger and set
    /// `log::max_level` from the logger's [`Logger::min_level`], so
    /// disabled `log` macros cost a single comparison.
    ///
    /// # Errors
    ///
    /// Returns [`log::SetLoggerError`] if a `log` logger is already
    /// installed.
    pub fn install(self) -> Result<(), log::SetLoggerError> {
        let max = to_level_filter(self.logger.min_level());
        log::set_boxed_logger(Box::new(self))?;
        log::set_max_level(max);
        Ok(())
    }
}

/// Convert a log-io level threshold to a [`log::LevelFilter`].
///
/// # Examples
///
/// ```
/// use log_io::compat::to_level_filter;
/// use log_io::Level;
///
/// assert_eq!(to_level_filter(Level::Warn), log::LevelFilter::Warn);
/// assert_eq!(to_level_filter(Level::Off), log::LevelFilter::Off);
/// ```
#[must_use]
pub const fn to_level_filter(level: Level) -> log::LevelFilter {
    match level {
        Level::Trace => log::LevelFilter::Trace,
        Level::Debug => log::LevelFilter::Debug,
        Level::Info => log::LevelFilter::Info,
        Level::Warn => log::LevelFilter::Warn,
        Level::Error => log::LevelFilter::Error,
        Level::Off => log::LevelFilter::Off,
    }
}

const fn from_log_level(level: log::Level) -> Level {
    match level {
        log::Level::Error => Level::Error,
        log::Level::Warn => Level::Warn,
        log::Level::Info => Level::Info,
        log::Level::Debug => Level::Debug,
        log::Level::Trace => Level::Trace,
    }
}

impl log::Log for LogBridge {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.logger
            .enabled(metadata.target(), from_log_level(metadata.level()))
    }

    fn log(&self, record: &log::Record<'_>) {
        let level = from_log_level(record.level());
        let target = record.target();
        if !self.logger.enabled(target, level) {
            return;
        }
        let mut metadata = Metadata::new(level, target);
        if let (Some(file), Some(line)) = (record.file(), record.line()) {
            metadata = metadata.with_location(file, line);
        }

        let mut pairs = Pairs::default();
        // `visit` only fails if a visitor does; ours never does.
        let _infallible = record.key_values().visit(&mut pairs);
        if pairs.heap.is_empty() {
            let mut fields = [EMPTY; INLINE];
            for (slot, (key, value)) in fields.iter_mut().zip(pairs.inline.iter().flatten()) {
                *slot = Field {
                    key: key.as_str(),
                    value: convert(value),
                };
            }
            self.logger
                .emit_fmt(metadata, *record.args(), &fields[..pairs.len], &[]);
        } else {
            let fields: Vec<Field<'_>> = pairs
                .heap
                .iter()
                .map(|(key, value)| Field {
                    key: key.as_str(),
                    value: convert(value),
                })
                .collect();
            self.logger.emit_fmt(metadata, *record.args(), &fields, &[]);
        }
    }

    fn flush(&self) {
        // Failures were already passed to the logger's error handler.
        let _reported = self.logger.flush();
    }
}

const INLINE: usize = 16;

const EMPTY: Field<'static> = Field {
    key: "",
    value: Value::Null,
};

/// Key-value pairs of one record: inline up to [`INLINE`], then all on
/// the heap.
struct Pairs<'kvs> {
    inline: [Option<(Key<'kvs>, kv::Value<'kvs>)>; INLINE],
    len: usize,
    heap: Vec<(Key<'kvs>, kv::Value<'kvs>)>,
}

impl Default for Pairs<'_> {
    fn default() -> Self {
        Self {
            inline: [const { None }; INLINE],
            len: 0,
            heap: Vec::new(),
        }
    }
}

impl<'kvs> VisitSource<'kvs> for Pairs<'kvs> {
    fn visit_pair(&mut self, key: Key<'kvs>, value: kv::Value<'kvs>) -> Result<(), kv::Error> {
        if !self.heap.is_empty() {
            self.heap.push((key, value));
        } else if let Some(slot) = self.inline.get_mut(self.len) {
            *slot = Some((key, value));
            self.len += 1;
        } else {
            self.heap.reserve(INLINE * 2);
            self.heap
                .extend(self.inline.iter_mut().filter_map(Option::take));
            self.heap.push((key, value));
        }
        Ok(())
    }
}

/// The primitive inside a `log::kv::Value`, if it has one that can be
/// borrowed for the value's whole lifetime.
enum Primitive<'v> {
    Str(&'v str),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    F64(f64),
    Bool(bool),
    Char(char),
    Null,
    Other,
}

struct Probe<'v>(Primitive<'v>);

impl<'v> VisitValue<'v> for Probe<'v> {
    fn visit_any(&mut self, _value: kv::Value<'_>) -> Result<(), kv::Error> {
        self.0 = Primitive::Other;
        Ok(())
    }
    fn visit_null(&mut self) -> Result<(), kv::Error> {
        self.0 = Primitive::Null;
        Ok(())
    }
    fn visit_u64(&mut self, value: u64) -> Result<(), kv::Error> {
        self.0 = Primitive::U64(value);
        Ok(())
    }
    fn visit_i64(&mut self, value: i64) -> Result<(), kv::Error> {
        self.0 = Primitive::I64(value);
        Ok(())
    }
    fn visit_u128(&mut self, value: u128) -> Result<(), kv::Error> {
        self.0 = Primitive::U128(value);
        Ok(())
    }
    fn visit_i128(&mut self, value: i128) -> Result<(), kv::Error> {
        self.0 = Primitive::I128(value);
        Ok(())
    }
    fn visit_f64(&mut self, value: f64) -> Result<(), kv::Error> {
        self.0 = Primitive::F64(value);
        Ok(())
    }
    fn visit_bool(&mut self, value: bool) -> Result<(), kv::Error> {
        self.0 = Primitive::Bool(value);
        Ok(())
    }
    fn visit_str(&mut self, _value: &str) -> Result<(), kv::Error> {
        // Not borrowable for 'v; fall back to Display.
        self.0 = Primitive::Other;
        Ok(())
    }
    fn visit_borrowed_str(&mut self, value: &'v str) -> Result<(), kv::Error> {
        self.0 = Primitive::Str(value);
        Ok(())
    }
    fn visit_char(&mut self, value: char) -> Result<(), kv::Error> {
        self.0 = Primitive::Char(value);
        Ok(())
    }
}

fn convert<'a>(value: &'a kv::Value<'_>) -> Value<'a> {
    let mut probe = Probe(Primitive::Other);
    if value.visit(&mut probe).is_err() {
        return Value::Display(value as &dyn fmt::Display);
    }
    match probe.0 {
        Primitive::Str(s) => Value::Str(s),
        Primitive::I64(n) => Value::I64(n),
        Primitive::U64(n) => Value::U64(n),
        Primitive::I128(n) => Value::I128(n),
        Primitive::U128(n) => Value::U128(n),
        Primitive::F64(n) => Value::F64(n),
        Primitive::Bool(b) => Value::Bool(b),
        Primitive::Char(c) => Value::Char(c),
        Primitive::Null => Value::Null,
        Primitive::Other => Value::Display(value as &dyn fmt::Display),
    }
}

#[cfg(all(test, feature = "logfmt"))]
mod tests {
    use super::*;
    use crate::format::LogfmtFormat;
    use log::Log;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

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

    fn bridge(level: Level) -> (LogBridge, Capture) {
        let out = Capture::default();
        let logger = Logger::builder()
            .level(level)
            .no_timestamps()
            .no_context()
            .writer(out.clone(), LogfmtFormat::new())
            .build();
        (LogBridge::new(logger), out)
    }

    #[test]
    fn test_log_bridge_maps_levels_targets_and_location() {
        let (bridge, out) = bridge(Level::Trace);
        let n = 3;
        bridge.log(
            &log::Record::builder()
                .level(log::Level::Warn)
                .target("app::db")
                .file(Some("src/db.rs"))
                .line(Some(9))
                .args(format_args!("slow by {n}x"))
                .build(),
        );
        assert_eq!(
            out.take(),
            "level=warn target=app::db message=\"slow by 3x\" file=src/db.rs line=9\n"
        );
        for (log_level, ours) in [
            (log::Level::Error, "error"),
            (log::Level::Info, "info"),
            (log::Level::Debug, "debug"),
            (log::Level::Trace, "trace"),
        ] {
            bridge.log(
                &log::Record::builder()
                    .level(log_level)
                    .args(format_args!("x"))
                    .build(),
            );
            assert!(out.take().starts_with(&format!("level={ours} ")));
        }
    }

    #[test]
    fn test_log_bridge_respects_filter() {
        let (bridge, out) = bridge(Level::Warn);
        let meta = log::Metadata::builder()
            .level(log::Level::Info)
            .target("x")
            .build();
        assert!(!bridge.enabled(&meta));
        bridge.log(
            &log::Record::builder()
                .level(log::Level::Info)
                .args(format_args!("no"))
                .build(),
        );
        assert_eq!(out.take(), "");
        let meta = log::Metadata::builder()
            .level(log::Level::Error)
            .target("x")
            .build();
        assert!(bridge.enabled(&meta));
    }

    #[test]
    fn test_log_bridge_key_values_keep_types() {
        let (bridge, out) = bridge(Level::Trace);
        let addr = std::net::Ipv4Addr::LOCALHOST;
        let kvs: &[(&str, kv::Value<'_>)] = &[
            ("user", kv::Value::from("alice")),
            ("n", kv::Value::from(-3_i64)),
            ("u", kv::Value::from(7_u64)),
            ("big", kv::Value::from(u128::MAX)),
            ("neg", kv::Value::from(i128::MIN)),
            ("ratio", kv::Value::from(0.5_f64)),
            ("ok", kv::Value::from(true)),
            ("c", kv::Value::from('z')),
            ("addr", kv::Value::from_display(&addr)),
            ("none", kv::Value::null()),
        ];
        bridge.log(
            &log::Record::builder()
                .level(log::Level::Info)
                .target("t")
                .key_values(&kvs)
                .args(format_args!("m"))
                .build(),
        );
        assert_eq!(
            out.take(),
            format!(
                "level=info target=t message=m user=alice n=-3 u=7 big={} neg={} ratio=0.5 ok=true c=z addr=127.0.0.1 none=\n",
                u128::MAX,
                i128::MIN
            )
        );
    }

    #[test]
    fn test_log_bridge_many_key_values_spill_to_heap() {
        let (bridge, out) = bridge(Level::Trace);
        let keys: Vec<String> = (0..20).map(|i| format!("k{i}")).collect();
        let kvs: Vec<(&str, kv::Value<'_>)> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_str(), kv::Value::from(i as u64)))
            .collect();
        bridge.log(
            &log::Record::builder()
                .level(log::Level::Info)
                .key_values(&kvs)
                .args(format_args!("many"))
                .build(),
        );
        let text = out.take();
        for (i, k) in keys.iter().enumerate() {
            assert!(text.contains(&format!(" {k}={i}")), "{k}: {text}");
        }
    }

    #[test]
    fn test_log_bridge_flush_and_accessors() {
        let (bridge, _out) = bridge(Level::Info);
        bridge.flush();
        assert_eq!(bridge.logger().min_level(), Level::Info);
        assert_eq!(to_level_filter(Level::Trace), log::LevelFilter::Trace);
        assert_eq!(to_level_filter(Level::Debug), log::LevelFilter::Debug);
        assert_eq!(to_level_filter(Level::Info), log::LevelFilter::Info);
        assert_eq!(to_level_filter(Level::Error), log::LevelFilter::Error);
    }
}

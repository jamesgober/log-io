//! The top-level [`Logger`] and its [`LoggerBuilder`].
//!
//! A `Logger` bundles a [`Filter`] gate, one or more [`Sink`]s, and
//! (when std is available) timestamping and thread-local context
//! propagation. Cloning a `Logger` clones a pair of `Arc`s; the
//! underlying sinks are shared.

use std::io::Write;
use std::sync::Arc;

use crate::context;
use crate::error::Result;
use crate::filter::Filter;
use crate::format::Format;
use crate::level::Level;
use crate::record::{Field, Metadata, Record};
use crate::sink::{Sink, StderrSink, StdoutSink, WriterSink};
use crate::time;
use crate::value::Value;

/// Owned counterpart of [`Value`], used for default fields stored in
/// the logger.
#[derive(Debug, Clone)]
enum OwnedValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Char(char),
}

impl OwnedValue {
    fn as_value(&self) -> Value<'_> {
        match self {
            Self::Null => Value::Null,
            Self::Bool(b) => Value::Bool(*b),
            Self::I64(n) => Value::I64(*n),
            Self::U64(n) => Value::U64(*n),
            Self::F64(n) => Value::F64(*n),
            Self::Str(s) => Value::Str(s.as_str()),
            Self::Char(c) => Value::Char(*c),
        }
    }

    fn from_value(v: Value<'_>) -> Self {
        match v {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Bool(b),
            Value::I64(n) => Self::I64(n),
            Value::U64(n) => Self::U64(n),
            Value::F64(n) => Self::F64(n),
            Value::Str(s) => Self::Str(s.to_owned()),
            Value::Char(c) => Self::Char(c),
        }
    }
}

#[derive(Debug, Clone)]
struct OwnedField {
    key: String,
    value: OwnedValue,
}

/// Optional handler invoked when a sink returns an error.
///
/// The default behavior of [`Logger::log`] is to silently swallow sink
/// errors. Install an error handler to surface them (typically to
/// stderr) without taking down the calling thread.
pub type ErrorHandler = Arc<dyn Fn(&crate::error::Error) + Send + Sync>;

/// The configured log pipeline.
#[derive(Clone)]
pub struct Logger {
    inner: Arc<LoggerInner>,
}

struct LoggerInner {
    filter: Filter,
    sinks: Box<[Arc<dyn Sink>]>,
    default_fields: Box<[OwnedField]>,
    capture_timestamps: bool,
    capture_context: bool,
    on_error: Option<ErrorHandler>,
}

impl Logger {
    /// Start configuring a logger.
    #[must_use]
    pub fn builder() -> LoggerBuilder {
        LoggerBuilder {
            filter: Filter::new(Level::Info),
            sinks: Vec::new(),
            default_fields: Vec::new(),
            capture_timestamps: true,
            capture_context: true,
            on_error: None,
        }
    }

    /// The configured filter threshold for `target`.
    #[must_use]
    pub fn threshold_for(&self, target: &str) -> Level {
        self.inner.filter.threshold_for(target)
    }

    /// Returns `true` if a record with this severity and target would
    /// pass the filter. Use in hot paths to skip field construction
    /// before [`Self::log`].
    #[must_use]
    pub fn enabled(&self, target: &str, level: Level) -> bool {
        self.inner.filter.is_enabled(target, level)
    }

    /// The lowest severity the filter admits anywhere. Useful for
    /// short-circuiting macros at the call site.
    #[must_use]
    pub fn min_level(&self) -> Level {
        let mut min = self.inner.filter.default_level();
        for rule in self.inner.filter.rules() {
            if rule.level < min {
                min = rule.level;
            }
        }
        min
    }

    /// Emit a record using the empty string as the target. The
    /// simplest entry point; macros prefer [`Self::try_emit`] so they
    /// can attach source location.
    pub fn log(&self, level: Level, message: &str, fields: &[Field<'_>]) {
        let _ = self.try_log(level, "", message, fields);
    }

    /// Emit a record at `level` with explicit `target`.
    ///
    /// Sink errors are passed to the configured error handler (if any)
    /// and then returned. Use [`Self::log`] for fire-and-forget calls.
    ///
    /// # Errors
    ///
    /// Returns the first sink error; remaining sinks still receive the
    /// record so a transient failure on one destination does not
    /// starve the others.
    pub fn try_log(
        &self,
        level: Level,
        target: &str,
        message: &str,
        fields: &[Field<'_>],
    ) -> Result<()> {
        let metadata = Metadata::new(level, target);
        self.try_emit(metadata, message, fields)
    }

    /// Lowest-level emission entry point. Accepts a fully-constructed
    /// [`Metadata`] so the caller (typically a macro) can attach
    /// source location.
    ///
    /// The logger augments the metadata with a timestamp (if enabled)
    /// before dispatch. Filtering uses `metadata.target` and
    /// `metadata.level`.
    ///
    /// # Errors
    ///
    /// Returns the first sink error encountered. Other sinks are still
    /// attempted; see the type-level docs on dispatch semantics.
    pub fn try_emit(
        &self,
        mut metadata: Metadata<'_>,
        message: &str,
        fields: &[Field<'_>],
    ) -> Result<()> {
        if !self
            .inner
            .filter
            .is_enabled(metadata.target, metadata.level)
        {
            return Ok(());
        }
        if self.inner.capture_timestamps && metadata.timestamp_unix_nanos.is_none() {
            metadata = metadata.with_timestamp(time::now_unix_nanos());
        }

        let result = if self.inner.capture_context {
            context::with_snapshot(|ctx_fields| {
                self.dispatch_with_context(metadata, message, fields, ctx_fields)
            })
        } else {
            self.dispatch_with_context(metadata, message, fields, &[])
        };

        if let Err(ref e) = result {
            if let Some(handler) = &self.inner.on_error {
                handler(e);
            }
        }
        result
    }

    /// Flush every configured sink. Use during graceful shutdown to
    /// drain buffered output.
    ///
    /// # Errors
    ///
    /// Returns the first flush error. Other sinks are still flushed.
    pub fn flush(&self) -> Result<()> {
        let mut first_err: Option<crate::error::Error> = None;
        for sink in self.inner.sinks.iter() {
            if let Err(e) = sink.flush() {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Number of installed sinks. Useful for assertions in tests and
    /// for logging the logger's own configuration.
    #[must_use]
    pub fn sink_count(&self) -> usize {
        self.inner.sinks.len()
    }

    fn dispatch_with_context(
        &self,
        metadata: Metadata<'_>,
        message: &str,
        fields: &[Field<'_>],
        ctx_fields: &[Field<'_>],
    ) -> Result<()> {
        // Merge logger-default fields, thread-local context, and call
        // fields into the record's `context` slot. Per-call fields go
        // in `fields` so formatters can still distinguish them.
        //
        // Materialize default fields onto the stack via a small inline
        // buffer; on overflow fall back to a Vec.
        if self.inner.default_fields.is_empty() {
            let record = Record::with_context(metadata, message, fields, ctx_fields);
            return self.dispatch(&record);
        }

        // Stack buffer large enough for typical cases.
        const STACK: usize = 16;
        let total = self.inner.default_fields.len() + ctx_fields.len();
        if total <= STACK {
            let mut buf: [Field<'_>; STACK] = [Field {
                key: "",
                value: Value::Null,
            }; STACK];
            let mut n = 0;
            for f in self.inner.default_fields.iter() {
                buf[n] = Field::new(f.key.as_str(), f.value.as_value());
                n += 1;
            }
            for f in ctx_fields {
                buf[n] = *f;
                n += 1;
            }
            let record = Record::with_context(metadata, message, fields, &buf[..n]);
            self.dispatch(&record)
        } else {
            let mut merged: Vec<Field<'_>> = Vec::with_capacity(total);
            for f in self.inner.default_fields.iter() {
                merged.push(Field::new(f.key.as_str(), f.value.as_value()));
            }
            merged.extend_from_slice(ctx_fields);
            let record = Record::with_context(metadata, message, fields, &merged);
            self.dispatch(&record)
        }
    }

    fn dispatch(&self, record: &Record<'_>) -> Result<()> {
        let mut first_err: Option<crate::error::Error> = None;
        for sink in self.inner.sinks.iter() {
            if let Err(e) = sink.write_record(record) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl core::fmt::Debug for Logger {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Logger")
            .field("filter", &self.inner.filter)
            .field("sinks", &self.inner.sinks.len())
            .field("default_fields", &self.inner.default_fields.len())
            .field("capture_timestamps", &self.inner.capture_timestamps)
            .field("capture_context", &self.inner.capture_context)
            .finish()
    }
}

/// Builder for [`Logger`]. Construct with [`Logger::builder`].
///
/// The builder is not generic over a format. Each call that installs
/// a sink ([`Self::with_sink`], [`Self::stdout`], [`Self::stderr`],
/// [`Self::writer`], or one of the format-specific shortcuts) carries
/// its own format.
pub struct LoggerBuilder {
    filter: Filter,
    sinks: Vec<Arc<dyn Sink>>,
    default_fields: Vec<OwnedField>,
    capture_timestamps: bool,
    capture_context: bool,
    on_error: Option<ErrorHandler>,
}

impl LoggerBuilder {
    /// Set the default minimum severity.
    #[must_use]
    pub fn level(mut self, level: Level) -> Self {
        let prior_rules = self.filter.rules().to_vec();
        self.filter = Filter::new(level);
        for rule in prior_rules {
            self.filter = self.filter.with_rule(rule.target, rule.level);
        }
        self
    }

    /// Replace the filter wholesale.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = filter;
        self
    }

    /// Add a per-target filter override. See [`Filter::with_rule`].
    #[must_use]
    pub fn target_level(mut self, target: impl Into<String>, level: Level) -> Self {
        self.filter = self.filter.with_rule(target, level);
        self
    }

    /// Parse `directive` and replace the filter. Silently retains the
    /// previous filter on parse error; use [`Filter::parse`] up front
    /// if you need to surface the error.
    #[must_use]
    pub fn filter_directive(mut self, directive: &str) -> Self {
        if let Ok(f) = Filter::parse(directive) {
            self.filter = f;
        }
        self
    }

    /// Disable timestamp capture. By default the logger records a
    /// timestamp on every record using [`std::time::SystemTime`].
    #[must_use]
    pub fn no_timestamps(mut self) -> Self {
        self.capture_timestamps = false;
        self
    }

    /// Disable thread-local context capture.
    #[must_use]
    pub fn no_context(mut self) -> Self {
        self.capture_context = false;
        self
    }

    /// Attach a field that the logger emits on every record. Useful
    /// for service-level identifiers like `service_name` or `version`.
    #[must_use]
    pub fn with_default_field<'a, V: Into<Value<'a>>>(
        mut self,
        key: impl Into<String>,
        value: V,
    ) -> Self {
        self.default_fields.push(OwnedField {
            key: key.into(),
            value: OwnedValue::from_value(value.into()),
        });
        self
    }

    /// Install a callback invoked when a sink returns an error.
    #[must_use]
    pub fn on_error(mut self, handler: ErrorHandler) -> Self {
        self.on_error = Some(handler);
        self
    }

    /// Install a sink directly. Use this when you have a pre-built
    /// sink (typical for [`crate::sink::FileSink`] which is fallible
    /// to open).
    #[must_use]
    pub fn with_sink(mut self, sink: impl Sink + 'static) -> Self {
        self.sinks.push(Arc::new(sink));
        self
    }

    /// Install a stdout sink with the given format.
    #[must_use]
    pub fn stdout<F>(mut self, format: F) -> Self
    where
        F: Format + Send + Sync + 'static,
    {
        self.sinks.push(Arc::new(StdoutSink::new(format)));
        self
    }

    /// Install a stderr sink with the given format.
    #[must_use]
    pub fn stderr<F>(mut self, format: F) -> Self
    where
        F: Format + Send + Sync + 'static,
    {
        self.sinks.push(Arc::new(StderrSink::new(format)));
        self
    }

    /// Install a sink that writes to `writer` with `format`.
    #[must_use]
    pub fn writer<W, F>(mut self, writer: W, format: F) -> Self
    where
        W: Write + Send + 'static,
        F: Format + Send + Sync + 'static,
    {
        self.sinks.push(Arc::new(WriterSink::new(writer, format)));
        self
    }

    /// Finish configuration. Panics if no sink has been installed.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::with_sink`], [`Self::stdout`], [`Self::stderr`],
    /// [`Self::writer`], or one of the format shortcuts has not been
    /// called. Building a logger without a destination is almost
    /// certainly a programmer error.
    #[must_use]
    pub fn build(self) -> Logger {
        assert!(
            !self.sinks.is_empty(),
            "log_io: builder must install at least one sink before build()"
        );
        Logger {
            inner: Arc::new(LoggerInner {
                filter: self.filter,
                sinks: self.sinks.into_boxed_slice(),
                default_fields: self.default_fields.into_boxed_slice(),
                capture_timestamps: self.capture_timestamps,
                capture_context: self.capture_context,
                on_error: self.on_error,
            }),
        }
    }
}

// Format-shortcut methods. These are feature-gated and produce the
// most common (stdout, stderr) sinks pre-wired to a built-in format.

#[cfg(feature = "json")]
impl LoggerBuilder {
    /// Install a stdout sink with the default JSON format.
    #[must_use]
    pub fn stdout_json(self) -> Self {
        self.stdout(crate::format::JsonFormat::new())
    }

    /// Install a stderr sink with the default JSON format.
    #[must_use]
    pub fn stderr_json(self) -> Self {
        self.stderr(crate::format::JsonFormat::new())
    }
}

#[cfg(feature = "logfmt")]
impl LoggerBuilder {
    /// Install a stdout sink with the logfmt format.
    #[must_use]
    pub fn stdout_logfmt(self) -> Self {
        self.stdout(crate::format::LogfmtFormat::new())
    }

    /// Install a stderr sink with the logfmt format.
    #[must_use]
    pub fn stderr_logfmt(self) -> Self {
        self.stderr(crate::format::LogfmtFormat::new())
    }
}

#[cfg(feature = "human")]
impl LoggerBuilder {
    /// Install a stdout sink with the human-readable format.
    #[must_use]
    pub fn stdout_human(self) -> Self {
        self.stdout(crate::format::HumanFormat::new())
    }

    /// Install a stderr sink with the human-readable format.
    #[must_use]
    pub fn stderr_human(self) -> Self {
        self.stderr(crate::format::HumanFormat::new())
    }
}

#[cfg(all(test, feature = "std", feature = "logfmt", feature = "json"))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

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

    fn logfmt_logger() -> (Logger, MemBuf) {
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .no_context()
            .writer(buf.clone(), crate::format::LogfmtFormat::new())
            .build();
        (logger, buf)
    }

    #[test]
    fn enabled_respects_filter() {
        let (logger, _) = logfmt_logger();
        assert!(logger.enabled("x", Level::Trace));
        assert!(logger.enabled("x", Level::Error));
    }

    #[test]
    fn min_level_is_lowest_configured() {
        let logger = Logger::builder()
            .level(Level::Warn)
            .target_level("loud", Level::Trace)
            .no_timestamps()
            .no_context()
            .with_sink(crate::sink::NullSink::new())
            .build();
        assert_eq!(logger.min_level(), Level::Trace);
    }

    #[test]
    fn log_writes_through_logfmt() {
        let (logger, buf) = logfmt_logger();
        logger.log(Level::Info, "hi", &[Field::new("k", 1_u64)]);
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("level=info"));
        assert!(out.contains("message=hi"));
        assert!(out.contains("k=1"));
    }

    #[test]
    fn filter_blocks_records_below_threshold() {
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Warn)
            .no_timestamps()
            .no_context()
            .writer(buf.clone(), crate::format::LogfmtFormat::new())
            .build();
        logger.log(Level::Info, "skip", &[]);
        logger.log(Level::Error, "keep", &[]);
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(!out.contains("skip"));
        assert!(out.contains("keep"));
    }

    #[test]
    fn context_is_spliced_in() {
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .writer(buf.clone(), crate::format::LogfmtFormat::new())
            .build();
        crate::context::clear();
        let guard = crate::context::with_trace_id("tx-42");
        logger.log(Level::Info, "hello", &[]);
        drop(guard);
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("trace_id=tx-42"), "{out}");
    }

    #[test]
    fn default_fields_appear_on_every_record() {
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .no_context()
            .with_default_field("service", "api")
            .with_default_field("region", "us-east-1")
            .writer(buf.clone(), crate::format::LogfmtFormat::new())
            .build();
        logger.log(Level::Info, "x", &[]);
        logger.log(Level::Warn, "y", &[Field::new("port", 80_u32)]);
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert_eq!(out.matches("service=api").count(), 2);
        assert_eq!(out.matches("region=us-east-1").count(), 2);
        assert!(out.contains("port=80"));
    }

    #[test]
    fn dispatch_continues_after_sink_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct FailingSink(AtomicUsize);
        impl Sink for FailingSink {
            fn write_record(&self, _record: &Record<'_>) -> Result<()> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Err(crate::error::Error::Configuration("boom"))
            }
            fn flush(&self) -> Result<()> {
                Ok(())
            }
        }
        let failing: Arc<FailingSink> = Arc::new(FailingSink(AtomicUsize::new(0)));
        let buf = MemBuf::default();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .no_context()
            .with_sink(failing.clone())
            .writer(buf.clone(), crate::format::JsonFormat::new())
            .build();
        let _ = logger.try_log(Level::Info, "t", "m", &[]);
        // Second sink still wrote even though first failed.
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("\"message\":\"m\""), "{out}");
        assert_eq!(failing.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn on_error_callback_fires() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Bad;
        impl Sink for Bad {
            fn write_record(&self, _r: &Record<'_>) -> Result<()> {
                Err(crate::error::Error::Configuration("nope"))
            }
            fn flush(&self) -> Result<()> {
                Ok(())
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .no_context()
            .with_sink(Bad)
            .on_error(Arc::new(move |_| {
                count_clone.fetch_add(1, Ordering::Relaxed);
            }))
            .build();
        let _ = logger.try_log(Level::Info, "t", "m", &[]);
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }
}

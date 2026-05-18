//! The top-level [`Logger`] and its [`LoggerBuilder`].
//!
//! A `Logger` is the entry point an application code calls. It bundles
//! a [`Filter`] gate, one or more [`Sink`]s, and (when std is
//! available) timestamping and thread-local context propagation.

use std::sync::Arc;

use crate::context;
use crate::error::Result;
use crate::filter::Filter;
use crate::format::Format;
use crate::level::Level;
use crate::record::{Field, Metadata, Record};
use crate::sink::{FileSink, NullSink, Sink, StderrSink, StdoutSink, WriterSink};
use crate::time;

/// The configured log pipeline.
///
/// Cloning a `Logger` clones a pair of `Arc`s; it does not duplicate
/// the underlying sinks. Pass `Logger` by value to long-lived owners
/// and borrow elsewhere.
#[derive(Clone)]
pub struct Logger {
    filter: Arc<Filter>,
    sinks: Arc<[Arc<dyn Sink>]>,
    capture_timestamps: bool,
    capture_context: bool,
}

impl Logger {
    /// Start configuring a logger.
    pub fn builder() -> LoggerBuilder<NoFormat> {
        LoggerBuilder {
            filter: Filter::new(Level::Info),
            sinks: Vec::new(),
            format: NoFormat,
            sink_intent: SinkIntent::None,
            capture_timestamps: true,
            capture_context: true,
        }
    }

    /// The configured filter threshold for `target`.
    pub fn threshold_for(&self, target: &str) -> Level {
        self.filter.threshold_for(target)
    }

    /// Returns `true` if a record with this severity and target would
    /// pass the filter. Use this in hot paths to skip allocation of
    /// fields before calling [`Self::log`].
    pub fn enabled(&self, target: &str, level: Level) -> bool {
        self.filter.is_enabled(target, level)
    }

    /// The lowest severity the filter admits anywhere. Useful for
    /// short-circuiting macros at the call site.
    pub fn min_level(&self) -> Level {
        let mut min = self.filter.default_level();
        for rule in self.filter.rules() {
            if rule.level < min {
                min = rule.level;
            }
        }
        min
    }

    /// Emit a record at `level` with `target`, `message`, and `fields`.
    ///
    /// This is the lowest-level entry point. The macros forward to it.
    /// Errors from sinks are returned as-is; callers that want a
    /// fire-and-forget API can use [`Self::log`] instead.
    ///
    /// # Errors
    ///
    /// Surfaces the first sink error. Subsequent sinks are not
    /// invoked.
    pub fn try_log_with_target(
        &self,
        level: Level,
        target: &str,
        message: &str,
        fields: &[Field<'_>],
    ) -> Result<()> {
        if !self.filter.is_enabled(target, level) {
            return Ok(());
        }
        let mut metadata = Metadata::new(level, target);
        if self.capture_timestamps {
            metadata = metadata.with_timestamp(time::now_unix_nanos());
        }

        if self.capture_context {
            context::with_snapshot(|ctx_fields| {
                let record = Record::with_context(metadata, message, fields, ctx_fields);
                self.dispatch(&record)
            })
        } else {
            let record = Record::new(metadata, message, fields);
            self.dispatch(&record)
        }
    }

    /// Emit a record using the calling module path as the target.
    ///
    /// Errors are silently swallowed; for a fallible variant call
    /// [`Self::try_log_with_target`].
    pub fn log(&self, level: Level, message: &str, fields: &[Field<'_>]) {
        let _ = self.try_log_with_target(level, "", message, fields);
    }

    /// Flush every configured sink. Use during graceful shutdown to
    /// drain any buffered output.
    ///
    /// # Errors
    ///
    /// Surfaces the first sink flush error.
    pub fn flush(&self) -> Result<()> {
        for sink in self.sinks.iter() {
            sink.flush()?;
        }
        Ok(())
    }

    fn dispatch(&self, record: &Record<'_>) -> Result<()> {
        for sink in self.sinks.iter() {
            sink.write_record(record)?;
        }
        Ok(())
    }
}

impl core::fmt::Debug for Logger {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Logger")
            .field("filter", &self.filter)
            .field("sink_count", &self.sinks.len())
            .field("capture_timestamps", &self.capture_timestamps)
            .field("capture_context", &self.capture_context)
            .finish()
    }
}

/// Placeholder format used until the builder is told what to emit.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct NoFormat;

impl Format for NoFormat {
    fn write_record<W: core::fmt::Write + ?Sized>(
        &self,
        _record: &Record<'_>,
        _writer: &mut W,
    ) -> core::fmt::Result {
        Ok(())
    }
}

enum SinkIntent {
    None,
    Stdout,
    Stderr,
    File {
        path: std::path::PathBuf,
        append: bool,
    },
    Writer(Box<dyn FnOnce() -> Box<dyn std::io::Write + Send> + Send>),
    Null,
}

/// Builder for [`Logger`]. Construct with [`Logger::builder`].
///
/// The builder is parameterized by its format `F`. Methods that
/// install a format return a builder with that format slotted in.
/// Calling `.build()` is only available once a format and a sink are
/// both selected.
pub struct LoggerBuilder<F> {
    filter: Filter,
    sinks: Vec<Arc<dyn Sink>>,
    format: F,
    sink_intent: SinkIntent,
    capture_timestamps: bool,
    capture_context: bool,
}

impl<F> LoggerBuilder<F> {
    /// Set the default minimum severity.
    pub fn level(mut self, level: Level) -> Self {
        self.filter = Filter::new(level);
        for rule in self.filter.rules().to_vec() {
            self.filter = self.filter.with_rule(rule.target, rule.level);
        }
        self
    }

    /// Replace the filter wholesale.
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = filter;
        self
    }

    /// Add a per-target filter rule. See [`Filter::with_rule`].
    pub fn target_level(mut self, target: impl Into<String>, level: Level) -> Self {
        self.filter = self.filter.with_rule(target, level);
        self
    }

    /// Parse `directive` (`info,app::auth=debug`) and replace the
    /// filter. Falls back to the current filter on parse error.
    pub fn filter_directive(mut self, directive: &str) -> Self {
        if let Ok(f) = Filter::parse(directive) {
            self.filter = f;
        }
        self
    }

    /// Disable timestamp capture. By default the logger records a
    /// timestamp on every record using [`std::time::SystemTime`].
    pub fn no_timestamps(mut self) -> Self {
        self.capture_timestamps = false;
        self
    }

    /// Disable thread-local context capture. By default the logger
    /// splices the current thread's context fields into every record.
    pub fn no_context(mut self) -> Self {
        self.capture_context = false;
        self
    }

    /// Append a pre-built sink. Records are fanned out to every
    /// installed sink in insertion order.
    pub fn add_sink(mut self, sink: impl Sink + 'static) -> Self {
        self.sinks.push(Arc::new(sink) as Arc<dyn Sink>);
        self
    }

    /// Route the next-installed format to stdout.
    pub fn stdout(mut self) -> Self {
        self.sink_intent = SinkIntent::Stdout;
        self
    }

    /// Route the next-installed format to stderr.
    pub fn stderr(mut self) -> Self {
        self.sink_intent = SinkIntent::Stderr;
        self
    }

    /// Route the next-installed format to a file (truncating).
    pub fn file<P: Into<std::path::PathBuf>>(mut self, path: P) -> Self {
        self.sink_intent = SinkIntent::File {
            path: path.into(),
            append: false,
        };
        self
    }

    /// Route the next-installed format to a file (appending).
    pub fn file_append<P: Into<std::path::PathBuf>>(mut self, path: P) -> Self {
        self.sink_intent = SinkIntent::File {
            path: path.into(),
            append: true,
        };
        self
    }

    /// Route the next-installed format to a custom writer factory.
    ///
    /// The closure is invoked at `build` time, on the calling thread.
    /// Used by tests that capture output into a memory buffer.
    pub fn writer<W, MakeW>(mut self, factory: MakeW) -> Self
    where
        W: std::io::Write + Send + 'static,
        MakeW: FnOnce() -> W + Send + 'static,
    {
        self.sink_intent = SinkIntent::Writer(Box::new(move || Box::new(factory())));
        self
    }

    /// Route the next-installed format to a sink that discards records.
    pub fn null(mut self) -> Self {
        self.sink_intent = SinkIntent::Null;
        self
    }
}

// Format-installing methods. Each consumes the builder and returns a
// builder with the format slotted in, ready for build().
impl<F> LoggerBuilder<F> {
    fn replace_format<G: Format>(self, format: G) -> LoggerBuilder<G> {
        LoggerBuilder {
            filter: self.filter,
            sinks: self.sinks,
            format,
            sink_intent: self.sink_intent,
            capture_timestamps: self.capture_timestamps,
            capture_context: self.capture_context,
        }
    }
}

#[cfg(feature = "json")]
impl<F> LoggerBuilder<F> {
    /// Use compact JSON for the most recently selected output target.
    pub fn json(self) -> LoggerBuilder<crate::format::JsonFormat> {
        self.replace_format(crate::format::JsonFormat::new())
    }

    /// Use a configured JSON formatter.
    pub fn json_with(
        self,
        fmt: crate::format::JsonFormat,
    ) -> LoggerBuilder<crate::format::JsonFormat> {
        self.replace_format(fmt)
    }
}

#[cfg(feature = "logfmt")]
impl<F> LoggerBuilder<F> {
    /// Use logfmt output for the most recently selected output target.
    pub fn logfmt(self) -> LoggerBuilder<crate::format::LogfmtFormat> {
        self.replace_format(crate::format::LogfmtFormat::new())
    }
}

#[cfg(feature = "human")]
impl<F> LoggerBuilder<F> {
    /// Use the human-readable format for the most recently selected
    /// output target.
    pub fn human(self) -> LoggerBuilder<crate::format::HumanFormat> {
        self.replace_format(crate::format::HumanFormat::new())
    }

    /// Use a configured human formatter.
    pub fn human_with(
        self,
        fmt: crate::format::HumanFormat,
    ) -> LoggerBuilder<crate::format::HumanFormat> {
        self.replace_format(fmt)
    }
}

impl<F: Format + Clone + 'static> LoggerBuilder<F> {
    /// Finish configuration and return a usable [`Logger`].
    ///
    /// # Panics
    ///
    /// Panics if no sink has been configured or a configured
    /// file sink fails to open. To handle the latter explicitly,
    /// build a [`crate::sink::FileSink`] directly and pass it to
    /// [`Self::add_sink`].
    pub fn build(mut self) -> Logger {
        let intent = std::mem::replace(&mut self.sink_intent, SinkIntent::None);
        match intent {
            SinkIntent::None => {
                if self.sinks.is_empty() {
                    panic!(
                        "log_io: builder must configure at least one sink before calling build()"
                    );
                }
            }
            SinkIntent::Stdout => {
                self.sinks
                    .push(Arc::new(StdoutSink::new(self.format.clone())));
            }
            SinkIntent::Stderr => {
                self.sinks
                    .push(Arc::new(StderrSink::new(self.format.clone())));
            }
            SinkIntent::File { path, append } => {
                let sink = if append {
                    FileSink::append(&path, self.format.clone())
                } else {
                    FileSink::create(&path, self.format.clone())
                }
                .unwrap_or_else(|e| {
                    panic!("log_io: failed to open file sink {}: {e}", path.display())
                });
                self.sinks.push(Arc::new(sink));
            }
            SinkIntent::Writer(factory) => {
                let writer = factory();
                self.sinks
                    .push(Arc::new(WriterSink::new(writer, self.format.clone())));
            }
            SinkIntent::Null => {
                self.sinks.push(Arc::new(NullSink::new()));
            }
        }
        Logger {
            filter: Arc::new(self.filter),
            sinks: Arc::from(self.sinks.into_boxed_slice()),
            capture_timestamps: self.capture_timestamps,
            capture_context: self.capture_context,
        }
    }
}

#[cfg(test)]
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

    fn capture_logger() -> (Logger, MemBuf) {
        let buf = MemBuf::default();
        let writer = buf.clone();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .no_context()
            .writer(move || writer)
            .logfmt()
            .build();
        (logger, buf)
    }

    #[test]
    fn enabled_respects_filter() {
        let (logger, _) = capture_logger();
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
            .null()
            .json()
            .build();
        assert_eq!(logger.min_level(), Level::Trace);
    }

    #[test]
    fn log_writes_through_logfmt() {
        let (logger, buf) = capture_logger();
        logger.log(Level::Info, "hi", &[Field::new("k", 1_u64)]);
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("level=info"));
        assert!(out.contains("message=hi"));
        assert!(out.contains("k=1"));
    }

    #[test]
    fn filter_blocks_records_below_threshold() {
        let buf = MemBuf::default();
        let writer = buf.clone();
        let logger = Logger::builder()
            .level(Level::Warn)
            .no_timestamps()
            .no_context()
            .writer(move || writer)
            .logfmt()
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
        let writer = buf.clone();
        let logger = Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .writer(move || writer)
            .logfmt()
            .build();
        crate::context::clear();
        let guard = crate::context::with_trace_id("tx-42");
        logger.log(Level::Info, "hello", &[]);
        drop(guard);
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains("trace_id=tx-42"), "{out}");
    }
}

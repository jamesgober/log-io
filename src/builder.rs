//! [`LoggerBuilder`] and [`BuildError`].

use std::ffi::OsString;
use std::io::Write;

use crate::filter::{Filter, ParseFilterError};
use crate::format::Format;
use crate::level::Level;
use crate::logger::{ErrorHandler, Logger, LoggerInner};
use crate::sink::{Sink, StderrSink, StdoutSink, WriterSink};
use crate::value::{OwnedField, Value};

/// Error returned by [`LoggerBuilder::try_build`].
///
/// # Examples
///
/// ```
/// use log_io::{BuildError, Logger};
///
/// let err = Logger::builder().try_build().unwrap_err();
/// assert!(matches!(err, BuildError::NoSink));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildError {
    /// No sink was installed. Add one with [`LoggerBuilder::with_sink`],
    /// [`LoggerBuilder::writer`], or a shortcut such as
    /// [`LoggerBuilder::stdout_json`].
    NoSink,
    /// A directive passed to [`LoggerBuilder::filter_directive`] or read
    /// by [`LoggerBuilder::env_filter`] did not parse. Fix the directive
    /// (the error names the offending segment).
    InvalidFilter(ParseFilterError),
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoSink => f.write_str("logger has no sink; install at least one before building"),
            Self::InvalidFilter(e) => write!(f, "logger filter is invalid: {e}"),
        }
    }
}

impl std::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoSink => None,
            Self::InvalidFilter(e) => Some(e),
        }
    }
}

/// Builder for [`Logger`]. Create with [`Logger::builder`].
///
/// Defaults: filter `info` for every target, timestamps on, thread-local
/// context capture on, flush after every record at [`Level::Error`] or
/// above, no default fields, no redaction, empty default target, no
/// error handler, no sinks.
///
/// # Examples
///
/// ```
/// use log_io::{Level, Logger};
///
/// let logger = Logger::builder()
///     .level(Level::Info)
///     .env_filter("LOG_IO")              // e.g. LOG_IO="info,app::db=debug"
///     .with_default_field("service", "api")
///     .redact_common_secrets()
///     .stderr_human()
///     .try_build()?;
/// # let _ = logger;
/// # Ok::<(), log_io::BuildError>(())
/// ```
#[must_use = "a builder does nothing until `build()` or `try_build()` is called"]
pub struct LoggerBuilder {
    filter: Filter,
    filter_error: Option<ParseFilterError>,
    sinks: Vec<Box<dyn Sink>>,
    defaults: Vec<OwnedField>,
    default_target: String,
    capture_timestamps: bool,
    capture_context: bool,
    flush_level: Level,
    on_error: Option<ErrorHandler>,
    redact: Vec<Box<str>>,
}

impl LoggerBuilder {
    /// Keys redacted by [`LoggerBuilder::redact_common_secrets`].
    pub const COMMON_SECRET_KEYS: &'static [&'static str] = &[
        "password",
        "passwd",
        "secret",
        "client_secret",
        "token",
        "access_token",
        "refresh_token",
        "id_token",
        "api_key",
        "apikey",
        "authorization",
        "cookie",
        "set_cookie",
        "private_key",
        "credentials",
    ];

    pub(crate) fn new() -> Self {
        Self {
            filter: Filter::new(Level::Info),
            filter_error: None,
            sinks: Vec::new(),
            defaults: Vec::new(),
            default_target: String::new(),
            capture_timestamps: true,
            capture_context: true,
            flush_level: Level::Error,
            on_error: None,
            redact: Vec::new(),
        }
    }

    /// Set the default minimum severity, keeping any per-target rules.
    pub fn level(mut self, level: Level) -> Self {
        self.filter = self.filter.with_default_level(level);
        self
    }

    /// Replace the filter wholesale.
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = filter;
        self
    }

    /// Add (or replace) a per-target rule. See [`Filter::with_rule`].
    pub fn target_level(mut self, target: impl Into<String>, level: Level) -> Self {
        self.filter = self.filter.with_rule(target, level);
        self
    }

    /// Parse `directive` (see [`Filter::parse`]) and replace the filter.
    ///
    /// A blank directive changes nothing. An invalid directive keeps
    /// the previous filter and is reported by [`Self::try_build`] as
    /// [`BuildError::InvalidFilter`]; [`Self::build`] ignores it. Use
    /// [`Self::try_filter_directive`] to handle the error immediately.
    pub fn filter_directive(mut self, directive: &str) -> Self {
        if directive.trim().is_empty() {
            return self;
        }
        match Filter::parse(directive) {
            Ok(filter) => self.filter = filter,
            Err(e) => self.filter_error = Some(e),
        }
        self
    }

    /// Parse `directive` and replace the filter, failing immediately on
    /// an invalid directive.
    ///
    /// # Errors
    ///
    /// Returns the [`ParseFilterError`] from [`Filter::parse`].
    pub fn try_filter_directive(mut self, directive: &str) -> Result<Self, ParseFilterError> {
        self.filter = Filter::parse(directive)?;
        Ok(self)
    }

    /// Configure the filter from the environment variable `var`, using
    /// the [`Filter::parse`] directive syntax (`RUST_LOG`-style), for
    /// example `.env_filter("LOG_IO")` or `.env_filter("RUST_LOG")`.
    ///
    /// If the variable is unset, empty, or blank, the filter configured
    /// so far is kept, so call [`Self::level`] first to set the
    /// fallback. An invalid value (including one that is not valid
    /// Unicode) keeps the previous filter and is reported by
    /// [`Self::try_build`].
    pub fn env_filter(self, var: &str) -> Self {
        self.apply_env_value(std::env::var_os(var))
    }

    fn apply_env_value(mut self, value: Option<OsString>) -> Self {
        match value {
            None => self,
            Some(raw) => match raw.to_str() {
                Some(text) => self.filter_directive(text),
                None => {
                    self.filter_error = Some(ParseFilterError::not_unicode(&raw));
                    self
                }
            },
        }
    }

    /// Target used by [`Logger::log`], which has no target argument.
    /// Defaults to the empty string. Per-target filter rules apply to
    /// `Logger::log` records only through this target.
    pub fn default_target(mut self, target: impl Into<String>) -> Self {
        self.default_target = target.into();
        self
    }

    /// Do not timestamp records. By default every record gets the
    /// current wall-clock time from [`std::time::SystemTime`].
    pub fn no_timestamps(mut self) -> Self {
        self.capture_timestamps = false;
        self
    }

    /// Do not attach thread-local context ([`crate::context`]).
    pub fn no_context(mut self) -> Self {
        self.capture_context = false;
        self
    }

    /// Attach a field to every record, for identifiers such as
    /// `service` or `version`. Setting the same key again replaces the
    /// value. Context and call-site fields with the same key take
    /// precedence over defaults.
    pub fn with_default_field<'a, V: Into<Value<'a>>>(
        mut self,
        key: impl Into<String>,
        value: V,
    ) -> Self {
        let key = key.into();
        let field = OwnedField::new(&key, value.into());
        if let Some(existing) = self.defaults.iter_mut().find(|f| *f.key == *key) {
            *existing = field;
        } else {
            self.defaults.push(field);
        }
        self
    }

    /// Flush every sink after each record at or above `level`. Default
    /// [`Level::Error`], so errors reach the OS before the logging call
    /// returns. [`Level::Off`] disables flush-on-level; [`Level::Trace`]
    /// flushes after every record.
    pub fn flush_on(mut self, level: Level) -> Self {
        self.flush_level = level;
        self
    }

    /// Replace the value of every field named `key` (ASCII
    /// case-insensitive, also matching the last dot-separated segment,
    /// so `password` covers `db.password`) with `"[REDACTED]"`. Applies
    /// to call-site fields, context, and default fields. The message
    /// text is never inspected.
    pub fn redact_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into().to_ascii_lowercase().into_boxed_str();
        if !self.redact.contains(&key) {
            self.redact.push(key);
        }
        self
    }

    /// Redact every key in [`Self::COMMON_SECRET_KEYS`].
    pub fn redact_common_secrets(self) -> Self {
        Self::COMMON_SECRET_KEYS
            .iter()
            .fold(self, |builder, key| builder.redact_key(*key))
    }

    /// Install a callback for sink, flush, and formatting errors. See
    /// [`ErrorHandler`].
    pub fn on_error(mut self, handler: ErrorHandler) -> Self {
        self.on_error = Some(handler);
        self
    }

    /// Install a sink.
    pub fn with_sink(mut self, sink: impl Sink + 'static) -> Self {
        self.sinks.push(Box::new(sink));
        self
    }

    /// Install a standard-output sink with `format`.
    pub fn stdout<F: Format + 'static>(self, format: F) -> Self {
        self.with_sink(StdoutSink::new(format))
    }

    /// Install a standard-error sink with `format`.
    pub fn stderr<F: Format + 'static>(self, format: F) -> Self {
        self.with_sink(StderrSink::new(format))
    }

    /// Install a sink that writes to `writer` with `format`.
    pub fn writer<W, F>(self, writer: W, format: F) -> Self
    where
        W: Write + Send + 'static,
        F: Format + 'static,
    {
        self.with_sink(WriterSink::new(writer, format))
    }

    /// Finish configuration.
    ///
    /// An invalid filter directive recorded by
    /// [`Self::filter_directive`] or [`Self::env_filter`] is ignored
    /// (the previous filter stays in effect); use [`Self::try_build`]
    /// to surface it.
    ///
    /// # Panics
    ///
    /// Panics if no sink has been installed, which is a programming
    /// error rather than a runtime condition. Use [`Self::try_build`]
    /// when the sinks depend on configuration.
    pub fn build(self) -> Logger {
        match self.into_logger(false) {
            Ok(logger) => logger,
            Err(e) => panic!("log_io: {e}"),
        }
    }

    /// Finish configuration, reporting problems as values.
    ///
    /// # Errors
    ///
    /// * [`BuildError::NoSink`] if no sink was installed.
    /// * [`BuildError::InvalidFilter`] if a filter directive (including
    ///   one read from the environment) failed to parse.
    pub fn try_build(self) -> Result<Logger, BuildError> {
        self.into_logger(true)
    }

    fn into_logger(self, strict: bool) -> Result<Logger, BuildError> {
        if self.sinks.is_empty() {
            return Err(BuildError::NoSink);
        }
        if strict {
            if let Some(e) = self.filter_error {
                return Err(BuildError::InvalidFilter(e));
            }
        }
        Ok(Logger::from_inner(LoggerInner {
            filter: self.filter,
            sinks: self.sinks.into_boxed_slice(),
            defaults: self.defaults.into_boxed_slice(),
            default_target: self.default_target.into_boxed_str(),
            capture_timestamps: self.capture_timestamps,
            capture_context: self.capture_context,
            flush_level: self.flush_level,
            on_error: self.on_error,
            redact: self.redact.into_boxed_slice(),
        }))
    }
}

// Format shortcuts for the most common sinks.

#[cfg(feature = "json")]
impl LoggerBuilder {
    /// Install a standard-output sink with [`crate::format::JsonFormat`].
    pub fn stdout_json(self) -> Self {
        self.stdout(crate::format::JsonFormat::new())
    }

    /// Install a standard-error sink with [`crate::format::JsonFormat`].
    pub fn stderr_json(self) -> Self {
        self.stderr(crate::format::JsonFormat::new())
    }
}

#[cfg(feature = "logfmt")]
impl LoggerBuilder {
    /// Install a standard-output sink with [`crate::format::LogfmtFormat`].
    pub fn stdout_logfmt(self) -> Self {
        self.stdout(crate::format::LogfmtFormat::new())
    }

    /// Install a standard-error sink with [`crate::format::LogfmtFormat`].
    pub fn stderr_logfmt(self) -> Self {
        self.stderr(crate::format::LogfmtFormat::new())
    }
}

#[cfg(feature = "human")]
impl LoggerBuilder {
    /// Install a standard-output sink with [`crate::format::HumanFormat`],
    /// coloured when standard output is a terminal (see
    /// [`crate::format::ColorChoice::Auto`]).
    pub fn stdout_human(self) -> Self {
        let color = crate::format::ColorChoice::Auto.enabled_for_stdout();
        self.stdout(crate::format::HumanFormat::new().color(color))
    }

    /// Install a standard-error sink with [`crate::format::HumanFormat`],
    /// coloured when standard error is a terminal (see
    /// [`crate::format::ColorChoice::Auto`]).
    pub fn stderr_human(self) -> Self {
        let color = crate::format::ColorChoice::Auto.enabled_for_stderr();
        self.stderr(crate::format::HumanFormat::new().color(color))
    }
}

impl core::fmt::Debug for LoggerBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LoggerBuilder")
            .field("filter", &self.filter)
            .field("filter_error", &self.filter_error)
            .field("sinks", &self.sinks.len())
            .field("default_fields", &self.defaults.len())
            .field("default_target", &self.default_target)
            .field("capture_timestamps", &self.capture_timestamps)
            .field("capture_context", &self.capture_context)
            .field("flush_level", &self.flush_level)
            .field("redacted_keys", &self.redact)
            .field("on_error", &self.on_error.is_some())
            .finish()
    }
}

#[cfg(all(test, feature = "json", feature = "logfmt", feature = "human"))]
mod tests {
    use super::*;
    use crate::filter::ParseFilterErrorKind;
    use crate::sink::NullSink;

    #[test]
    fn test_try_build_without_sink_is_no_sink_error() {
        let err = Logger::builder().try_build().unwrap_err();
        assert_eq!(err, BuildError::NoSink);
        assert!(err.to_string().contains("no sink"));
        assert!(std::error::Error::source(&err).is_none());
    }

    #[test]
    fn test_build_without_sink_panics() {
        let result = std::panic::catch_unwind(|| Logger::builder().build());
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_directive_is_reported_by_try_build_only() {
        let err = Logger::builder()
            .filter_directive("info,app=loud")
            .with_sink(NullSink::new())
            .try_build()
            .unwrap_err();
        let BuildError::InvalidFilter(inner) = &err else {
            panic!("unexpected {err:?}");
        };
        assert_eq!(inner.kind(), ParseFilterErrorKind::InvalidLevel);
        assert!(std::error::Error::source(&err).is_some());
        assert!(err.to_string().contains("app=loud"));

        let lenient = Logger::builder()
            .level(Level::Warn)
            .filter_directive("info,app=loud")
            .with_sink(NullSink::new())
            .build();
        assert_eq!(
            lenient.filter().default_level(),
            Level::Warn,
            "previous filter kept"
        );
    }

    #[test]
    fn test_try_filter_directive_fails_fast() {
        assert!(Logger::builder().try_filter_directive("nope").is_err());
        let builder = Logger::builder().try_filter_directive("debug").unwrap();
        let logger = builder.with_sink(NullSink::new()).build();
        assert_eq!(logger.filter().default_level(), Level::Debug);
    }

    #[test]
    fn test_blank_directive_changes_nothing() {
        let logger = Logger::builder()
            .level(Level::Warn)
            .filter_directive("   ")
            .with_sink(NullSink::new())
            .try_build()
            .unwrap();
        assert_eq!(logger.filter().default_level(), Level::Warn);
    }

    #[test]
    fn test_level_after_directive_keeps_rules() {
        let logger = Logger::builder()
            .filter_directive("warn,app=trace")
            .level(Level::Error)
            .with_sink(NullSink::new())
            .build();
        assert_eq!(logger.threshold_for("app"), Level::Trace);
        assert_eq!(logger.threshold_for("other"), Level::Error);
    }

    #[test]
    fn test_env_filter_values() {
        let base = || {
            Logger::builder()
                .level(Level::Warn)
                .with_sink(NullSink::new())
        };
        let unset = base().apply_env_value(None).try_build().unwrap();
        assert_eq!(unset.filter().default_level(), Level::Warn);
        let empty = base().apply_env_value(Some("".into())).try_build().unwrap();
        assert_eq!(empty.filter().default_level(), Level::Warn);
        let set = base()
            .apply_env_value(Some("debug,hyper=off".into()))
            .try_build()
            .unwrap();
        assert_eq!(set.filter().default_level(), Level::Debug);
        assert_eq!(set.threshold_for("hyper"), Level::Off);
        let bad = base().apply_env_value(Some("loud".into())).try_build();
        assert!(matches!(bad, Err(BuildError::InvalidFilter(_))));
        // A variable that is almost certainly unset.
        let real = base()
            .env_filter("LOG_IO_TEST_VARIABLE_THAT_IS_NOT_SET_7f3a")
            .try_build()
            .unwrap();
        assert_eq!(real.filter().default_level(), Level::Warn);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn test_env_filter_rejects_non_unicode() {
        #[cfg(unix)]
        let raw = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![0x66, 0xff, 0x6f])
        };
        #[cfg(windows)]
        let raw = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[0x0066, 0xd800, 0x006f])
        };
        let err = Logger::builder()
            .with_sink(NullSink::new())
            .apply_env_value(Some(raw))
            .try_build()
            .unwrap_err();
        let BuildError::InvalidFilter(inner) = err else {
            panic!("unexpected {err:?}");
        };
        assert_eq!(inner.kind(), ParseFilterErrorKind::NotUnicode);
    }

    #[test]
    fn test_default_fields_same_key_replaces() {
        let builder = Logger::builder()
            .with_default_field("service", "a")
            .with_default_field("service", "b");
        assert_eq!(builder.defaults.len(), 1);
        assert_eq!(builder.defaults[0].as_field().value, Value::Str("b"));
    }

    #[test]
    fn test_redact_keys_are_lowercased_and_deduplicated() {
        let builder = Logger::builder()
            .redact_key("Token")
            .redact_key("token")
            .redact_common_secrets();
        assert_eq!(
            builder.redact.len(),
            LoggerBuilder::COMMON_SECRET_KEYS.len(),
            "token appears once"
        );
        let text = format!("{builder:?}");
        assert!(text.contains("token"), "{text}");
    }

    #[test]
    fn test_shortcuts_install_sinks() {
        let logger = Logger::builder()
            .stdout_json()
            .stderr_json()
            .stdout_logfmt()
            .stderr_logfmt()
            .stdout_human()
            .stderr_human()
            .stdout(crate::format::JsonFormat::new())
            .writer(Vec::<u8>::new(), crate::format::JsonFormat::new())
            .build();
        assert_eq!(logger.sink_count(), 8);
    }
}

//! Crate-level error type.

use std::fmt;
use std::io;

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced by the logging pipeline.
///
/// Logging is best-effort by design: [`crate::Logger::log`] and the
/// macros never return these errors. They reach you through
/// [`crate::Logger::try_log`] / [`crate::Logger::try_emit`], through
/// [`crate::Logger::flush`] and [`crate::Logger::shutdown`], and
/// through the handler installed with
/// [`crate::LoggerBuilder::on_error`].
///
/// The enum is `#[non_exhaustive]`; match it with a wildcard arm.
///
/// # Examples
///
/// ```
/// use log_io::Error;
///
/// let err = Error::from(std::io::Error::other("disk full"));
/// assert_eq!(err.to_string(), "log sink I/O error: disk full");
/// assert!(std::error::Error::source(&err).is_some());
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The sink's destination returned an I/O error (disk full, broken
    /// pipe, permission denied, ...). Retrying may help for transient
    /// conditions; persistent failures usually need operator action.
    /// The record that triggered it was not written to that sink, but
    /// every other sink still received it.
    Io(io::Error),
    /// A formatter failed. With the built-in formats this only happens
    /// when a lazily formatted value (`Value::Display`, `Value::Debug`,
    /// `Value::Error`) or a message's format arguments return an error
    /// from their own `fmt` implementation; it indicates a bug in that
    /// implementation or in a custom [`crate::format::Format`].
    Format(fmt::Error),
    /// A custom sink or format rejected its configuration. Not produced
    /// by the built-in components; available to implementors.
    Configuration(&'static str),
    /// The sink has stopped accepting records: its background worker
    /// exited (for example after the writer panicked). Records sent to
    /// it are lost; rebuild the sink to recover.
    Closed,
    /// An error from a custom sink that does not fit the other
    /// variants. Construct with [`Error::custom`].
    Custom(Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl Error {
    /// Wrap an arbitrary error from a custom sink.
    ///
    /// # Examples
    ///
    /// ```
    /// let err = log_io::Error::custom(std::fmt::Error);
    /// assert!(matches!(err, log_io::Error::Custom(_)));
    /// ```
    pub fn custom<E>(error: E) -> Self
    where
        E: Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    {
        Self::Custom(error.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "log sink I/O error: {e}"),
            Self::Format(_) => f.write_str("log record formatting failed"),
            Self::Configuration(msg) => write!(f, "log sink configuration error: {msg}"),
            Self::Closed => f.write_str("log sink is closed and no longer accepts records"),
            Self::Custom(e) => write!(f, "log sink error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Format(e) => Some(e),
            Self::Custom(e) => Some(&**e),
            Self::Configuration(_) | Self::Closed => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<fmt::Error> for Error {
    fn from(value: fmt::Error) -> Self {
        Self::Format(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn test_error_display_has_context_for_every_variant() {
        let cases: [(Error, &str); 5] = [
            (
                io::Error::other("disk full").into(),
                "log sink I/O error: disk full",
            ),
            (fmt::Error.into(), "log record formatting failed"),
            (
                Error::Configuration("bad"),
                "log sink configuration error: bad",
            ),
            (
                Error::Closed,
                "log sink is closed and no longer accepts records",
            ),
            (Error::custom("boom"), "log sink error: boom"),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn test_error_source_chain() {
        assert!(Error::from(io::Error::other("x")).source().is_some());
        assert!(Error::from(fmt::Error).source().is_some());
        assert!(Error::custom("x").source().is_some());
        assert!(Error::Configuration("x").source().is_none());
        assert!(Error::Closed.source().is_none());
    }
}

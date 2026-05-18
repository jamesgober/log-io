//! Crate-level error type.

use std::fmt;
use std::io;

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors produced by the logging pipeline.
#[derive(Debug)]
pub enum Error {
    /// An IO error from a sink or writer.
    Io(io::Error),
    /// A formatter produced invalid output. This is rare and indicates
    /// a bug in a custom formatter.
    Format(fmt::Error),
    /// Configuration error during builder construction.
    Configuration(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Format(e) => write!(f, "format error: {e}"),
            Self::Configuration(msg) => write!(f, "configuration error: {msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Format(e) => Some(e),
            Self::Configuration(_) => None,
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

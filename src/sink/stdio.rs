//! Standard-stream sinks.

use std::io::{self, Write};

use super::{Sink, with_formatted};
use crate::error::{Error, Result};
use crate::format::Format;
use crate::record::Record;

/// Writes records to the process's standard output.
///
/// Each record is formatted first and then written with a single
/// `write_all` while holding the standard library's stdout lock, so a
/// record is never interleaved with other output that also goes
/// through `std::io::stdout()` (including `println!`). Rust's stdout is
/// line-buffered, so every complete record reaches the OS before
/// `write_record` returns.
///
/// # Examples
///
/// ```
/// use log_io::format::LogfmtFormat;
/// use log_io::sink::StdoutSink;
/// use log_io::{Level, Logger};
///
/// let logger = Logger::builder().with_sink(StdoutSink::new(LogfmtFormat::new())).build();
/// logger.log(Level::Info, "hello", &[]);
/// ```
pub struct StdoutSink<F: Format> {
    format: F,
}

impl<F: Format> StdoutSink<F> {
    /// New sink writing to standard output with `format`.
    pub const fn new(format: F) -> Self {
        Self { format }
    }
}

impl<F: Format> Sink for StdoutSink<F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        with_formatted(&self.format, record, |text| {
            io::stdout().lock().write_all(text.as_bytes())
        })
    }
    fn flush(&self) -> Result<()> {
        io::stdout().lock().flush().map_err(Error::from)
    }
}

impl<F: Format + core::fmt::Debug> core::fmt::Debug for StdoutSink<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StdoutSink")
            .field("format", &self.format)
            .finish()
    }
}

/// Writes records to the process's standard error.
///
/// Same locking behaviour as [`StdoutSink`]. Standard error is
/// unbuffered, so each record is handed to the OS immediately.
///
/// # Examples
///
/// ```
/// use log_io::format::HumanFormat;
/// use log_io::sink::StderrSink;
/// use log_io::{Level, Logger};
///
/// let logger = Logger::builder().with_sink(StderrSink::new(HumanFormat::new())).build();
/// logger.log(Level::Warn, "careful", &[]);
/// ```
pub struct StderrSink<F: Format> {
    format: F,
}

impl<F: Format> StderrSink<F> {
    /// New sink writing to standard error with `format`.
    pub const fn new(format: F) -> Self {
        Self { format }
    }
}

impl<F: Format> Sink for StderrSink<F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        with_formatted(&self.format, record, |text| {
            io::stderr().lock().write_all(text.as_bytes())
        })
    }
    fn flush(&self) -> Result<()> {
        io::stderr().lock().flush().map_err(Error::from)
    }
}

impl<F: Format + core::fmt::Debug> core::fmt::Debug for StderrSink<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StderrSink")
            .field("format", &self.format)
            .finish()
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::format::JsonFormat;
    use crate::{Level, Metadata};

    #[test]
    fn test_stdio_sinks_write_and_flush() {
        let record = Record::new(Metadata::new(Level::Info, "stdio-test"), "smoke", &[]);
        let out = StdoutSink::new(JsonFormat::new());
        out.write_record(&record).unwrap();
        out.flush().unwrap();
        out.shutdown().unwrap();
        let err = StderrSink::new(JsonFormat::new());
        err.write_record(&record).unwrap();
        err.flush().unwrap();
        assert!(format!("{out:?}").starts_with("StdoutSink"));
        assert!(format!("{err:?}").starts_with("StderrSink"));
    }
}

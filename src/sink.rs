//! Output sinks.
//!
//! A sink consumes a serialized record. The crate's built-in sinks
//! wrap a [`std::io::Write`] target with a mutex so they can be shared
//! across threads. Custom sinks implement [`Sink`] directly.

use core::fmt;
use std::io::Write;
use std::sync::Mutex;

use crate::error::{Error, Result};
use crate::format::Format;
use crate::record::Record;

/// Output stage of the pipeline.
///
/// A sink is responsible for taking a [`Record`], serializing it with
/// its configured [`Format`], and writing the bytes to a destination.
/// Implementors must be safe to call from many threads concurrently.
pub trait Sink: Send + Sync {
    /// Write the record. Returns whatever IO error the destination
    /// produced.
    fn write_record(&self, record: &Record<'_>) -> Result<()>;

    /// Flush any buffered bytes.
    fn flush(&self) -> Result<()>;
}

impl<S: Sink + ?Sized> Sink for std::sync::Arc<S> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        (**self).write_record(record)
    }
    fn flush(&self) -> Result<()> {
        (**self).flush()
    }
}

impl<S: Sink + ?Sized> Sink for Box<S> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        (**self).write_record(record)
    }
    fn flush(&self) -> Result<()> {
        (**self).flush()
    }
}

mod stdio;
mod writer;

pub use self::stdio::{StderrSink, StdoutSink};
pub use self::writer::{FileSink, NullSink, WriterSink};

/// Helper that serializes a record with `format` into a heap-allocated
/// `String`, returning the formatted bytes ready for IO. Most sinks
/// route through this so a single allocation per record is shared
/// across stages.
pub(crate) fn format_to_string<F: Format>(format: &F, record: &Record<'_>) -> Result<String> {
    let mut buf = String::with_capacity(128);
    format
        .write_record(record, &mut buf)
        .map_err(|e: fmt::Error| Error::Format(e))?;
    Ok(buf)
}

/// Write a record to any [`std::io::Write`] under a mutex. Used
/// internally by [`WriterSink`], [`FileSink`], [`StdoutSink`], and
/// [`StderrSink`].
pub(crate) fn write_locked<W: Write, F: Format>(
    target: &Mutex<W>,
    format: &F,
    record: &Record<'_>,
) -> Result<()> {
    let bytes = format_to_string(format, record)?;
    let mut guard = target
        .lock()
        .map_err(|_| Error::Configuration("sink mutex poisoned"))?;
    guard.write_all(bytes.as_bytes()).map_err(Error::from)
}

/// Flush any [`std::io::Write`] under a mutex.
pub(crate) fn flush_locked<W: Write>(target: &Mutex<W>) -> Result<()> {
    let mut guard = target
        .lock()
        .map_err(|_| Error::Configuration("sink mutex poisoned"))?;
    guard.flush().map_err(Error::from)
}

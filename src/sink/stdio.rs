//! Standard-stream sinks.

use std::io::{self, Stderr, Stdout};
use std::sync::Mutex;

use super::{flush_locked, write_locked, Sink};
use crate::error::Result;
use crate::format::Format;
use crate::record::Record;

/// Write records to the process's standard output.
///
/// Writes are serialized through a per-sink mutex. Stdout itself has a
/// global lock, but we use our own so the serialized record is written
/// atomically without interleaving with other stdout users.
pub struct StdoutSink<F: Format> {
    format: F,
    target: Mutex<Stdout>,
}

impl<F: Format> StdoutSink<F> {
    /// New sink writing to `io::stdout()` with `format`.
    pub fn new(format: F) -> Self {
        Self {
            format,
            target: Mutex::new(io::stdout()),
        }
    }
}

impl<F: Format> Sink for StdoutSink<F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        write_locked(&self.target, &self.format, record)
    }
    fn flush(&self) -> Result<()> {
        flush_locked(&self.target)
    }
}

impl<F: Format + core::fmt::Debug> core::fmt::Debug for StdoutSink<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StdoutSink")
            .field("format", &self.format)
            .finish()
    }
}

/// Write records to the process's standard error.
pub struct StderrSink<F: Format> {
    format: F,
    target: Mutex<Stderr>,
}

impl<F: Format> StderrSink<F> {
    /// New sink writing to `io::stderr()` with `format`.
    pub fn new(format: F) -> Self {
        Self {
            format,
            target: Mutex::new(io::stderr()),
        }
    }
}

impl<F: Format> Sink for StderrSink<F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        write_locked(&self.target, &self.format, record)
    }
    fn flush(&self) -> Result<()> {
        flush_locked(&self.target)
    }
}

impl<F: Format + core::fmt::Debug> core::fmt::Debug for StderrSink<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StderrSink")
            .field("format", &self.format)
            .finish()
    }
}

//! Generic writer-backed sinks.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use super::{flush_locked, write_locked, Sink};
use crate::error::Result;
use crate::format::Format;
use crate::record::Record;

/// Adapter that turns any `Send + Write` value into a [`Sink`].
///
/// Useful for piping records to in-memory buffers, sockets, or other
/// non-file destinations. Wraps the writer in a mutex so the sink is
/// safe to share across threads.
pub struct WriterSink<W: Write + Send, F: Format> {
    format: F,
    target: Mutex<W>,
}

impl<W: Write + Send, F: Format> WriterSink<W, F> {
    /// Wrap `writer` in a sink using `format`.
    pub fn new(writer: W, format: F) -> Self {
        Self {
            format,
            target: Mutex::new(writer),
        }
    }
}

impl<W: Write + Send, F: Format> Sink for WriterSink<W, F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        write_locked(&self.target, &self.format, record)
    }
    fn flush(&self) -> Result<()> {
        flush_locked(&self.target)
    }
}

impl<W: Write + Send, F: Format + core::fmt::Debug> core::fmt::Debug for WriterSink<W, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WriterSink")
            .field("format", &self.format)
            .finish()
    }
}

/// Buffered file sink.
///
/// Wraps a [`std::fs::File`] in a [`BufWriter`] under a mutex.
/// [`Sink::flush`] flushes the buffer through to the OS; it does not
/// `fsync` the underlying file. Use the file directly if a stricter
/// guarantee is needed.
pub struct FileSink<F: Format> {
    format: F,
    target: Mutex<BufWriter<File>>,
}

impl<F: Format> FileSink<F> {
    /// Open `path` for append and wrap it as a sink. Creates the file
    /// if it does not exist.
    pub fn append<P: AsRef<Path>>(path: P, format: F) -> Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            format,
            target: Mutex::new(BufWriter::new(file)),
        })
    }

    /// Open `path` for truncating write and wrap it as a sink.
    pub fn create<P: AsRef<Path>>(path: P, format: F) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        Ok(Self {
            format,
            target: Mutex::new(BufWriter::new(file)),
        })
    }
}

impl<F: Format> Sink for FileSink<F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        write_locked(&self.target, &self.format, record)
    }
    fn flush(&self) -> Result<()> {
        flush_locked(&self.target)
    }
}

impl<F: Format + core::fmt::Debug> core::fmt::Debug for FileSink<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileSink")
            .field("format", &self.format)
            .finish()
    }
}

/// Sink that discards every record. Useful in benchmarks and tests.
#[derive(Debug, Default)]
pub struct NullSink;

impl NullSink {
    /// New null sink.
    pub const fn new() -> Self {
        Self
    }
}

impl Sink for NullSink {
    fn write_record(&self, _record: &Record<'_>) -> Result<()> {
        Ok(())
    }
    fn flush(&self) -> Result<()> {
        Ok(())
    }
}

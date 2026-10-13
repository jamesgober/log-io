//! Generic writer-backed sinks.

use std::io::Write;
use std::sync::Mutex;

use super::{CachePadded, Sink, flush_locked, lock_recover, write_locked};
use crate::error::Result;
use crate::format::Format;
use crate::record::Record;

/// Adapter that turns any `Write + Send` value into a [`Sink`].
///
/// The writer is wrapped in a mutex so the sink can be shared across
/// threads; each record is formatted outside the lock and written with
/// one `write_all` under it. Use it for in-memory buffers, sockets,
/// pipes, or a [`super::RotatingFile`].
///
/// # Examples
///
/// ```
/// use log_io::format::LogfmtFormat;
/// use log_io::sink::WriterSink;
/// use log_io::{Level, Logger};
///
/// let sink = WriterSink::new(Vec::<u8>::new(), LogfmtFormat::new());
/// let logger = Logger::builder().no_timestamps().with_sink(sink).build();
/// logger.log(Level::Info, "to memory", &[]);
/// ```
pub struct WriterSink<W: Write + Send, F: Format> {
    format: F,
    target: CachePadded<Mutex<W>>,
}

impl<W: Write + Send, F: Format> WriterSink<W, F> {
    /// Wrap `writer` in a sink using `format`.
    pub const fn new(writer: W, format: F) -> Self {
        Self {
            format,
            target: CachePadded(Mutex::new(writer)),
        }
    }

    /// Run `f` with exclusive access to the writer, for example to
    /// inspect an in-memory buffer or rotate a file by hand.
    ///
    /// # Examples
    ///
    /// ```
    /// use log_io::format::LogfmtFormat;
    /// use log_io::sink::WriterSink;
    /// use log_io::{Level, Metadata, Record, Sink};
    ///
    /// let sink = WriterSink::new(Vec::<u8>::new(), LogfmtFormat::new());
    /// sink.write_record(&Record::new(Metadata::new(Level::Info, "t"), "m", &[]))?;
    /// let len = sink.with_writer(|buf| buf.len());
    /// assert!(len > 0);
    /// # Ok::<(), log_io::Error>(())
    /// ```
    pub fn with_writer<R>(&self, f: impl FnOnce(&mut W) -> R) -> R {
        f(&mut lock_recover(&self.target))
    }

    /// Consume the sink and return the writer.
    pub fn into_inner(self) -> W {
        self.target
            .0
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            .finish_non_exhaustive()
    }
}

/// Sink that discards every record without formatting it. Useful in
/// benchmarks and tests.
///
/// # Examples
///
/// ```
/// use log_io::sink::NullSink;
/// use log_io::{Level, Logger};
///
/// let logger = Logger::builder().with_sink(NullSink::new()).build();
/// logger.log(Level::Error, "dropped", &[]);
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct NullSink;

impl NullSink {
    /// New null sink.
    #[must_use]
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

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::format::JsonFormat;
    use crate::{Level, Metadata};

    #[test]
    fn test_writer_sink_round_trip_and_into_inner() {
        let sink = WriterSink::new(Vec::<u8>::new(), JsonFormat::new());
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &[]);
        sink.write_record(&record).unwrap();
        sink.flush().unwrap();
        sink.shutdown().unwrap();
        assert!(format!("{sink:?}").starts_with("WriterSink"));
        let bytes = sink.into_inner();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "{\"level\":\"info\",\"target\":\"t\",\"message\":\"m\"}\n"
        );
    }

    #[test]
    fn test_writer_sink_recovers_from_poisoned_lock() {
        // A writer that panics once while the sink's mutex is held,
        // poisoning it; later writes must still succeed. This is the
        // audit's R13 approach rather than a test that poisons nothing.
        struct PanicOnce {
            panicked: bool,
            out: Vec<u8>,
        }
        impl Write for PanicOnce {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                if !self.panicked {
                    self.panicked = true;
                    panic!("writer panic while holding the sink lock");
                }
                self.out.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let sink = WriterSink::new(
            PanicOnce {
                panicked: false,
                out: Vec::new(),
            },
            JsonFormat::new(),
        );
        let record = Record::new(Metadata::new(Level::Info, "t"), "second", &[]);
        let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ignored = sink.write_record(&record);
        }));
        assert!(first.is_err());
        assert!(
            sink.target.is_poisoned(),
            "the lock must really be poisoned"
        );
        sink.write_record(&record).unwrap();
        sink.flush().unwrap();
        let out = sink.with_writer(|w| String::from_utf8(w.out.clone()).unwrap());
        assert!(out.contains("second"), "{out}");
    }

    #[test]
    fn test_null_sink_accepts_everything() {
        let sink = NullSink::new();
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &[]);
        sink.write_record(&record).unwrap();
        sink.flush().unwrap();
        sink.shutdown().unwrap();
    }
}

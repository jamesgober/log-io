//! Output sinks.
//!
//! A sink takes a [`Record`], serializes it with its [`Format`] and
//! delivers the bytes somewhere. The built-in sinks are:
//!
//! | Sink | Destination | Notes |
//! |---|---|---|
//! | [`StdoutSink`], [`StderrSink`] | standard streams | written under the stream's own lock |
//! | [`FileSink`] | a file | buffered; see [`FileSink`] for durability |
//! | [`WriterSink`] | any `io::Write` | e.g. a [`RotatingFile`] ([`RotatingFileSink`]) |
//! | [`NonBlocking`] | any `io::Write`, on a background thread | bounded queue, explicit overflow policy |
//! | [`NullSink`] | nowhere | benchmarks and tests |
//!
//! # Threading
//!
//! Every sink is `Send + Sync`. The synchronous sinks serialize writers
//! with a mutex and perform the write on the calling thread, so in an
//! async runtime they block the executor thread for the duration of
//! the write. Use [`NonBlocking`] to move I/O off the calling thread.
//!
//! # Allocation
//!
//! Records are formatted into a reusable thread-local buffer, so the
//! synchronous sinks do not allocate in steady state. A buffer that
//! grew past 64 KiB for an unusually large record is shrunk back to
//! 64 KiB afterwards, so one huge record does not pin memory on every
//! thread for the life of the process.
//!
//! # Reliability
//!
//! The built-in sinks recover from mutex poisoning: if a thread panics
//! while holding a sink's lock (for example inside a custom writer),
//! later records are still written. Logging stays available during
//! incident response.

use core::cell::RefCell;
use std::io::{self, Write};
use std::sync::{Mutex, MutexGuard};

use crate::error::{Error, Result};
use crate::format::Format;
use crate::record::Record;

/// Output stage of the pipeline.
///
/// Implementors must be safe to call from many threads concurrently.
/// A sink should never panic on bad input; return an error instead so
/// the logger can report it and keep feeding the other sinks.
///
/// # Examples
///
/// ```
/// use std::sync::Mutex;
/// use log_io::format::{Format, JsonFormat};
/// use log_io::{Level, Logger, Record, Result, Sink};
///
/// /// Collects formatted records in memory.
/// #[derive(Default)]
/// struct Memory(Mutex<Vec<String>>);
///
/// impl Sink for Memory {
///     fn write_record(&self, record: &Record<'_>) -> Result<()> {
///         let mut line = String::new();
///         JsonFormat::new().write_record(record, &mut line)?;
///         self.0.lock().unwrap_or_else(|p| p.into_inner()).push(line);
///         Ok(())
///     }
///     fn flush(&self) -> Result<()> {
///         Ok(())
///     }
/// }
///
/// let sink = std::sync::Arc::new(Memory::default());
/// let logger = Logger::builder().no_timestamps().with_sink(sink.clone()).build();
/// logger.log(Level::Info, "hello", &[]);
/// assert_eq!(sink.0.lock().unwrap().len(), 1);
/// ```
pub trait Sink: Send + Sync {
    /// Serialize and deliver one record.
    ///
    /// # Errors
    ///
    /// Returns the destination's I/O error, a format error, or a
    /// sink-specific error.
    fn write_record(&self, record: &Record<'_>) -> Result<()>;

    /// Push any buffered bytes to the destination (for files, to the
    /// operating system; see the sink's own docs for `fsync`).
    ///
    /// # Errors
    ///
    /// Returns the destination's I/O error, or an error deferred from
    /// an earlier background operation.
    fn flush(&self) -> Result<()>;

    /// Flush and release background resources. Called by
    /// [`crate::Logger::shutdown`], by a [`crate::FlushGuard`] on drop,
    /// and when the last clone of a [`crate::Logger`] is dropped.
    ///
    /// A sink must keep accepting records after `shutdown` (writing
    /// them synchronously if its background machinery is gone): late
    /// records from other threads must not be lost. The default
    /// implementation calls [`Sink::flush`].
    ///
    /// # Errors
    ///
    /// Same as [`Sink::flush`].
    fn shutdown(&self) -> Result<()> {
        self.flush()
    }
}

impl<S: Sink + ?Sized> Sink for std::sync::Arc<S> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        (**self).write_record(record)
    }
    fn flush(&self) -> Result<()> {
        (**self).flush()
    }
    fn shutdown(&self) -> Result<()> {
        (**self).shutdown()
    }
}

impl<S: Sink + ?Sized> Sink for Box<S> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        (**self).write_record(record)
    }
    fn flush(&self) -> Result<()> {
        (**self).flush()
    }
    fn shutdown(&self) -> Result<()> {
        (**self).shutdown()
    }
}

mod file;
mod non_blocking;
mod rotating;
mod stdio;
mod writer;

pub use self::file::{FileSink, FileSinkBuilder};
pub use self::non_blocking::{NonBlocking, NonBlockingBuilder, Overflow};
pub use self::rotating::{RotatingFile, RotatingFileBuilder, Rotation};
pub use self::stdio::{StderrSink, StdoutSink};
pub use self::writer::{NullSink, WriterSink};

/// A [`WriterSink`] over a [`RotatingFile`].
///
/// # Examples
///
/// ```no_run
/// use log_io::format::JsonFormat;
/// use log_io::sink::{RotatingFile, RotatingFileSink, Rotation};
///
/// let file = RotatingFile::builder("logs/app.jsonl")
///     .max_bytes(10 * 1024 * 1024)
///     .rotation(Rotation::Daily)
///     .max_files(7)
///     .open()?;
/// let sink: RotatingFileSink<JsonFormat> = RotatingFileSink::new(file, JsonFormat::new());
/// # Ok::<(), std::io::Error>(())
/// ```
pub type RotatingFileSink<F> = WriterSink<RotatingFile, F>;

/// Keeps a sink's lock on its own cache lines.
///
/// Every logging thread reads the logger's sink table on each record,
/// and every thread writes the sink's mutex. If a small sink allocation
/// shares a cache line with other hot data, each lock and unlock
/// invalidates that line for all threads. Measured on the reference
/// machine, padding restored the 4-thread throughput of 1.0.
#[repr(align(128))]
pub(crate) struct CachePadded<T>(pub(crate) T);

impl<T> core::ops::Deref for CachePadded<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// Capacity a thread-local format buffer may keep between records.
pub(crate) const MAX_RETAINED_CAPACITY: usize = 64 * 1024;

thread_local! {
    /// Reusable formatting buffer, so steady-state formatting does not
    /// touch the allocator.
    static FORMAT_BUF: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Shrink `buf` back to [`MAX_RETAINED_CAPACITY`] after an oversized
/// record. Cheap when no shrinking is needed: one comparison.
#[inline]
pub(crate) fn trim_buffer(buf: &mut String) {
    if buf.capacity() > MAX_RETAINED_CAPACITY {
        buf.clear();
        buf.shrink_to(MAX_RETAINED_CAPACITY);
    }
}

/// Format `record` into the thread-local buffer and pass the text to
/// `write`.
///
/// If the buffer is already in use on this thread (a sink that logs
/// from inside its own write) or thread-local storage is being torn
/// down, a temporary buffer is used instead.
pub(crate) fn with_formatted<F, W>(format: &F, record: &Record<'_>, mut write: W) -> Result<()>
where
    F: Format,
    W: FnMut(&str) -> io::Result<()>,
{
    let pooled = FORMAT_BUF.try_with(|cell| {
        let mut buf = cell.try_borrow_mut().ok()?;
        Some(format_into(&mut buf, format, record, &mut write))
    });
    match pooled {
        Ok(Some(result)) => result,
        _ => format_into(&mut String::with_capacity(256), format, record, &mut write),
    }
}

fn format_into<F, W>(buf: &mut String, format: &F, record: &Record<'_>, write: &mut W) -> Result<()>
where
    F: Format,
    W: FnMut(&str) -> io::Result<()>,
{
    buf.clear();
    let result = format
        .write_record_generic(record, buf)
        .map_err(Error::Format)
        .and_then(|()| write(buf).map_err(Error::Io));
    trim_buffer(buf);
    result
}

/// Acquire a mutex guard even if the mutex is poisoned.
///
/// Poisoning only means a previous holder unwound; the writer inside is
/// still usable, and logging must keep working after a panic.
pub(crate) fn lock_recover<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Format `record` and write it to the writer behind `target`.
pub(crate) fn write_locked<W: Write + ?Sized, F: Format>(
    target: &Mutex<W>,
    format: &F,
    record: &Record<'_>,
) -> Result<()> {
    with_formatted(format, record, |text| {
        lock_recover(target).write_all(text.as_bytes())
    })
}

/// Flush the writer behind `target`.
pub(crate) fn flush_locked<W: Write + ?Sized>(target: &Mutex<W>) -> Result<()> {
    lock_recover(target).flush().map_err(Error::from)
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::format::JsonFormat;
    use crate::{Level, Metadata};

    #[test]
    fn test_trim_buffer_caps_retained_capacity() {
        let mut buf = String::with_capacity(4 * MAX_RETAINED_CAPACITY);
        buf.push('x');
        trim_buffer(&mut buf);
        assert!(buf.capacity() <= MAX_RETAINED_CAPACITY);
        let mut small = String::with_capacity(128);
        small.push('y');
        trim_buffer(&mut small);
        assert_eq!(small, "y", "small buffers are left alone");
    }

    #[test]
    fn test_thread_local_buffer_shrinks_after_oversize_record() {
        // The audit's LOG-007 reproduction, scaled down: one 1 MiB
        // record must not leave a 1 MiB+ buffer behind.
        std::thread::spawn(|| {
            let big = "x".repeat(1024 * 1024);
            let record = Record::new(Metadata::new(Level::Info, "t"), &big, &[]);
            let mut written = 0;
            with_formatted(&JsonFormat::new(), &record, |s| {
                written = s.len();
                Ok(())
            })
            .unwrap();
            assert!(written > big.len());
            let cap = FORMAT_BUF.with(|b| b.borrow().capacity());
            assert!(cap <= MAX_RETAINED_CAPACITY, "retained {cap} bytes");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn test_with_formatted_reentrant_call_uses_temporary_buffer() {
        let record = Record::new(Metadata::new(Level::Info, "t"), "outer", &[]);
        let mut inner_text = String::new();
        with_formatted(&JsonFormat::new(), &record, |outer| {
            let inner = Record::new(Metadata::new(Level::Info, "t"), "inner", &[]);
            with_formatted(&JsonFormat::new(), &inner, |s| {
                inner_text = s.to_owned();
                Ok(())
            })
            .map_err(|e| io::Error::other(e.to_string()))?;
            assert!(outer.contains("outer"));
            Ok(())
        })
        .unwrap();
        assert!(inner_text.contains("inner"));
    }

    #[test]
    fn test_with_formatted_surfaces_write_and_format_errors() {
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &[]);
        let err = with_formatted(&JsonFormat::new(), &record, |_| Err(io::Error::other("x")))
            .unwrap_err();
        assert!(matches!(err, Error::Io(_)));

        struct Broken;
        impl core::fmt::Display for Broken {
            fn fmt(&self, _: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                Err(core::fmt::Error)
            }
        }
        let fields = [crate::Field::new("b", crate::Value::Display(&Broken))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let err = with_formatted(&JsonFormat::new(), &record, |_| Ok(())).unwrap_err();
        assert!(matches!(err, Error::Format(_)));
    }
}

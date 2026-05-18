//! Output sinks.
//!
//! A sink consumes a serialized record. The crate's built-in sinks
//! wrap a [`std::io::Write`] target with a mutex so they can be shared
//! across threads. Custom sinks implement [`Sink`] directly.
//!
//! # Reliability
//!
//! Built-in sinks recover from mutex poisoning. If a thread panics
//! while holding the sink's write lock, subsequent log calls still
//! make progress instead of returning errors forever; this matches
//! the expectation that logging should remain available during
//! incident response.

use core::fmt;
use std::io::Write;
use std::sync::{Mutex, MutexGuard};

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
    ///
    /// # Errors
    ///
    /// Returns the destination's IO error, or a format error from a
    /// custom formatter.
    fn write_record(&self, record: &Record<'_>) -> Result<()>;

    /// Flush any buffered bytes.
    ///
    /// # Errors
    ///
    /// Returns the destination's IO error.
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

thread_local! {
    /// Reusable formatting buffer. The fast path avoids per-record
    /// allocation by handing the formatter a buffer that lives for
    /// the lifetime of the thread.
    static FORMAT_BUF: core::cell::RefCell<String> = const {
        core::cell::RefCell::new(String::new())
    };
}

/// Acquire a mutex guard even if the mutex is poisoned.
///
/// Logging should remain available across panics that briefly held a
/// sink lock; poisoning just means "a previous holder unwound", and
/// the inner writer is still valid for our purposes.
fn lock_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Write a record to any [`std::io::Write`] under a mutex.
///
/// Uses a thread-local scratch buffer so the format step is
/// allocation-free in steady state. If the buffer is already in use
/// on this thread (rare: caused by a sink that re-entered logging
/// from within `write_record`), falls back to a stack-local buffer.
pub(crate) fn write_locked<W: Write, F: Format>(
    target: &Mutex<W>,
    format: &F,
    record: &Record<'_>,
) -> Result<()> {
    FORMAT_BUF.with(|cell| {
        if let Ok(mut buf) = cell.try_borrow_mut() {
            buf.clear();
            format
                .write_record(record, &mut *buf)
                .map_err(|e: fmt::Error| Error::Format(e))?;
            let mut guard = lock_recover(target);
            guard.write_all(buf.as_bytes()).map_err(Error::from)
        } else {
            // Reentrant path: another sink on this thread is already
            // using the buffer. Allocate a fresh one.
            let mut buf = String::with_capacity(128);
            format
                .write_record(record, &mut buf)
                .map_err(|e: fmt::Error| Error::Format(e))?;
            let mut guard = lock_recover(target);
            guard.write_all(buf.as_bytes()).map_err(Error::from)
        }
    })
}

/// Flush any [`std::io::Write`] under a mutex.
pub(crate) fn flush_locked<W: Write>(target: &Mutex<W>) -> Result<()> {
    let mut guard = lock_recover(target);
    guard.flush().map_err(Error::from)
}

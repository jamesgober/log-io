//! The process-wide logger.
//!
//! Most programs build one [`Logger`] at startup and use it everywhere.
//! [`init`] installs it globally and returns a [`FlushGuard`]; the
//! logging macros use the global logger when called without a logger
//! argument (`log_io::info!("ready")`).
//!
//! The global logger lives in a `static`, and Rust never drops statics.
//! Keep the [`FlushGuard`] alive for the whole of `main` so buffered
//! records are flushed (and background writers drained) when `main`
//! returns:
//!
//! ```no_run
//! use log_io::{Level, Logger};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let logger = Logger::builder().level(Level::Info).stdout_json().build();
//!     let _guard = log_io::init(logger)?;
//!
//!     log_io::info!("service started", port = 8080_u16);
//!     Ok(())
//! } // `_guard` drops here: every sink is flushed and shut down.
//! ```
//!
//! Exit paths that skip destructors (`std::process::exit`, `abort`,
//! fatal signals) also skip the guard: call [`shutdown`] before
//! `process::exit`, and see [`install_panic_hook`] for
//! `panic = "abort"` builds.

use std::sync::OnceLock;

use crate::error::Result;
use crate::level::Level;
use crate::logger::{Logger, in_dispatch};
use crate::record::{Field, Metadata};

static GLOBAL: OnceLock<Logger> = OnceLock::new();

/// Install `logger` as the process-wide logger.
///
/// # Errors
///
/// Returns [`SetGlobalError`] (which hands the logger back) if a global
/// logger is already installed. The global logger can be set once.
///
/// # Examples
///
/// ```
/// use log_io::{Logger, sink::NullSink};
///
/// let first = Logger::builder().with_sink(NullSink::new()).build();
/// let second = first.clone();
/// let _ = log_io::set_global(first);
/// assert!(log_io::set_global(second).is_err());
/// ```
pub fn set(logger: Logger) -> core::result::Result<(), SetGlobalError> {
    GLOBAL.set(logger).map_err(SetGlobalError)
}

/// Install `logger` globally and return a guard that flushes and shuts
/// down its sinks when dropped. Keep the guard alive in `main`.
///
/// # Errors
///
/// Returns [`SetGlobalError`] if a global logger is already installed.
pub fn init(logger: Logger) -> core::result::Result<FlushGuard, SetGlobalError> {
    let guard = FlushGuard::new(logger.clone());
    set(logger)?;
    Ok(guard)
}

/// The process-wide logger, if one has been installed.
#[must_use]
pub fn get() -> Option<&'static Logger> {
    GLOBAL.get()
}

/// Flush the global logger's sinks. Does nothing if none is installed.
///
/// # Errors
///
/// Returns the first sink error (see [`Logger::flush`]).
pub fn flush() -> Result<()> {
    GLOBAL.get().map_or(Ok(()), Logger::flush)
}

/// Shut down the global logger's sinks: flush, drain background queues
/// and join their threads. Call it before `std::process::exit`. The
/// logger keeps working afterwards (background sinks write
/// synchronously). Does nothing if no global logger is installed.
///
/// # Errors
///
/// Returns the first sink error (see [`Logger::shutdown`]).
pub fn shutdown() -> Result<()> {
    GLOBAL.get().map_or(Ok(()), Logger::shutdown)
}

/// Install a panic hook that logs every panic through the global logger
/// and flushes it, then runs the previously installed hook.
///
/// The record is emitted at [`Level::Error`] with target `panic`, the
/// panic message as its message, and `file`, `line`, `column` and
/// `thread` fields. Because the hook runs before unwinding (or before
/// the process aborts under `panic = "abort"`), this preserves the
/// final records that would otherwise sit in a buffer.
///
/// If the panic happens inside the logging pipeline itself (in a sink or
/// format on the panicking thread), the hook does not touch the logger,
/// because the panicking frame may hold a sink's lock. Install it once,
/// after [`init`]; installing it twice logs each panic twice.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !in_dispatch() {
            if let Some(logger) = GLOBAL.get() {
                log_panic(logger, info);
            }
        }
        previous(info);
    }));
}

fn log_panic(logger: &Logger, info: &std::panic::PanicHookInfo<'_>) {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("panic with a non-string payload");
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>");
    let (file, line, column) = info
        .location()
        .map_or(("<unknown>", 0, 0), |l| (l.file(), l.line(), l.column()));
    let fields = [
        Field::new("file", file),
        Field::new("line", line),
        Field::new("column", column),
        Field::new("thread", thread_name),
    ];
    let metadata = Metadata::new(Level::Error, "panic");
    // Errors were already passed to the logger's error handler.
    let _reported = logger.try_emit(metadata, message, &fields);
    let _flushed = logger.flush();
}

/// Flushes and shuts down a logger's sinks when dropped.
///
/// Returned by [`init`] and [`Logger::flush_guard`]. Bind it to a named
/// variable (`let _guard = ...`), not `_`, which would drop it at once.
/// Errors during the final shutdown go to the logger's error handler.
#[must_use = "the guard flushes the logger when dropped; bind it to a named variable"]
#[derive(Debug)]
pub struct FlushGuard {
    logger: Logger,
}

impl FlushGuard {
    pub(crate) const fn new(logger: Logger) -> Self {
        Self { logger }
    }

    /// The logger this guard flushes.
    #[must_use]
    pub fn logger(&self) -> &Logger {
        &self.logger
    }
}

impl Drop for FlushGuard {
    fn drop(&mut self) {
        // Every failure was already passed to the error handler.
        let _reported = self.logger.shutdown();
    }
}

/// Error returned when a global logger is already installed. Holds the
/// logger that was not installed.
pub struct SetGlobalError(Logger);

impl SetGlobalError {
    /// Recover the logger that was not installed.
    #[must_use]
    pub fn into_inner(self) -> Logger {
        self.0
    }
}

impl core::fmt::Debug for SetGlobalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SetGlobalError(..)")
    }
}

impl core::fmt::Display for SetGlobalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a global logger is already installed")
    }
}

impl std::error::Error for SetGlobalError {}

//! Convenience macros.
//!
//! Each macro takes a [`crate::Logger`] expression and produces a log
//! call. Fields use a `key = expr` syntax; the expression must produce
//! a value convertible to [`crate::Value`].
//!
//! # Example
//!
//! ```
//! use log_io::{Field, Level, Logger};
//!
//! let logger = Logger::builder()
//!     .level(Level::Info)
//!     .no_timestamps()
//!     .no_context()
//!     .null()
//!     .json()
//!     .build();
//!
//! log_io::info!(logger, "request handled", port = 8080_u32, ok = true);
//! ```

/// Emit a log record at the given level.
///
/// The first argument is a `Logger` (by reference or by value). The
/// second is the message string. Any number of `key = expr` pairs may
/// follow.
#[macro_export]
macro_rules! log_at {
    ($logger:expr, $level:expr, $msg:expr $(,)?) => {{
        let __logger = &$logger;
        if __logger.enabled(module_path!(), $level) {
            let __fields: [$crate::Field<'_>; 0] = [];
            let _ = __logger.try_log_with_target(
                $level,
                module_path!(),
                $msg,
                &__fields,
            );
        }
    }};
    ($logger:expr, $level:expr, $msg:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let __logger = &$logger;
        if __logger.enabled(module_path!(), $level) {
            let __fields = [
                $(
                    $crate::Field::new(stringify!($key), $crate::Value::from($value)),
                )+
            ];
            let _ = __logger.try_log_with_target(
                $level,
                module_path!(),
                $msg,
                &__fields,
            );
        }
    }};
}

/// Emit a [`crate::Level::Trace`] record.
#[macro_export]
macro_rules! trace {
    ($logger:expr, $($rest:tt)*) => {
        $crate::log_at!($logger, $crate::Level::Trace, $($rest)*)
    };
}

/// Emit a [`crate::Level::Debug`] record.
#[macro_export]
macro_rules! debug {
    ($logger:expr, $($rest:tt)*) => {
        $crate::log_at!($logger, $crate::Level::Debug, $($rest)*)
    };
}

/// Emit a [`crate::Level::Info`] record.
#[macro_export]
macro_rules! info {
    ($logger:expr, $($rest:tt)*) => {
        $crate::log_at!($logger, $crate::Level::Info, $($rest)*)
    };
}

/// Emit a [`crate::Level::Warn`] record.
#[macro_export]
macro_rules! warn_ {
    ($logger:expr, $($rest:tt)*) => {
        $crate::log_at!($logger, $crate::Level::Warn, $($rest)*)
    };
}

// Workaround: `warn` collides with the attribute, so export both.
#[macro_export]
#[doc(hidden)]
macro_rules! __log_io_warn {
    ($logger:expr, $($rest:tt)*) => {
        $crate::log_at!($logger, $crate::Level::Warn, $($rest)*)
    };
}

/// Emit a [`crate::Level::Error`] record.
#[macro_export]
macro_rules! error {
    ($logger:expr, $($rest:tt)*) => {
        $crate::log_at!($logger, $crate::Level::Error, $($rest)*)
    };
}

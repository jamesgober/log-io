//! Convenience macros.
//!
//! Each macro takes a [`crate::Logger`] expression and produces a log
//! call. Fields use a `key = expr` syntax; the expression must produce
//! a value convertible to [`crate::Value`].
//!
//! All macros capture the caller's source location ([`file!`],
//! [`line!`]) and module path ([`module_path!`]) and attach them to
//! the record's [`crate::Metadata`].
//!
//! # Example
//!
//! ```
//! use log_io::{Level, Logger};
//!
//! let logger = Logger::builder()
//!     .level(Level::Info)
//!     .no_timestamps()
//!     .no_context()
//!     .with_sink(log_io::sink::NullSink::new())
//!     .build();
//!
//! log_io::info!(logger, "request handled", port = 8080_u32, ok = true);
//! ```

/// Emit a log record at the given level.
///
/// The first argument is a `Logger` (by reference or by value). The
/// second is the level. The third is the message string. Any number
/// of `key = expr` pairs may follow.
#[macro_export]
macro_rules! log_at {
    ($logger:expr, $level:expr, $msg:expr $(,)?) => {{
        let __logger = &$logger;
        let __level: $crate::Level = $level;
        if __logger.enabled(::core::module_path!(), __level) {
            let __fields: [$crate::Field<'_>; 0] = [];
            let __metadata = $crate::Metadata::new(__level, ::core::module_path!())
                .with_location(::core::file!(), ::core::line!());
            let _ = __logger.try_emit(__metadata, $msg, &__fields);
        }
    }};
    ($logger:expr, $level:expr, $msg:expr, $($key:ident = $value:expr),+ $(,)?) => {{
        let __logger = &$logger;
        let __level: $crate::Level = $level;
        if __logger.enabled(::core::module_path!(), __level) {
            let __fields = [
                $(
                    $crate::Field::new(stringify!($key), $crate::Value::from($value)),
                )+
            ];
            let __metadata = $crate::Metadata::new(__level, ::core::module_path!())
                .with_location(::core::file!(), ::core::line!());
            let _ = __logger.try_emit(__metadata, $msg, &__fields);
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
///
/// Despite the name colliding with the `warn` attribute, this macro
/// works in expression position the same way `log::warn!` does in the
/// upstream `log` crate.
#[macro_export]
macro_rules! warn {
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

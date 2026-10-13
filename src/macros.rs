//! Logging macros and the hidden helpers they expand to.

/// Hidden support items for the logging macros. Not part of the public
/// API; may change in any release.
#[doc(hidden)]
pub mod __private {
    use core::fmt;

    use crate::logger::Logger;
    use crate::record::{Field, Metadata};

    /// The global logger, if installed.
    #[inline]
    #[must_use]
    pub fn global() -> Option<&'static Logger> {
        crate::global::get()
    }

    /// Emit a record whose message is format arguments. The caller has
    /// already checked `Logger::enabled`.
    #[inline]
    pub fn emit_args(
        logger: &Logger,
        metadata: Metadata<'_>,
        args: fmt::Arguments<'_>,
        fields: &[Field<'_>],
    ) {
        logger.emit_fmt(metadata, args, fields, &[]);
    }

    /// Emit a record whose message is a `&str` expression. The caller
    /// has already checked `Logger::enabled`.
    #[inline]
    pub fn emit_str(logger: &Logger, metadata: Metadata<'_>, message: &str, fields: &[Field<'_>]) {
        // Failures were already passed to the logger's error handler.
        let _reported = logger.emit_unchecked(metadata, message, fields, &[]);
    }
}

/// Emit a record at a runtime-chosen level.
///
/// ```text
/// log_at!(LOGGER, LEVEL, [target: EXPR,] MESSAGE [, ARG | FIELD]* [,])
/// log_at!(LEVEL, [target: EXPR,] "literal message" [, ARG | FIELD]* [,])   // global logger
/// ```
///
/// The level macros ([`trace!`](crate::trace!), [`debug!`](crate::debug!),
/// [`info!`](crate::info!), [`warn!`](crate::warn!),
/// [`error!`](crate::error!)) take the same arguments without `LEVEL`.
///
/// # Logger
///
/// The first argument is anything that dereferences to a
/// [`crate::Logger`] (`Logger`, `&Logger`, `Arc<Logger>`, ...). When the
/// first argument is the message literal (or `target:`), the global
/// logger from [`crate::init`] is used; if none is installed the call
/// does nothing.
///
/// # Message
///
/// A string literal is a format string, exactly as in `format!`:
/// positional arguments follow it and inline captures (`{id}`) work.
/// Write `{{` and `}}` for literal braces. A literal without arguments
/// is used as-is and costs nothing extra; otherwise the text is
/// rendered into a reused thread-local buffer (no allocation in steady
/// state). A non-literal message must be a `&str` expression and takes
/// no format arguments.
///
/// # Fields
///
/// After the message, `key = value` pairs become structured fields. Keys
/// are identifiers, dotted paths (`http.status = 200`), or string
/// literals (`"content-type" = ct`). Values are converted with
/// `Value::from`; prefix a value with `%` to record it through
/// `Display`, or with `?` to record it through `Debug`, lazily.
/// Because `name = value` always means a field, named format
/// arguments (`format!("{x}", x = 1)`) are not supported; use inline
/// captures or positional arguments instead.
///
/// # Evaluation
///
/// The filter is checked once, first. Field values and format
/// arguments are evaluated only when the record is enabled. The target
/// defaults to `module_path!()` and the source location (`file!()`,
/// `line!()`) is always attached.
///
/// # Examples
///
#[cfg_attr(all(feature = "logfmt"), doc = "```")]
#[cfg_attr(not(all(feature = "logfmt")), doc = "```ignore")]
/// use log_io::{Level, Logger};
///
/// let logger = Logger::builder().stdout_logfmt().build();
/// let user = "alice";
/// let addr = std::net::Ipv4Addr::LOCALHOST;
///
/// log_io::log_at!(logger, Level::Info, "user {user} logged in from {}", addr,
///     http.status = 200_u16, peer = %addr, attempts = ?[1, 2]);
/// log_io::log_at!(logger, Level::Warn, target: "auth", "retrying", "retry-after" = 3_u8);
/// ```
#[macro_export]
macro_rules! log_at {
    // Global logger, explicit target.
    ($level:expr, target: $target:expr, $msg:literal $($rest:tt)*) => {
        $crate::__log_io_event!(@munch ((global) ($level) ($target) (lit $msg)) [] [] $($rest)*)
    };
    // Global logger.
    ($level:expr, $msg:literal $($rest:tt)*) => {
        $crate::__log_io_event!(
            @munch ((global) ($level) (::core::module_path!()) (lit $msg)) [] [] $($rest)*
        )
    };
    // Explicit logger and target.
    ($logger:expr, $level:expr, target: $target:expr, $($rest:tt)+) => {
        $crate::__log_io_event!(@msg (logger $logger) ($level) ($target) $($rest)+)
    };
    // Explicit logger.
    ($logger:expr, $level:expr, $($rest:tt)+) => {
        $crate::__log_io_event!(@msg (logger $logger) ($level) (::core::module_path!()) $($rest)+)
    };
}

/// Emit a [`crate::Level::Trace`] record. See [`log_at!`](crate::log_at!) for the syntax.
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// # let logger = log_io::Logger::builder().stdout_json().build();
/// log_io::trace!(logger, "entering", depth = 3_u8);
/// ```
#[macro_export]
macro_rules! trace {
    ($($args:tt)+) => { $crate::__log_io_level!($crate::Level::Trace; $($args)+) };
}

/// Emit a [`crate::Level::Debug`] record. See [`log_at!`](crate::log_at!) for the syntax.
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// # let logger = log_io::Logger::builder().stdout_json().build();
/// log_io::debug!(logger, "cache miss for {}", "user:42", shard = 7_u8);
/// ```
#[macro_export]
macro_rules! debug {
    ($($args:tt)+) => { $crate::__log_io_level!($crate::Level::Debug; $($args)+) };
}

/// Emit a [`crate::Level::Info`] record. See [`log_at!`](crate::log_at!) for the syntax.
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// let logger = log_io::Logger::builder().stdout_json().build();
/// log_io::info!(logger, "server started", port = 8080_u16);
///
/// // Without a logger argument the global logger is used (if any).
/// log_io::info!("no global logger yet, so this is a no-op");
/// ```
#[macro_export]
macro_rules! info {
    ($($args:tt)+) => { $crate::__log_io_level!($crate::Level::Info; $($args)+) };
}

/// Emit a [`crate::Level::Warn`] record. See [`log_at!`](crate::log_at!) for the syntax.
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// # let logger = log_io::Logger::builder().stdout_json().build();
/// log_io::warn!(logger, "slow request", path = "/api", ms = 412_u64);
/// ```
#[macro_export]
macro_rules! warn {
    ($($args:tt)+) => { $crate::__log_io_level!($crate::Level::Warn; $($args)+) };
}

/// Emit a [`crate::Level::Error`] record. See [`log_at!`](crate::log_at!) for the syntax.
///
/// With the default flush policy the record is flushed to every sink
/// before the macro returns.
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// # let logger = log_io::Logger::builder().stdout_json().build();
/// let err = std::io::Error::other("connection reset");
/// log_io::error!(logger, "request failed", error = log_io::Value::Error(&err));
/// ```
#[macro_export]
macro_rules! error {
    ($($args:tt)+) => { $crate::__log_io_level!($crate::Level::Error; $($args)+) };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __log_io_level {
    // Global logger: the first argument is `target:` or the message literal.
    ($level:expr; target: $target:expr, $msg:literal $($rest:tt)*) => {
        $crate::__log_io_event!(@munch ((global) ($level) ($target) (lit $msg)) [] [] $($rest)*)
    };
    ($level:expr; $msg:literal $($rest:tt)*) => {
        $crate::__log_io_event!(
            @munch ((global) ($level) (::core::module_path!()) (lit $msg)) [] [] $($rest)*
        )
    };
    // Explicit logger.
    ($level:expr; $logger:expr, target: $target:expr, $($rest:tt)+) => {
        $crate::__log_io_event!(@msg (logger $logger) ($level) ($target) $($rest)+)
    };
    ($level:expr; $logger:expr, $($rest:tt)+) => {
        $crate::__log_io_event!(@msg (logger $logger) ($level) (::core::module_path!()) $($rest)+)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __log_io_event {
    // ---- message ---------------------------------------------------
    (@msg $lg:tt $level:tt $target:tt $msg:literal $($rest:tt)*) => {
        $crate::__log_io_event!(@munch ($lg $level $target (lit $msg)) [] [] $($rest)*)
    };
    (@msg $lg:tt $level:tt $target:tt $msg:expr $(, $($rest:tt)*)?) => {
        $crate::__log_io_event!(@munch ($lg $level $target (expr $msg)) [] [] $(, $($rest)*)?)
    };

    // ---- arguments and fields ---------------------------------------
    // End of input (optionally after a trailing comma).
    (@munch $ctx:tt [$($a:tt)*] [$($f:tt)*] $(,)?) => {
        $crate::__log_io_event!(@emit $ctx [$($a)*] [$($f)*])
    };
    // String-literal keys.
    (@munch $ctx:tt [$($a:tt)*] [$($f:tt)*] , $k:literal = % $v:expr $(, $($rest:tt)*)?) => {
        $crate::__log_io_event!(@munch $ctx [$($a)*]
            [$($f)* ($crate::Field::new($k, $crate::Value::Display(&$v)))] $(, $($rest)*)?)
    };
    (@munch $ctx:tt [$($a:tt)*] [$($f:tt)*] , $k:literal = ? $v:expr $(, $($rest:tt)*)?) => {
        $crate::__log_io_event!(@munch $ctx [$($a)*]
            [$($f)* ($crate::Field::new($k, $crate::Value::Debug(&$v)))] $(, $($rest)*)?)
    };
    (@munch $ctx:tt [$($a:tt)*] [$($f:tt)*] , $k:literal = $v:expr $(, $($rest:tt)*)?) => {
        $crate::__log_io_event!(@munch $ctx [$($a)*]
            [$($f)* ($crate::Field::new($k, $crate::Value::from($v)))] $(, $($rest)*)?)
    };
    // Identifier and dotted-path keys.
    (@munch $ctx:tt [$($a:tt)*] [$($f:tt)*]
        , $k0:ident $(. $kn:ident)* = % $v:expr $(, $($rest:tt)*)?) => {
        $crate::__log_io_event!(@munch $ctx [$($a)*]
            [$($f)* ($crate::Field::new(
                ::core::concat!(::core::stringify!($k0) $(, ".", ::core::stringify!($kn))*),
                $crate::Value::Display(&$v),
            ))] $(, $($rest)*)?)
    };
    (@munch $ctx:tt [$($a:tt)*] [$($f:tt)*]
        , $k0:ident $(. $kn:ident)* = ? $v:expr $(, $($rest:tt)*)?) => {
        $crate::__log_io_event!(@munch $ctx [$($a)*]
            [$($f)* ($crate::Field::new(
                ::core::concat!(::core::stringify!($k0) $(, ".", ::core::stringify!($kn))*),
                $crate::Value::Debug(&$v),
            ))] $(, $($rest)*)?)
    };
    (@munch $ctx:tt [$($a:tt)*] [$($f:tt)*]
        , $k0:ident $(. $kn:ident)* = $v:expr $(, $($rest:tt)*)?) => {
        $crate::__log_io_event!(@munch $ctx [$($a)*]
            [$($f)* ($crate::Field::new(
                ::core::concat!(::core::stringify!($k0) $(, ".", ::core::stringify!($kn))*),
                $crate::Value::from($v),
            ))] $(, $($rest)*)?)
    };
    // Anything else is a positional format argument.
    (@munch $ctx:tt [$($a:tt)*] [$($f:tt)*] , $arg:expr $(, $($rest:tt)*)?) => {
        $crate::__log_io_event!(@munch $ctx [$($a)* ($arg)] [$($f)*] $(, $($rest)*)?)
    };

    // ---- emission -----------------------------------------------------
    (@emit ((global) $level:tt $target:tt $msg:tt) $args:tt $fields:tt) => {
        if let ::core::option::Option::Some(__log_io_logger) = $crate::__private::global() {
            $crate::__log_io_event!(@body __log_io_logger $level $target $msg $args $fields)
        }
    };
    (@emit ((logger $logger:expr) $level:tt $target:tt $msg:tt) $args:tt $fields:tt) => {{
        let __log_io_logger: &$crate::Logger = &$logger;
        $crate::__log_io_event!(@body __log_io_logger $level $target $msg $args $fields)
    }};
    (@body $lg:ident ($level:expr) ($target:expr) (lit $msg:literal) [$($a:tt)*] [$($f:tt)*]) => {{
        let __log_io_level: $crate::Level = $level;
        let __log_io_target: &str = $target;
        if $lg.enabled(__log_io_target, __log_io_level) {
            $crate::__private::emit_args(
                $lg,
                $crate::Metadata::new(__log_io_level, __log_io_target)
                    .with_location(::core::file!(), ::core::line!()),
                ::core::format_args!($msg $(, $a)*),
                &[$($f),*],
            );
        }
    }};
    (@body $lg:ident ($level:expr) ($target:expr) (expr $msg:expr) [] [$($f:tt)*]) => {{
        let __log_io_level: $crate::Level = $level;
        let __log_io_target: &str = $target;
        if $lg.enabled(__log_io_target, __log_io_level) {
            $crate::__private::emit_str(
                $lg,
                $crate::Metadata::new(__log_io_level, __log_io_target)
                    .with_location(::core::file!(), ::core::line!()),
                $msg,
                &[$($f),*],
            );
        }
    }};
}

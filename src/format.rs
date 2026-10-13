//! Output formats.
//!
//! A [`Format`] turns a [`crate::Record`] into text on a
//! [`core::fmt::Write`] destination. Three formats ship in the crate,
//! each behind its own feature flag: [`JsonFormat`], [`LogfmtFormat`]
//! and [`HumanFormat`]. Custom formats implement the trait, or wrap a
//! closure with [`from_fn`].
//!
//! # Output safety
//!
//! The built-in formats share one escaping policy for untrusted text:
//! C0 and C1 control characters, DEL, the Unicode line and paragraph
//! separators (U+2028, U+2029) and the bidirectional override and
//! isolate controls are always escaped. Every record therefore renders
//! as exactly one line (except [`JsonFormat::pretty`]) and cannot
//! inject terminal escape sequences, whatever the message, target, key
//! or value contains.
//!
//! JSON and logfmt reserve the keys `timestamp`, `level`, `target`,
//! `message`, `file` and `line`. A user field with one of those names
//! is written as `fields.<name>` so it can never be mistaken for, or
//! override, the record's real metadata.
//!
//! # Dynamic and static dispatch
//!
//! [`Format::write_record`] takes `&mut dyn fmt::Write`, which makes
//! the trait object-safe: `Box<dyn Format>`, `&dyn Format` and
//! `Arc<dyn Format>` all work, so a format can be chosen at run time.
//! [`Format::write_record_generic`] is the statically dispatched
//! variant the built-in sinks call. The built-in formats override it
//! with a monomorphized implementation; measured on the reference
//! machine, the `dyn` path costs roughly 40-60% more per record for
//! the built-in formats because every small write becomes an indirect
//! call.

use core::fmt;

use crate::record::Record;

/// A record-to-text serializer.
///
/// Implementors write exactly one logical record per call, including
/// any trailing newline: the built-in sinks add no separators between
/// records. Formatters write to [`core::fmt::Write`] (not
/// `std::io::Write`) so they work under `no_std` and can format into a
/// `String` or a fixed buffer.
///
/// Only [`Format::write_record`] is required. Override
/// [`Format::write_record_generic`] as well if your format is on a hot
/// path and you want the built-in sinks to call it without dynamic
/// dispatch.
///
/// # Examples
///
/// ```
/// use core::fmt;
/// use log_io::format::Format;
/// use log_io::{Level, Metadata, Record};
///
/// struct Minimal;
///
/// impl Format for Minimal {
///     fn write_record(&self, record: &Record<'_>, w: &mut dyn fmt::Write) -> fmt::Result {
///         writeln!(w, "{} {}", record.metadata.level, record.message)
///     }
/// }
///
/// let record = Record::new(Metadata::new(Level::Info, "app"), "ready", &[]);
/// let mut out = String::new();
/// Minimal.write_record(&record, &mut out)?;
/// assert_eq!(out, "info ready\n");
///
/// // The trait is object-safe, so formats can be picked at run time.
/// let chosen: Box<dyn Format> = Box::new(Minimal);
/// out.clear();
/// chosen.write_record(&record, &mut out)?;
/// assert_eq!(out, "info ready\n");
/// # Ok::<(), fmt::Error>(())
/// ```
pub trait Format: Send + Sync {
    /// Write `record` to `writer`.
    ///
    /// # Errors
    ///
    /// Returns [`core::fmt::Error`] if the writer rejects a write (or
    /// if a lazily formatted value's `Display` / `Debug` impl fails).
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result;

    /// Statically dispatched variant of [`Format::write_record`].
    ///
    /// The built-in sinks call this method. The default implementation
    /// forwards to `write_record`; formats that care about the cost of
    /// dynamic dispatch override it with a generic implementation.
    ///
    /// # Errors
    ///
    /// Same as [`Format::write_record`].
    #[inline]
    fn write_record_generic<W: fmt::Write>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result
    where
        Self: Sized,
    {
        self.write_record(record, writer)
    }
}

impl<T: Format> Format for &T {
    #[inline]
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result {
        (**self).write_record(record, writer)
    }

    #[inline]
    fn write_record_generic<W: fmt::Write>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        (**self).write_record_generic(record, writer)
    }
}

impl Format for &dyn Format {
    #[inline]
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result {
        (**self).write_record(record, writer)
    }
}

#[cfg(feature = "std")]
impl<T: Format> Format for std::sync::Arc<T> {
    #[inline]
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result {
        (**self).write_record(record, writer)
    }

    #[inline]
    fn write_record_generic<W: fmt::Write>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        (**self).write_record_generic(record, writer)
    }
}

#[cfg(feature = "std")]
impl Format for std::sync::Arc<dyn Format> {
    #[inline]
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result {
        (**self).write_record(record, writer)
    }
}

#[cfg(feature = "std")]
impl<T: Format> Format for Box<T> {
    #[inline]
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result {
        (**self).write_record(record, writer)
    }

    #[inline]
    fn write_record_generic<W: fmt::Write>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        (**self).write_record_generic(record, writer)
    }
}

#[cfg(feature = "std")]
impl Format for Box<dyn Format> {
    #[inline]
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result {
        (**self).write_record(record, writer)
    }
}

/// A [`Format`] built from a closure. Create one with [`from_fn`].
#[derive(Clone, Copy)]
pub struct FnFormat<F> {
    f: F,
}

impl<F> fmt::Debug for FnFormat<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnFormat").finish_non_exhaustive()
    }
}

impl<F> Format for FnFormat<F>
where
    F: Fn(&Record<'_>, &mut dyn fmt::Write) -> fmt::Result + Send + Sync,
{
    #[inline]
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result {
        (self.f)(record, writer)
    }
}

/// Use a closure as a [`Format`].
///
/// The closure receives the record and the destination writer and must
/// write one complete record, including its trailing newline.
///
/// # Examples
///
/// ```
/// use log_io::format::{self, Format};
/// use log_io::{Level, Metadata, Record};
///
/// let csv = format::from_fn(|r: &Record<'_>, w: &mut dyn core::fmt::Write| {
///     writeln!(w, "{},{},{}", r.metadata.level, r.metadata.target, r.message)
/// });
///
/// let record = Record::new(Metadata::new(Level::Warn, "db"), "slow", &[]);
/// let mut out = String::new();
/// csv.write_record(&record, &mut out)?;
/// assert_eq!(out, "warn,db,slow\n");
/// # Ok::<(), core::fmt::Error>(())
/// ```
pub const fn from_fn<F>(f: F) -> FnFormat<F>
where
    F: Fn(&Record<'_>, &mut dyn fmt::Write) -> fmt::Result + Send + Sync,
{
    FnFormat { f }
}

#[cfg(any(feature = "json", feature = "logfmt", feature = "human"))]
pub(crate) mod escape;

#[cfg(any(feature = "json", feature = "human", feature = "logfmt"))]
pub(crate) mod rfc3339;

#[cfg(feature = "json")]
mod json;
#[cfg(feature = "json")]
pub use self::json::JsonFormat;

#[cfg(feature = "logfmt")]
mod logfmt;
#[cfg(feature = "logfmt")]
pub use self::logfmt::LogfmtFormat;

#[cfg(feature = "human")]
mod human;
#[cfg(all(feature = "human", feature = "std"))]
pub use self::human::ColorChoice;
#[cfg(feature = "human")]
pub use self::human::HumanFormat;

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::{Level, Metadata};

    struct Upper;
    impl Format for Upper {
        fn write_record(&self, record: &Record<'_>, w: &mut dyn fmt::Write) -> fmt::Result {
            writeln!(w, "{}", record.metadata.level.as_str_upper())
        }
    }

    fn record() -> Record<'static> {
        Record::new(Metadata::new(Level::Info, "t"), "m", &[])
    }

    fn render<F: Format>(f: &F) -> String {
        let mut out = String::new();
        f.write_record_generic(&record(), &mut out).unwrap();
        let mut dyn_out = String::new();
        f.write_record(&record(), &mut dyn_out).unwrap();
        assert_eq!(out, dyn_out, "static and dynamic paths must agree");
        out
    }

    #[test]
    fn test_format_adapters_forward_to_inner() {
        assert_eq!(render(&Upper), "INFO\n");
        assert_eq!(render(&&Upper), "INFO\n");
        assert_eq!(render(&Box::new(Upper)), "INFO\n");
        assert_eq!(render(&std::sync::Arc::new(Upper)), "INFO\n");
        let boxed: Box<dyn Format> = Box::new(Upper);
        assert_eq!(render(&boxed), "INFO\n");
        let arced: std::sync::Arc<dyn Format> = std::sync::Arc::new(Upper);
        assert_eq!(render(&arced), "INFO\n");
        let borrowed: &dyn Format = &Upper;
        assert_eq!(render(&borrowed), "INFO\n");
    }

    #[test]
    fn test_from_fn_closure_is_a_format() {
        let f = from_fn(|r: &Record<'_>, w: &mut dyn fmt::Write| writeln!(w, "<{}>", r.message));
        assert_eq!(render(&f), "<m>\n");
        assert!(format!("{f:?}").starts_with("FnFormat"));
    }

    #[test]
    fn test_format_errors_propagate() {
        struct Failing;
        impl fmt::Write for Failing {
            fn write_str(&mut self, _: &str) -> fmt::Result {
                Err(fmt::Error)
            }
        }
        assert!(Upper.write_record_generic(&record(), &mut Failing).is_err());
    }
}

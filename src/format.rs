//! Output formats.
//!
//! A [`Format`] turns a [`crate::Record`] into bytes on a writer. Three
//! formats ship in the crate and are gated by feature flags. Custom
//! formats can be supplied as a closure or by implementing the trait.
//!
//! All formatters write directly to the supplied writer without
//! intermediate allocation.

use core::fmt;

use crate::record::Record;

/// A record-to-bytes serializer.
///
/// Implementors should write exactly one logical "line" (or one
/// logical document) per call. The included sinks add no extra
/// separators between records; formatters that produce line-oriented
/// output are expected to emit their own trailing newline.
pub trait Format: Send + Sync {
    /// Write the record to `writer` using a [`core::fmt::Write`] sink.
    ///
    /// `core::fmt::Write` is used (instead of `std::io::Write`) so that
    /// formatters compile under `no_std` and can be used to format
    /// into a `String` or fixed-size buffer.
    ///
    /// # Errors
    ///
    /// Returns [`core::fmt::Error`] if the underlying writer rejects a
    /// write.
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result;
}

/// Helper adapter that lets `&dyn Format` be used as `Format`.
impl<T: Format + ?Sized> Format for &T {
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        (**self).write_record(record, writer)
    }
}

#[cfg(feature = "std")]
impl<T: Format + ?Sized> Format for std::sync::Arc<T> {
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        (**self).write_record(record, writer)
    }
}

#[cfg(feature = "std")]
impl<T: Format + ?Sized> Format for Box<T> {
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        (**self).write_record(record, writer)
    }
}

#[cfg(any(feature = "json", feature = "human"))]
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
#[cfg(feature = "human")]
pub use self::human::HumanFormat;

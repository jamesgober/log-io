//! Log records and their constituent parts.
//!
//! The data model is intentionally small. A [`Record`] is the
//! immutable unit that flows from logger to filter to formatter to
//! sink. All its parts borrow, so a record can be constructed on the
//! stack without touching the allocator.

use crate::level::Level;
use crate::value::Value;

/// A single key-value pair attached to a [`Record`].
///
/// Keys are borrowed strings. Values are borrowed via [`Value`]. Keys
/// may contain any UTF-8; the built-in formats sanitize or escape them
/// so a hostile key cannot break the output framing.
///
/// # Examples
///
/// ```
/// use log_io::{Field, Value};
///
/// let f = Field::new("port", 8080_u32);
/// assert_eq!(f.key, "port");
/// assert_eq!(f.value, Value::U64(8080));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field<'a> {
    /// The field name.
    pub key: &'a str,
    /// The field value.
    pub value: Value<'a>,
}

impl<'a> Field<'a> {
    /// Construct a field from a key and anything convertible to a
    /// [`Value`], so callers can write `Field::new("port", 8080_u32)`.
    pub fn new<V: Into<Value<'a>>>(key: &'a str, value: V) -> Self {
        Self {
            key,
            value: value.into(),
        }
    }
}

/// Metadata describing the origin and severity of a record.
///
/// `level` and `target` are required; the rest is best-effort context.
/// The logging macros populate `file` and `line` automatically and the
/// [`crate::Logger`] fills in `timestamp_unix_nanos` when timestamps
/// are enabled.
///
/// The struct is `#[non_exhaustive]` so new metadata can be added in a
/// minor release. Build it with [`Metadata::new`] and the `with_*`
/// methods; every field stays publicly readable.
///
/// # Examples
///
/// ```
/// use log_io::{Level, Metadata};
///
/// let meta = Metadata::new(Level::Warn, "app::db")
///     .with_location("src/db.rs", 42)
///     .with_timestamp(1_700_000_000_000_000_000);
/// assert_eq!(meta.target, "app::db");
/// assert_eq!(meta.line, Some(42));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Metadata<'a> {
    /// Severity of the record.
    pub level: Level,
    /// Logical target. Typically the module path, but callers are free
    /// to use a domain-specific identifier for filtering.
    pub target: &'a str,
    /// Source file path, if known.
    pub file: Option<&'a str>,
    /// Source line number, if known.
    pub line: Option<u32>,
    /// Nanoseconds since the Unix epoch, if known. Populated by the
    /// logger when std is available and timestamps are enabled. `None`
    /// means the consumer should supply its own clock (typical in
    /// `no_std` builds).
    pub timestamp_unix_nanos: Option<u128>,
}

impl<'a> Metadata<'a> {
    /// Build minimal metadata containing only `level` and `target`.
    /// All optional fields are `None`.
    #[must_use]
    pub const fn new(level: Level, target: &'a str) -> Self {
        Self {
            level,
            target,
            file: None,
            line: None,
            timestamp_unix_nanos: None,
        }
    }

    /// Replace the source location.
    #[must_use]
    pub const fn with_location(mut self, file: &'a str, line: u32) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self
    }

    /// Replace the timestamp (nanoseconds since the Unix epoch).
    #[must_use]
    pub const fn with_timestamp(mut self, ts_unix_nanos: u128) -> Self {
        self.timestamp_unix_nanos = Some(ts_unix_nanos);
        self
    }
}

/// A complete, immutable log record.
///
/// Records borrow their data; the lifetime parameter is the shortest
/// of the borrows. A [`crate::Sink`] receives a `&Record` and must not
/// retain it past the call.
///
/// `fields` holds the per-call fields and `context` holds ambient
/// fields (logger defaults and thread-local context). When a record is
/// produced by a [`crate::Logger`], keys are unique across both slices:
/// a call field shadows a context field with the same key, and a
/// context field shadows a default field. Hand-built records are
/// rendered exactly as given.
///
/// The struct is `#[non_exhaustive]`; construct it with
/// [`Record::new`] or [`Record::with_context`].
///
/// # Examples
///
/// ```
/// use log_io::{Field, Level, Metadata, Record};
///
/// let fields = [Field::new("port", 80_u16)];
/// let ctx = [Field::new("trace_id", "abc")];
/// let record = Record::with_context(Metadata::new(Level::Info, "app"), "up", &fields, &ctx);
/// let keys: Vec<&str> = record.all_fields().map(|f| f.key).collect();
/// assert_eq!(keys, ["trace_id", "port"]);
/// ```
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Record<'a> {
    /// Static origin and severity information.
    pub metadata: Metadata<'a>,
    /// Free-form message.
    pub message: &'a str,
    /// Borrowed per-call structured fields. May be empty.
    pub fields: &'a [Field<'a>],
    /// Borrowed context fields (defaults and thread-local context).
    /// May be empty. Kept apart from `fields` so formatters can label
    /// or order them differently.
    pub context: &'a [Field<'a>],
}

impl<'a> Record<'a> {
    /// Build a record with no context fields.
    #[must_use]
    pub const fn new(metadata: Metadata<'a>, message: &'a str, fields: &'a [Field<'a>]) -> Self {
        Self {
            metadata,
            message,
            fields,
            context: &[],
        }
    }

    /// Build a record with explicit context fields.
    #[must_use]
    pub const fn with_context(
        metadata: Metadata<'a>,
        message: &'a str,
        fields: &'a [Field<'a>],
        context: &'a [Field<'a>],
    ) -> Self {
        Self {
            metadata,
            message,
            fields,
            context,
        }
    }

    /// Iterate over context fields followed by per-call fields, the
    /// order every built-in formatter uses.
    pub fn all_fields(&self) -> impl Iterator<Item = &Field<'a>> {
        self.context.iter().chain(self.fields.iter())
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_field_new_accepts_typed_values() {
        let f = Field::new("port", 8080_u32);
        assert_eq!(f.key, "port");
        assert_eq!(f.value, Value::U64(8080));
    }

    #[test]
    fn test_metadata_chain_setters_populate_options() {
        let m = Metadata::new(Level::Info, "tgt")
            .with_location("file.rs", 12)
            .with_timestamp(1_700_000_000_000_000_000);
        assert_eq!(m.file, Some("file.rs"));
        assert_eq!(m.line, Some(12));
        assert_eq!(m.timestamp_unix_nanos, Some(1_700_000_000_000_000_000));
        let bare = Metadata::new(Level::Info, "tgt");
        assert_eq!(bare.file, None);
        assert_eq!(bare.timestamp_unix_nanos, None);
    }

    #[test]
    fn test_record_all_fields_orders_context_first() {
        let ctx = [Field::new("trace_id", "abc")];
        let fields = [Field::new("port", 80_u32)];
        let record = Record::with_context(Metadata::new(Level::Info, "tgt"), "msg", &fields, &ctx);
        let keys: Vec<_> = record.all_fields().map(|f| f.key).collect();
        assert_eq!(keys, vec!["trace_id", "port"]);
        let plain = Record::new(Metadata::new(Level::Info, "tgt"), "msg", &fields);
        assert!(plain.context.is_empty());
    }
}

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
/// Keys are borrowed strings. Values are borrowed via [`Value`]. A
/// builder that needs to attach owned data must keep that data alive
/// for the duration of the [`Record`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field<'a> {
    /// The field name.
    pub key: &'a str,
    /// The field value.
    pub value: Value<'a>,
}

impl<'a> Field<'a> {
    /// Construct a field from a key and a value.
    ///
    /// `value` is `impl Into<Value>` so callers can write
    /// `Field::new("port", 8080_u32)` directly.
    pub fn new<V: Into<Value<'a>>>(key: &'a str, value: V) -> Self {
        Self {
            key,
            value: value.into(),
        }
    }
}

/// Metadata describing the origin and severity of a record.
///
/// Most metadata is optional. `level` and `target` are required; the
/// rest is best-effort context that a caller may supply. Macros
/// populate `file` and `line` automatically.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// Unix nanoseconds since the epoch. Populated by the logger when
    /// std is available. `None` indicates the consumer should supply
    /// its own clock (typical in `no_std` builds).
    pub timestamp_unix_nanos: Option<u128>,
}

impl<'a> Metadata<'a> {
    /// Build minimal metadata containing only `level` and `target`.
    ///
    /// All optional fields are `None`. Useful for handcrafted records
    /// where a richer context isn't needed.
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
    pub const fn with_location(mut self, file: &'a str, line: u32) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self
    }

    /// Replace the timestamp.
    pub const fn with_timestamp(mut self, ts_unix_nanos: u128) -> Self {
        self.timestamp_unix_nanos = Some(ts_unix_nanos);
        self
    }
}

/// A complete, immutable log record.
///
/// Records borrow their data; the lifetime parameter is the shortest
/// of the borrows. A [`crate::Sink`] receives a `&Record` and must not
/// outlive it.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    /// Static origin and severity information.
    pub metadata: Metadata<'a>,
    /// Free-form message.
    pub message: &'a str,
    /// Borrowed structured fields. May be empty.
    pub fields: &'a [Field<'a>],
    /// Borrowed context fields. May be empty. Distinct from `fields`
    /// so formatters can label them (for example, prefixing context
    /// keys in human format) and filters can skip them.
    pub context: &'a [Field<'a>],
}

impl<'a> Record<'a> {
    /// Build a record with no context fields.
    pub const fn new(metadata: Metadata<'a>, message: &'a str, fields: &'a [Field<'a>]) -> Self {
        Self {
            metadata,
            message,
            fields,
            context: &[],
        }
    }

    /// Build a record with explicit context fields.
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

    /// Iterate over context fields followed by structured fields.
    ///
    /// Most formatters render them in this order so context appears
    /// before the per-call data.
    pub fn all_fields(&self) -> impl Iterator<Item = &Field<'a>> {
        self.context.iter().chain(self.fields.iter())
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn field_new_accepts_typed_values() {
        let f = Field::new("port", 8080_u32);
        assert_eq!(f.key, "port");
        assert_eq!(f.value, Value::U64(8080));
    }

    #[test]
    fn metadata_chain_setters() {
        let m = Metadata::new(Level::Info, "tgt")
            .with_location("file.rs", 12)
            .with_timestamp(1_700_000_000_000_000_000);
        assert_eq!(m.file, Some("file.rs"));
        assert_eq!(m.line, Some(12));
        assert_eq!(m.timestamp_unix_nanos, Some(1_700_000_000_000_000_000));
    }

    #[test]
    fn record_all_fields_orders_context_first() {
        let ctx = [Field::new("trace_id", "abc")];
        let fields = [Field::new("port", 80_u32)];
        let record = Record::with_context(Metadata::new(Level::Info, "tgt"), "msg", &fields, &ctx);
        let keys: Vec<_> = record.all_fields().map(|f| f.key).collect();
        assert_eq!(keys, vec!["trace_id", "port"]);
    }
}

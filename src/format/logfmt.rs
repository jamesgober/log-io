//! logfmt output format.
//!
//! Emits `key=value` pairs separated by single spaces and terminated by
//! a newline. Values that are empty or contain whitespace, `=`, `"`,
//! `\` or a control character are double-quoted, with JSON-compatible
//! escapes inside the quotes (`\"`, `\\`, `\n`, `\u001b`, ...), which
//! is what common logfmt decoders accept. Keys are never quoted:
//! characters that would break parsing are replaced with `_`.

use core::fmt;
use core::fmt::Write as _;

use super::Format;
use super::escape::{
    JsonEscaper, QuoteScan, RESERVED_PREFIX, is_reserved_key, value_needs_quotes,
    write_json_escaped, write_sanitized_key,
};
use crate::record::Record;
use crate::value::{Value, write_f64, write_hex};

/// logfmt formatter. See the module docs for the wire format.
///
/// User keys named `timestamp`, `level`, `target`, `message`, `file`
/// or `line` are written as `fields.<name>`.
///
/// # Examples
///
#[cfg_attr(all(feature = "logfmt"), doc = "```")]
#[cfg_attr(not(all(feature = "logfmt")), doc = "```ignore")]
/// use log_io::format::{Format, LogfmtFormat};
/// use log_io::{Field, Level, Metadata, Record};
///
/// let fields = [Field::new("port", 8080_u32), Field::new("path", "/a b")];
/// let record = Record::new(Metadata::new(Level::Info, "app"), "ok", &fields);
///
/// let mut out = String::new();
/// LogfmtFormat::new().write_record(&record, &mut out)?;
/// assert_eq!(out, "level=info target=app message=ok port=8080 path=\"/a b\"\n");
/// # Ok::<(), core::fmt::Error>(())
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct LogfmtFormat {
    timestamp_rfc3339: bool,
}

impl LogfmtFormat {
    /// New logfmt formatter.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            timestamp_rfc3339: false,
        }
    }

    /// Emit timestamps as RFC 3339 (`timestamp=2023-11-14T22:13:20.000000000Z`)
    /// instead of Unix nanoseconds (the default).
    #[must_use]
    pub const fn timestamp_rfc3339(mut self, enabled: bool) -> Self {
        self.timestamp_rfc3339 = enabled;
        self
    }

    // Kept out of line: it runs once per record, and inlining it into
    // every sink's write path made pipeline timings depend on unrelated
    // code layout (measured swings of 60% between builds).
    #[inline(never)]
    fn write_impl<W: fmt::Write + ?Sized>(&self, record: &Record<'_>, w: &mut W) -> fmt::Result {
        if let Some(ts) = record.metadata.timestamp_unix_nanos {
            w.write_str("timestamp=")?;
            if self.timestamp_rfc3339 {
                super::rfc3339::write_rfc3339_nanos(ts, w)?;
            } else {
                write!(w, "{ts}")?;
            }
            w.write_char(' ')?;
        }
        w.write_str("level=")?;
        w.write_str(record.metadata.level.as_str())?;
        w.write_str(" target=")?;
        write_str_value(w, record.metadata.target)?;
        w.write_str(" message=")?;
        write_str_value(w, record.message)?;
        if let Some(file) = record.metadata.file {
            w.write_str(" file=")?;
            write_str_value(w, file)?;
        }
        if let Some(line) = record.metadata.line {
            write!(w, " line={line}")?;
        }
        for field in record.all_fields() {
            w.write_char(' ')?;
            if is_reserved_key(field.key) {
                w.write_str(RESERVED_PREFIX)?;
            }
            write_sanitized_key(w, field.key)?;
            w.write_char('=')?;
            write_value(w, field.value)?;
        }
        w.write_char('\n')
    }
}

impl Format for LogfmtFormat {
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result {
        self.write_impl(record, writer)
    }

    #[inline]
    fn write_record_generic<W: fmt::Write>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        self.write_impl(record, writer)
    }
}

#[inline]
fn write_str_value<W: fmt::Write + ?Sized>(w: &mut W, s: &str) -> fmt::Result {
    if value_needs_quotes(s) {
        w.write_char('"')?;
        write_json_escaped(w, s)?;
        w.write_char('"')
    } else {
        w.write_str(s)
    }
}

fn write_value<W: fmt::Write + ?Sized>(w: &mut W, v: Value<'_>) -> fmt::Result {
    match v {
        Value::Null => Ok(()),
        Value::Bool(true) => w.write_str("true"),
        Value::Bool(false) => w.write_str("false"),
        Value::I64(n) => write!(w, "{n}"),
        Value::U64(n) => write!(w, "{n}"),
        Value::I128(n) => write!(w, "{n}"),
        Value::U128(n) => write!(w, "{n}"),
        Value::F64(n) => write_f64(w, n),
        Value::Str(s) => write_str_value(w, s),
        Value::Char(c) => {
            let mut buf = [0_u8; 4];
            write_str_value(w, c.encode_utf8(&mut buf))
        }
        Value::Bytes([]) => w.write_str("\"\""),
        Value::Bytes(b) => write_hex(w, b),
        lazy @ (Value::Display(_) | Value::Debug(_) | Value::Error(_)) => {
            // Rendered twice: once to decide on quoting, once to write.
            if QuoteScan::needs_quotes(&lazy) {
                w.write_char('"')?;
                write!(JsonEscaper(&mut *w), "{lazy}")?;
                w.write_char('"')
            } else {
                write!(w, "{lazy}")
            }
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::Level;
    use crate::record::{Field, Metadata};

    fn fmt_one(fmt: LogfmtFormat, record: &Record<'_>) -> String {
        let mut s = String::new();
        fmt.write_record_generic(record, &mut s).unwrap();
        let mut d = String::new();
        fmt.write_record(record, &mut d).unwrap();
        assert_eq!(s, d);
        s
    }

    fn field_logfmt(v: Value<'_>) -> String {
        let fields = [Field::new("v", v)];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(LogfmtFormat::new(), &record);
        let start = out.find(" v=").unwrap() + 3;
        out[start..out.len() - 1].to_string()
    }

    #[test]
    fn test_logfmt_simple_record() {
        let record = Record::new(Metadata::new(Level::Info, "app"), "hello", &[]);
        assert_eq!(
            fmt_one(LogfmtFormat::new(), &record),
            "level=info target=app message=hello\n"
        );
    }

    #[test]
    fn test_logfmt_metadata_and_timestamps() {
        let meta = Metadata::new(Level::Debug, "a")
            .with_location("src/x.rs", 9)
            .with_timestamp(1_700_000_000_000_000_000);
        let record = Record::new(meta, "m", &[]);
        assert_eq!(
            fmt_one(LogfmtFormat::new(), &record),
            "timestamp=1700000000000000000 level=debug target=a message=m file=src/x.rs line=9\n"
        );
        assert!(
            fmt_one(LogfmtFormat::new().timestamp_rfc3339(true), &record)
                .starts_with("timestamp=2023-11-14T22:13:20.000000000Z level=debug")
        );
    }

    #[test]
    fn test_logfmt_values_render_per_variant() {
        assert_eq!(field_logfmt(Value::Null), "");
        assert_eq!(field_logfmt(Value::Bool(true)), "true");
        assert_eq!(field_logfmt(Value::Bool(false)), "false");
        assert_eq!(field_logfmt(Value::I64(-1)), "-1");
        assert_eq!(field_logfmt(Value::U64(1)), "1");
        assert_eq!(field_logfmt(Value::I128(-2)), "-2");
        assert_eq!(field_logfmt(Value::U128(2)), "2");
        assert_eq!(field_logfmt(Value::F64(1.0)), "1.0");
        assert_eq!(field_logfmt(Value::F64(f64::NAN)), "NaN");
        assert_eq!(field_logfmt(Value::F64(f64::INFINITY)), "inf");
        assert_eq!(field_logfmt(Value::Str("")), "\"\"");
        assert_eq!(field_logfmt(Value::Str("plain")), "plain");
        assert_eq!(field_logfmt(Value::Str("a b")), "\"a b\"");
        assert_eq!(field_logfmt(Value::Str("a\"b")), "\"a\\\"b\"");
        assert_eq!(field_logfmt(Value::Str("a=b")), "\"a=b\"");
        assert_eq!(field_logfmt(Value::Str("a\\b")), "\"a\\\\b\"");
        assert_eq!(field_logfmt(Value::Char('\n')), "\"\\n\"");
        assert_eq!(field_logfmt(Value::Char('x')), "x");
        assert_eq!(field_logfmt(Value::Bytes(&[])), "\"\"");
        assert_eq!(field_logfmt(Value::Bytes(&[1, 255])), "01ff");
        assert_eq!(field_logfmt(Value::Display(&42)), "42");
        assert_eq!(field_logfmt(Value::Display(&"two words")), "\"two words\"");
        assert_eq!(field_logfmt(Value::Debug(&"q")), "\"\\\"q\\\"\"");
        let e = std::io::Error::other("boom");
        assert_eq!(field_logfmt(Value::Error(&e)), "boom");
    }

    #[test]
    fn test_logfmt_control_characters_are_escaped_and_quoted() {
        assert_eq!(field_logfmt(Value::Str("\u{1b}[31m")), "\"\\u001b[31m\"");
        assert_eq!(field_logfmt(Value::Str("x\u{9b}31m")), "\"x\\u009b31m\"");
        assert_eq!(field_logfmt(Value::Str("a\u{7f}")), "\"a\\u007f\"");
        assert_eq!(field_logfmt(Value::Str("a\u{2028}b")), "\"a\\u2028b\"");
        assert_eq!(field_logfmt(Value::Str("tab\there")), "\"tab\\there\"");
    }

    #[test]
    fn test_logfmt_keys_are_sanitized_and_reserved_keys_prefixed() {
        let fields = [
            Field::new("the key", 1_u32),
            Field::new("k\u{1b}[0m", 2_u32),
            Field::new("", 3_u32),
            Field::new("a=b", 4_u32),
            Field::new("level", "fake"),
        ];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(LogfmtFormat::new(), &record);
        assert_eq!(
            out,
            "level=info target=t message=m the_key=1 k_[0m=2 _=3 a_b=4 fields.level=fake\n"
        );
    }

    #[test]
    fn test_logfmt_message_and_target_are_quoted_when_needed() {
        let record = Record::new(Metadata::new(Level::Info, "my app"), "hi there", &[]);
        assert_eq!(
            fmt_one(LogfmtFormat::new(), &record),
            "level=info target=\"my app\" message=\"hi there\"\n"
        );
        let record = Record::new(Metadata::new(Level::Info, ""), "", &[]);
        assert_eq!(
            fmt_one(LogfmtFormat::new(), &record),
            "level=info target=\"\" message=\"\"\n"
        );
    }
}

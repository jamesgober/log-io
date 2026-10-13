//! JSON output format.
//!
//! Emits one JSON object per record, terminated by a newline (JSON
//! Lines). Key order is stable so consumers can rely on prefixes:
//! `timestamp` (if set), `level`, `target`, `message`, `file` and
//! `line` (if set), then context fields, then per-call fields.

use core::fmt;
use core::fmt::Write as _;

use super::Format;
use super::escape::{JsonEscaper, RESERVED_PREFIX, is_reserved_key, write_json_escaped};
use crate::record::Record;
use crate::value::{Value, write_hex};

/// JSON formatter.
///
/// The default configuration emits compact JSON Lines. Every string is
/// escaped so the output is valid JSON for any input, and the escaping
/// also covers DEL, C1 controls, U+2028/U+2029 and bidi controls so a
/// record stays on one line even for tools that treat those as line
/// breaks. User keys named `timestamp`, `level`, `target`, `message`,
/// `file` or `line` are written as `fields.<name>`.
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// use log_io::format::{Format, JsonFormat};
/// use log_io::{Field, Level, Metadata, Record, Value};
///
/// let fmt = JsonFormat::new();
/// let fields = [Field::new("port", 8080_u16), Field::new("level", "spoofed")];
/// let record = Record::new(Metadata::new(Level::Info, "app"), "started", &fields);
///
/// let mut out = String::new();
/// fmt.write_record(&record, &mut out)?;
/// assert_eq!(
///     out,
///     "{\"level\":\"info\",\"target\":\"app\",\"message\":\"started\",\
///       \"port\":8080,\"fields.level\":\"spoofed\"}\n"
/// );
/// # Ok::<(), core::fmt::Error>(())
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonFormat {
    pretty: bool,
    timestamp_rfc3339: bool,
}

impl JsonFormat {
    /// New compact JSON formatter. Equivalent to `Self::default()`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            pretty: false,
            timestamp_rfc3339: false,
        }
    }

    /// Toggle pretty (multi-line, indented) output. Intended for
    /// reading by eye; line-oriented ingestion needs the compact form.
    #[must_use]
    pub const fn pretty(mut self, enabled: bool) -> Self {
        self.pretty = enabled;
        self
    }

    /// Emit timestamps as RFC 3339 strings instead of Unix nanoseconds.
    ///
    /// Enabled: `"timestamp":"2023-11-14T22:13:20.000000000Z"`.
    /// Disabled (default): `"timestamp":1700000000000000000`. Note that
    /// nanosecond integers exceed 2^53, so consumers that parse JSON
    /// numbers as doubles lose sub-microsecond precision; prefer the
    /// RFC 3339 form for such consumers.
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
        let p = if self.pretty { &PRETTY } else { &COMPACT };
        w.write_str(p.open)?;
        if let Some(ts) = record.metadata.timestamp_unix_nanos {
            w.write_str(p.timestamp)?;
            if self.timestamp_rfc3339 {
                w.write_char('"')?;
                super::rfc3339::write_rfc3339_nanos(ts, w)?;
                w.write_char('"')?;
            } else {
                write!(w, "{ts}")?;
            }
            w.write_str(p.next)?;
        }
        w.write_str(p.level)?;
        w.write_str(record.metadata.level.as_str())?;
        w.write_str(p.target)?;
        write_string(w, record.metadata.target)?;
        w.write_str(p.message)?;
        write_string(w, record.message)?;
        if let Some(file) = record.metadata.file {
            w.write_str(p.file)?;
            write_string(w, file)?;
        }
        if let Some(line) = record.metadata.line {
            w.write_str(p.line)?;
            write!(w, "{line}")?;
        }
        for field in record.all_fields() {
            w.write_str(p.field)?;
            if is_reserved_key(field.key) {
                w.write_str(RESERVED_PREFIX)?;
            }
            write_json_escaped(w, field.key)?;
            w.write_str(p.key_end)?;
            write_value(w, field.value)?;
        }
        w.write_str(p.close)
    }
}

/// Punctuation for one layout. The metadata keys always appear in the
/// same order, so every separator is a fixed string and each one is a
/// single write.
struct Punctuation {
    open: &'static str,
    timestamp: &'static str,
    next: &'static str,
    level: &'static str,
    target: &'static str,
    message: &'static str,
    file: &'static str,
    line: &'static str,
    field: &'static str,
    key_end: &'static str,
    close: &'static str,
}

const COMPACT: Punctuation = Punctuation {
    open: "{",
    timestamp: "\"timestamp\":",
    next: ",",
    level: "\"level\":\"",
    target: "\",\"target\":",
    message: ",\"message\":",
    file: ",\"file\":",
    line: ",\"line\":",
    field: ",\"",
    key_end: "\":",
    close: "}
",
};

const PRETTY: Punctuation = Punctuation {
    open: "{
  ",
    timestamp: "\"timestamp\": ",
    next: ",
  ",
    level: "\"level\": \"",
    target: "\",
  \"target\": ",
    message: ",
  \"message\": ",
    file: ",
  \"file\": ",
    line: ",
  \"line\": ",
    field: ",
  \"",
    key_end: "\": ",
    close: "
}
",
};

impl Format for JsonFormat {
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
fn write_string<W: fmt::Write + ?Sized>(w: &mut W, s: &str) -> fmt::Result {
    w.write_char('"')?;
    write_json_escaped(w, s)?;
    w.write_char('"')
}

fn write_value<W: fmt::Write + ?Sized>(w: &mut W, v: Value<'_>) -> fmt::Result {
    match v {
        Value::Null => w.write_str("null"),
        Value::Bool(true) => w.write_str("true"),
        Value::Bool(false) => w.write_str("false"),
        Value::I64(n) => write!(w, "{n}"),
        Value::U64(n) => write!(w, "{n}"),
        Value::I128(n) => write!(w, "{n}"),
        Value::U128(n) => write!(w, "{n}"),
        // `{:?}` always round-trips and always includes `.` or an
        // exponent, so `1.0` stays a float for schema-inferring
        // consumers and huge or tiny values stay short.
        Value::F64(n) if n.is_finite() => write!(w, "{n:?}"),
        Value::F64(_) => w.write_str("null"),
        Value::Str(s) => write_string(w, s),
        Value::Char(c) => {
            let mut buf = [0_u8; 4];
            write_string(w, c.encode_utf8(&mut buf))
        }
        Value::Bytes(b) => {
            w.write_char('"')?;
            write_hex(w, b)?;
            w.write_char('"')
        }
        lazy @ (Value::Display(_) | Value::Debug(_) | Value::Error(_)) => {
            w.write_char('"')?;
            write!(JsonEscaper(&mut *w), "{lazy}")?;
            w.write_char('"')
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::Level;
    use crate::record::{Field, Metadata};

    fn fmt_one(json: JsonFormat, record: &Record<'_>) -> String {
        let mut s = String::new();
        json.write_record_generic(record, &mut s).unwrap();
        let mut d = String::new();
        json.write_record(record, &mut d).unwrap();
        assert_eq!(s, d);
        s
    }

    fn field_json(v: Value<'_>) -> String {
        let fields = [Field::new("v", v)];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(JsonFormat::new(), &record);
        let start = out.find("\"v\":").unwrap() + 4;
        out[start..out.len() - 2].to_string()
    }

    #[test]
    fn test_json_compact_object_with_trailing_newline() {
        let record = Record::new(Metadata::new(Level::Info, "app"), "hello", &[]);
        assert_eq!(
            fmt_one(JsonFormat::new(), &record),
            "{\"level\":\"info\",\"target\":\"app\",\"message\":\"hello\"}\n"
        );
    }

    #[test]
    fn test_json_metadata_order_is_stable() {
        let meta = Metadata::new(Level::Warn, "a")
            .with_location("f.rs", 7)
            .with_timestamp(5);
        let record = Record::new(meta, "m", &[]);
        assert_eq!(
            fmt_one(JsonFormat::new(), &record),
            "{\"timestamp\":5,\"level\":\"warn\",\"target\":\"a\",\"message\":\"m\",\
             \"file\":\"f.rs\",\"line\":7}\n"
        );
    }

    #[test]
    fn test_json_context_before_fields() {
        let ctx = [Field::new("trace_id", "abc")];
        let fields = [Field::new("port", 80_u32)];
        let record = Record::with_context(Metadata::new(Level::Info, "app"), "msg", &fields, &ctx);
        let out = fmt_one(JsonFormat::new(), &record);
        assert!(out.find("\"trace_id\"").unwrap() < out.find("\"port\"").unwrap());
    }

    #[test]
    fn test_json_values_render_per_variant() {
        assert_eq!(field_json(Value::Null), "null");
        assert_eq!(field_json(Value::Bool(true)), "true");
        assert_eq!(field_json(Value::Bool(false)), "false");
        assert_eq!(field_json(Value::I64(-3)), "-3");
        assert_eq!(field_json(Value::U64(3)), "3");
        assert_eq!(field_json(Value::I128(i128::MIN)), i128::MIN.to_string());
        assert_eq!(field_json(Value::U128(u128::MAX)), u128::MAX.to_string());
        assert_eq!(field_json(Value::F64(1.0)), "1.0");
        assert_eq!(field_json(Value::F64(1.5)), "1.5");
        assert_eq!(field_json(Value::F64(1e300)), "1e300");
        assert_eq!(field_json(Value::F64(f64::NAN)), "null");
        assert_eq!(field_json(Value::F64(f64::NEG_INFINITY)), "null");
        assert_eq!(field_json(Value::Str("a\"b\nc")), "\"a\\\"b\\nc\"");
        assert_eq!(field_json(Value::Char('\n')), "\"\\n\"");
        assert_eq!(field_json(Value::Char('"')), "\"\\\"\"");
        assert_eq!(field_json(Value::Bytes(&[0xde, 0xad])), "\"dead\"");
        assert_eq!(field_json(Value::Display(&"x\ty")), "\"x\\ty\"");
        assert_eq!(field_json(Value::Debug(&"q")), "\"\\\"q\\\"\"");
        let e = std::io::Error::other("disk \"full\"");
        assert_eq!(field_json(Value::Error(&e)), "\"disk \\\"full\\\"\"");
    }

    #[test]
    fn test_json_reserved_keys_are_prefixed() {
        let fields = [
            Field::new("level", "fake"),
            Field::new("message", "spoofed"),
            Field::new("timestamp", 0_u32),
            Field::new("target", "x"),
            Field::new("file", "y"),
            Field::new("line", 1_u32),
            Field::new("levels", "not reserved"),
        ];
        let record = Record::new(Metadata::new(Level::Error, "t"), "real", &fields);
        let out = fmt_one(JsonFormat::new(), &record);
        assert_eq!(out.matches("\"level\":").count(), 1, "{out}");
        assert_eq!(out.matches("\"message\":").count(), 1, "{out}");
        for k in ["level", "message", "timestamp", "target", "file", "line"] {
            assert!(out.contains(&format!("\"fields.{k}\":")), "{k}: {out}");
        }
        assert!(out.contains("\"levels\":"), "{out}");
    }

    #[test]
    fn test_json_keys_are_escaped() {
        let fields = [Field::new("we\"ird\n", 1_u32)];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(JsonFormat::new(), &record);
        assert!(out.contains("\"we\\\"ird\\n\":1"), "{out}");
    }

    #[test]
    fn test_json_timestamp_forms() {
        let meta = Metadata::new(Level::Info, "t").with_timestamp(1_700_000_000_000_000_000);
        let record = Record::new(meta, "x", &[]);
        assert!(fmt_one(JsonFormat::new(), &record).contains("\"timestamp\":1700000000000000000"));
        assert!(
            fmt_one(JsonFormat::new().timestamp_rfc3339(true), &record)
                .contains("\"timestamp\":\"2023-11-14T22:13:20.000000000Z\"")
        );
    }

    #[test]
    fn test_json_pretty_indents_and_breaks_lines() {
        let fields = [Field::new("port", 80_u32)];
        let record = Record::new(Metadata::new(Level::Info, "t"), "x", &fields);
        let out = fmt_one(JsonFormat::new().pretty(true), &record);
        assert_eq!(
            out,
            "{\n  \"level\": \"info\",\n  \"target\": \"t\",\n  \"message\": \"x\",\n  \"port\": 80\n}\n"
        );
    }
}

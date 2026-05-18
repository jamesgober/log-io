//! JSON output format.
//!
//! Emits one JSON object per record terminated by a newline. The
//! field ordering is stable so consumers can pattern-match on prefixes:
//! `timestamp` (if set) -> `level` -> `target` -> `message` -> source
//! location -> context fields -> structured fields.

use core::fmt;

use super::Format;
use crate::record::Record;
use crate::value::Value;

/// JSON formatter.
///
/// The default configuration emits compact JSON with a trailing
/// newline. Set [`Self::pretty`] for an indented form intended for
/// developer eyeballs, not machine ingestion.
///
/// # Example
///
/// ```
/// use log_io::format::{Format, JsonFormat};
/// use log_io::{Field, Level, Metadata, Record, Value};
///
/// let fmt = JsonFormat::new();
/// let fields = [Field::new("port", Value::U64(8080))];
/// let record = Record::new(Metadata::new(Level::Info, "app"), "started", &fields);
///
/// let mut out = String::new();
/// fmt.write_record(&record, &mut out).unwrap();
///
/// assert!(out.starts_with('{'));
/// assert!(out.ends_with("}\n"));
/// assert!(out.contains("\"port\":8080"));
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonFormat {
    pretty: bool,
    timestamp_rfc3339: bool,
}

impl JsonFormat {
    /// New compact JSON formatter. Equivalent to `Self::default()`.
    pub const fn new() -> Self {
        Self {
            pretty: false,
            timestamp_rfc3339: false,
        }
    }

    /// Toggle pretty (multi-line, indented) output.
    pub const fn pretty(mut self, enabled: bool) -> Self {
        self.pretty = enabled;
        self
    }

    /// Emit timestamps as RFC 3339 strings instead of unix nanoseconds.
    ///
    /// When enabled and a record's timestamp is present, the output
    /// looks like `"timestamp":"2026-05-18T13:24:01.123456789Z"`. When
    /// disabled (default), it is `"timestamp":1700000000000000000`.
    pub const fn timestamp_rfc3339(mut self, enabled: bool) -> Self {
        self.timestamp_rfc3339 = enabled;
        self
    }
}

impl Format for JsonFormat {
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        let nl = if self.pretty { "\n" } else { "" };
        let ind = if self.pretty { "  " } else { "" };
        let sp = if self.pretty { " " } else { "" };

        writer.write_char('{')?;
        writer.write_str(nl)?;

        let mut first = true;

        if let Some(ts) = record.metadata.timestamp_unix_nanos {
            comma(writer, &mut first, nl, ind)?;
            writer.write_str("\"timestamp\":")?;
            writer.write_str(sp)?;
            if self.timestamp_rfc3339 {
                writer.write_char('"')?;
                crate::format::rfc3339::write_rfc3339_nanos(ts, writer)?;
                writer.write_char('"')?;
            } else {
                write!(writer, "{ts}")?;
            }
        }

        comma(writer, &mut first, nl, ind)?;
        writer.write_str("\"level\":")?;
        writer.write_str(sp)?;
        writer.write_char('"')?;
        writer.write_str(record.metadata.level.as_str())?;
        writer.write_char('"')?;

        comma(writer, &mut first, nl, ind)?;
        writer.write_str("\"target\":")?;
        writer.write_str(sp)?;
        write_json_str(writer, record.metadata.target)?;

        comma(writer, &mut first, nl, ind)?;
        writer.write_str("\"message\":")?;
        writer.write_str(sp)?;
        write_json_str(writer, record.message)?;

        if let Some(file) = record.metadata.file {
            comma(writer, &mut first, nl, ind)?;
            writer.write_str("\"file\":")?;
            writer.write_str(sp)?;
            write_json_str(writer, file)?;
        }
        if let Some(line) = record.metadata.line {
            comma(writer, &mut first, nl, ind)?;
            writer.write_str("\"line\":")?;
            writer.write_str(sp)?;
            write!(writer, "{line}")?;
        }

        for field in record.all_fields() {
            comma(writer, &mut first, nl, ind)?;
            write_json_str(writer, field.key)?;
            writer.write_char(':')?;
            writer.write_str(sp)?;
            write_json_value(writer, field.value)?;
        }

        if self.pretty {
            writer.write_char('\n')?;
        }
        writer.write_char('}')?;
        writer.write_char('\n')?;
        Ok(())
    }
}

fn comma<W: fmt::Write + ?Sized>(w: &mut W, first: &mut bool, nl: &str, ind: &str) -> fmt::Result {
    if *first {
        *first = false;
    } else {
        w.write_char(',')?;
        w.write_str(nl)?;
    }
    w.write_str(ind)?;
    Ok(())
}

fn write_json_str<W: fmt::Write + ?Sized>(w: &mut W, s: &str) -> fmt::Result {
    w.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => w.write_str("\\\"")?,
            '\\' => w.write_str("\\\\")?,
            '\n' => w.write_str("\\n")?,
            '\r' => w.write_str("\\r")?,
            '\t' => w.write_str("\\t")?,
            '\x08' => w.write_str("\\b")?,
            '\x0c' => w.write_str("\\f")?,
            c if (c as u32) < 0x20 => {
                write!(w, "\\u{:04x}", c as u32)?;
            }
            c => w.write_char(c)?,
        }
    }
    w.write_char('"')
}

fn write_json_value<W: fmt::Write + ?Sized>(w: &mut W, v: Value<'_>) -> fmt::Result {
    match v {
        Value::Null => w.write_str("null"),
        Value::Bool(true) => w.write_str("true"),
        Value::Bool(false) => w.write_str("false"),
        Value::I64(n) => write!(w, "{n}"),
        Value::U64(n) => write!(w, "{n}"),
        Value::F64(n) => {
            if n.is_finite() {
                // Use Rust's default float formatting; it round-trips
                // and never emits trailing dots.
                write!(w, "{n}")
            } else {
                w.write_str("null")
            }
        }
        Value::Str(s) => write_json_str(w, s),
        Value::Char(c) => {
            // One-character string. Use the string path so escaping
            // applies (the char might be a quote or control byte).
            let mut buf = [0u8; 4];
            let s: &str = c.encode_utf8(&mut buf);
            write_json_str(w, s)
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::record::{Field, Metadata};
    use crate::Level;

    fn fmt_one(json: JsonFormat, record: &Record<'_>) -> String {
        let mut s = String::new();
        json.write_record(record, &mut s).unwrap();
        s
    }

    #[test]
    fn compact_object_with_trailing_newline() {
        let record = Record::new(Metadata::new(Level::Info, "app"), "hello", &[]);
        let out = fmt_one(JsonFormat::new(), &record);
        assert_eq!(
            out,
            "{\"level\":\"info\",\"target\":\"app\",\"message\":\"hello\"}\n"
        );
    }

    #[test]
    fn fields_appear_after_message() {
        let fields = [
            Field::new("port", Value::U64(8080)),
            Field::new("host", Value::Str("0.0.0.0")),
        ];
        let record = Record::new(Metadata::new(Level::Info, "app"), "up", &fields);
        let out = fmt_one(JsonFormat::new(), &record);
        let msg_pos = out.find("\"message\":\"up\"").unwrap();
        let port_pos = out.find("\"port\":8080").unwrap();
        assert!(msg_pos < port_pos);
        assert!(out.contains("\"host\":\"0.0.0.0\""));
    }

    #[test]
    fn context_appears_before_fields() {
        let ctx = [Field::new("trace_id", Value::Str("abc"))];
        let fields = [Field::new("port", Value::U64(80))];
        let record = Record::with_context(Metadata::new(Level::Info, "app"), "msg", &fields, &ctx);
        let out = fmt_one(JsonFormat::new(), &record);
        let trace_pos = out.find("\"trace_id\"").unwrap();
        let port_pos = out.find("\"port\"").unwrap();
        assert!(trace_pos < port_pos);
    }

    #[test]
    fn escapes_control_and_quotes() {
        let fields = [Field::new("data", Value::Str("a\"b\nc\td"))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "x", &fields);
        let out = fmt_one(JsonFormat::new(), &record);
        assert!(out.contains(r#""data":"a\"b\nc\td""#), "{out}");
    }

    #[test]
    fn escapes_low_codepoint_to_unicode() {
        let fields = [Field::new("data", Value::Str("\x01"))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "x", &fields);
        let out = fmt_one(JsonFormat::new(), &record);
        let backslash = "\\";
        let expected = format!("\"data\":\"{}u0001\"", backslash);
        assert!(out.contains(&expected), "{}", out);
    }

    #[test]
    fn nan_inf_become_null() {
        let fields = [
            Field::new("a", Value::F64(f64::NAN)),
            Field::new("b", Value::F64(f64::INFINITY)),
            Field::new("c", Value::F64(1.5)),
        ];
        let record = Record::new(Metadata::new(Level::Info, "t"), "x", &fields);
        let out = fmt_one(JsonFormat::new(), &record);
        assert!(out.contains("\"a\":null"));
        assert!(out.contains("\"b\":null"));
        assert!(out.contains("\"c\":1.5"));
    }

    #[test]
    fn timestamp_unix_nanos_default() {
        let metadata = Metadata::new(Level::Info, "t").with_timestamp(1_700_000_000_000_000_000);
        let record = Record::new(metadata, "x", &[]);
        let out = fmt_one(JsonFormat::new(), &record);
        assert!(out.contains("\"timestamp\":1700000000000000000"));
    }

    #[test]
    fn timestamp_rfc3339_when_enabled() {
        let metadata = Metadata::new(Level::Info, "t").with_timestamp(1_700_000_000_000_000_000);
        let record = Record::new(metadata, "x", &[]);
        let out = fmt_one(JsonFormat::new().timestamp_rfc3339(true), &record);
        assert!(out.contains("\"timestamp\":\"2023-11-14T22:13:20"), "{out}");
    }

    #[test]
    fn pretty_indents_and_breaks_lines() {
        let fields = [Field::new("port", Value::U64(80))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "x", &fields);
        let out = fmt_one(JsonFormat::new().pretty(true), &record);
        assert!(out.contains("\n  \"level\": \"info\""), "{out}");
        assert!(out.ends_with("}\n"));
    }
}

//! logfmt output format.
//!
//! Emits `key=value` pairs separated by single spaces and terminated
//! by a newline. The format follows the conventions popularized by
//! Heroku and Brandur: values containing whitespace, `=`, or `"` are
//! double-quoted with `\"` and `\\` escapes; control characters are
//! C-escaped; bare alphanumerics are emitted without quotes.

use core::fmt;

use super::Format;
use crate::record::Record;
use crate::value::Value;

/// logfmt formatter. See module docs for the wire format.
///
/// # Example
///
/// ```
/// use log_io::format::{Format, LogfmtFormat};
/// use log_io::{Field, Level, Metadata, Record, Value};
///
/// let fmt = LogfmtFormat::new();
/// let fields = [Field::new("port", Value::U64(8080))];
/// let record = Record::new(Metadata::new(Level::Info, "app"), "ok", &fields);
///
/// let mut out = String::new();
/// fmt.write_record(&record, &mut out).unwrap();
/// assert!(out.contains("port=8080"));
/// assert!(out.ends_with('\n'));
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct LogfmtFormat {
    _private: (),
}

impl LogfmtFormat {
    /// New logfmt formatter.
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

impl Format for LogfmtFormat {
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        let mut first = true;
        if let Some(ts) = record.metadata.timestamp_unix_nanos {
            kv_prefix(writer, &mut first)?;
            write!(writer, "timestamp={ts}")?;
        }
        kv_prefix(writer, &mut first)?;
        write!(writer, "level={}", record.metadata.level.as_str())?;
        kv_prefix(writer, &mut first)?;
        writer.write_str("target=")?;
        write_logfmt_str(writer, record.metadata.target)?;
        kv_prefix(writer, &mut first)?;
        writer.write_str("message=")?;
        write_logfmt_str(writer, record.message)?;
        if let Some(file) = record.metadata.file {
            kv_prefix(writer, &mut first)?;
            writer.write_str("file=")?;
            write_logfmt_str(writer, file)?;
        }
        if let Some(line) = record.metadata.line {
            kv_prefix(writer, &mut first)?;
            write!(writer, "line={line}")?;
        }
        for field in record.all_fields() {
            kv_prefix(writer, &mut first)?;
            write_logfmt_key(writer, field.key)?;
            writer.write_char('=')?;
            write_logfmt_value(writer, field.value)?;
        }
        writer.write_char('\n')
    }
}

fn kv_prefix<W: fmt::Write + ?Sized>(w: &mut W, first: &mut bool) -> fmt::Result {
    if *first {
        *first = false;
        Ok(())
    } else {
        w.write_char(' ')
    }
}

fn needs_quotes(s: &str) -> bool {
    s.is_empty()
        || s.chars()
            .any(|c| c.is_whitespace() || matches!(c, '=' | '"' | '\\') || (c as u32) < 0x20)
}

fn write_logfmt_str<W: fmt::Write + ?Sized>(w: &mut W, s: &str) -> fmt::Result {
    if needs_quotes(s) {
        w.write_char('"')?;
        for c in s.chars() {
            match c {
                '"' => w.write_str("\\\"")?,
                '\\' => w.write_str("\\\\")?,
                '\n' => w.write_str("\\n")?,
                '\r' => w.write_str("\\r")?,
                '\t' => w.write_str("\\t")?,
                c if (c as u32) < 0x20 => write!(w, "\\x{:02x}", c as u32)?,
                c => w.write_char(c)?,
            }
        }
        w.write_char('"')
    } else {
        w.write_str(s)
    }
}

fn write_logfmt_key<W: fmt::Write + ?Sized>(w: &mut W, key: &str) -> fmt::Result {
    // Keys with `=` or whitespace would break parsing. Replace with `_`
    // rather than quote, because logfmt parsers never quote keys.
    for c in key.chars() {
        if c.is_whitespace() || c == '=' || c == '"' {
            w.write_char('_')?;
        } else {
            w.write_char(c)?;
        }
    }
    Ok(())
}

fn write_logfmt_value<W: fmt::Write + ?Sized>(w: &mut W, v: Value<'_>) -> fmt::Result {
    match v {
        Value::Null => Ok(()),
        Value::Bool(true) => w.write_str("true"),
        Value::Bool(false) => w.write_str("false"),
        Value::I64(n) => write!(w, "{n}"),
        Value::U64(n) => write!(w, "{n}"),
        Value::F64(n) => write!(w, "{n}"),
        Value::Str(s) => write_logfmt_str(w, s),
        Value::Char(c) => {
            let mut buf = [0u8; 4];
            let s: &str = c.encode_utf8(&mut buf);
            write_logfmt_str(w, s)
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::record::{Field, Metadata};
    use crate::Level;

    fn fmt_one(record: &Record<'_>) -> String {
        let mut s = String::new();
        LogfmtFormat::new().write_record(record, &mut s).unwrap();
        s
    }

    #[test]
    fn simple_record_format() {
        let record = Record::new(Metadata::new(Level::Info, "app"), "hello", &[]);
        let out = fmt_one(&record);
        assert_eq!(out, "level=info target=app message=hello\n");
    }

    #[test]
    fn message_with_spaces_is_quoted() {
        let record = Record::new(Metadata::new(Level::Info, "app"), "hi there", &[]);
        let out = fmt_one(&record);
        assert!(out.contains(r#"message="hi there""#), "{out}");
    }

    #[test]
    fn null_value_emits_empty() {
        let fields = [Field::new("x", Value::Null)];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(&record);
        assert!(out.contains("x="));
        assert!(out.ends_with("x=\n"));
    }

    #[test]
    fn fields_after_message() {
        let fields = [Field::new("port", Value::U64(80))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(&record);
        let msg_pos = out.find("message=").unwrap();
        let port_pos = out.find("port=").unwrap();
        assert!(msg_pos < port_pos);
    }

    #[test]
    fn quotes_in_value_are_escaped() {
        let fields = [Field::new("data", Value::Str("a\"b"))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(&record);
        assert!(out.contains(r#"data="a\"b""#), "{out}");
    }

    #[test]
    fn key_whitespace_replaced() {
        let fields = [Field::new("the key", Value::U64(1))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(&record);
        assert!(out.contains("the_key=1"), "{out}");
    }
}

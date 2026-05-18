//! Human-readable output format.
//!
//! A single-line, fixed-column format optimized for tailing logs in a
//! terminal. Layout:
//!
//! ```text
//! TIMESTAMP LEVEL [target] message key=value key=value
//! ```
//!
//! Timestamps are RFC 3339 when present. Levels are upper-case and
//! space-padded to a fixed width.

use core::fmt;

use super::Format;
use crate::record::Record;
use crate::value::Value;

/// Human-readable formatter.
///
/// # Example
///
/// ```
/// use log_io::format::{Format, HumanFormat};
/// use log_io::{Field, Level, Metadata, Record, Value};
///
/// let fmt = HumanFormat::new();
/// let fields = [Field::new("port", Value::U64(8080))];
/// let record = Record::new(Metadata::new(Level::Info, "app"), "ok", &fields);
///
/// let mut out = String::new();
/// fmt.write_record(&record, &mut out).unwrap();
/// assert!(out.contains("INFO "));
/// assert!(out.contains("[app]"));
/// assert!(out.ends_with('\n'));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct HumanFormat {
    show_target: bool,
    show_source_location: bool,
}

impl Default for HumanFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl HumanFormat {
    /// New formatter with sensible defaults: target shown, source
    /// location hidden.
    pub const fn new() -> Self {
        Self {
            show_target: true,
            show_source_location: false,
        }
    }

    /// Toggle the `[target]` column.
    pub const fn show_target(mut self, enabled: bool) -> Self {
        self.show_target = enabled;
        self
    }

    /// Toggle inclusion of the `file:line` suffix when the record
    /// carries source location metadata.
    pub const fn show_source_location(mut self, enabled: bool) -> Self {
        self.show_source_location = enabled;
        self
    }
}

impl Format for HumanFormat {
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        if let Some(ts) = record.metadata.timestamp_unix_nanos {
            crate::format::rfc3339::write_rfc3339_nanos(ts, writer)?;
            writer.write_char(' ')?;
        }
        // 5 characters wide to align across all levels.
        write!(writer, "{:<5} ", record.metadata.level.as_str_upper())?;
        if self.show_target {
            writer.write_char('[')?;
            write_line_safe(writer, record.metadata.target)?;
            writer.write_str("] ")?;
        }
        write_line_safe(writer, record.message)?;
        for field in record.all_fields() {
            writer.write_char(' ')?;
            write_line_safe(writer, field.key)?;
            writer.write_char('=')?;
            write_value(writer, field.value)?;
        }
        if self.show_source_location {
            if let (Some(file), Some(line)) = (record.metadata.file, record.metadata.line) {
                write!(writer, " ({file}:{line})")?;
            }
        }
        writer.write_char('\n')
    }
}

/// Write `s` without ever emitting a literal newline, tab, or CR.
/// These are escaped with `\n` / `\t` / `\r` so the human format
/// remains one record per line.
fn write_line_safe<W: fmt::Write + ?Sized>(w: &mut W, s: &str) -> fmt::Result {
    for c in s.chars() {
        match c {
            '\n' => w.write_str("\\n")?,
            '\r' => w.write_str("\\r")?,
            '\t' => w.write_str("\\t")?,
            c => w.write_char(c)?,
        }
    }
    Ok(())
}

fn write_value<W: fmt::Write + ?Sized>(w: &mut W, v: Value<'_>) -> fmt::Result {
    match v {
        Value::Null => w.write_str("<null>"),
        Value::Bool(b) => write!(w, "{b}"),
        Value::I64(n) => write!(w, "{n}"),
        Value::U64(n) => write!(w, "{n}"),
        Value::F64(n) => write!(w, "{n}"),
        Value::Str(s) => {
            let needs_quote = s
                .chars()
                .any(|c| c.is_whitespace() || c == '"' || (c as u32) < 0x20);
            if needs_quote {
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
        Value::Char(c) => write!(w, "{c}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Field, Metadata};
    use crate::Level;

    fn fmt_one(fmt: HumanFormat, record: &Record<'_>) -> String {
        let mut s = String::new();
        fmt.write_record(record, &mut s).unwrap();
        s
    }

    #[test]
    fn level_is_uppercase_padded() {
        let record = Record::new(Metadata::new(Level::Info, "app"), "hi", &[]);
        let out = fmt_one(HumanFormat::new(), &record);
        assert!(out.starts_with("INFO  ["));
    }

    #[test]
    fn target_can_be_hidden() {
        let record = Record::new(Metadata::new(Level::Warn, "app"), "hi", &[]);
        let out = fmt_one(HumanFormat::new().show_target(false), &record);
        assert!(!out.contains('['));
        assert!(out.contains("WARN  hi"));
    }

    #[test]
    fn fields_appended_to_line() {
        let fields = [Field::new("port", Value::U64(80))];
        let record = Record::new(Metadata::new(Level::Info, "app"), "ok", &fields);
        let out = fmt_one(HumanFormat::new(), &record);
        assert!(out.contains("ok port=80"));
    }

    #[test]
    fn source_location_optional() {
        let metadata = Metadata::new(Level::Info, "app").with_location("src/lib.rs", 42);
        let record = Record::new(metadata, "ok", &[]);
        let with = fmt_one(HumanFormat::new().show_source_location(true), &record);
        let without = fmt_one(HumanFormat::new(), &record);
        assert!(with.contains("(src/lib.rs:42)"), "{with}");
        assert!(!without.contains("src/lib.rs"));
    }

    #[test]
    fn timestamp_prepended_when_set() {
        let metadata = Metadata::new(Level::Info, "app").with_timestamp(1_700_000_000_000_000_000);
        let record = Record::new(metadata, "ok", &[]);
        let out = fmt_one(HumanFormat::new(), &record);
        assert!(out.starts_with("2023-11-14T"), "{out}");
    }

    #[test]
    fn null_value_renders_as_placeholder() {
        let fields = [Field::new("x", Value::Null)];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(HumanFormat::new(), &record);
        assert!(out.contains("x=<null>"));
    }
}

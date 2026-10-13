//! Human-readable output format.
//!
//! A single-line format for reading logs in a terminal:
//!
//! ```text
//! TIMESTAMP LEVEL [target] message key=value key="quoted value"
//! ```
//!
//! Timestamps are RFC 3339 when present. Levels are upper-case and
//! padded to five columns.

use core::fmt;
use core::fmt::Write as _;

use super::Format;
use super::escape::{HumanEscaper, QuoteScan, value_needs_quotes, write_human_escaped};
use crate::level::Level;
use crate::record::Record;
use crate::value::{Value, write_f64, write_hex};

/// Human-readable formatter.
///
/// Message and target are written as-is except that control
/// characters, DEL, C1 controls, U+2028/U+2029 and bidi controls are
/// escaped (`\n`, `\x1b`, `\u{85}`, ...), so hostile input can neither
/// break the one-record-per-line layout nor drive the terminal. Values
/// are quoted when they are empty or contain whitespace, `=`, `"`, `\`
/// or a special character. Keys are sanitized: whitespace, `=`, `"` and
/// special characters become `_`.
///
/// Backslashes in unquoted text are not escaped, so a message that
/// literally contains `\n` renders the same as one containing a
/// newline. The format is meant for people; use JSON or logfmt where
/// output must be parsed back exactly.
///
/// Colour is off by default. [`HumanFormat::color`] turns ANSI colour
/// on for the level column (and dims the target); the
/// `LoggerBuilder::stdout_human` / `stderr_human` shortcuts enable it
/// automatically when the stream is a terminal (see `ColorChoice`, available
/// with the `std` feature).
///
/// # Examples
///
/// ```
/// use log_io::format::{Format, HumanFormat};
/// use log_io::{Field, Level, Metadata, Record};
///
/// let fields = [Field::new("port", 8080_u32), Field::new("note", "two words")];
/// let record = Record::new(Metadata::new(Level::Info, "app"), "ok\x1b[2J", &fields);
///
/// let mut out = String::new();
/// HumanFormat::new().write_record(&record, &mut out)?;
/// assert_eq!(out, "INFO  [app] ok\\x1b[2J port=8080 note=\"two words\"\n");
/// # Ok::<(), core::fmt::Error>(())
/// ```
#[derive(Debug, Clone, Copy)]
pub struct HumanFormat {
    show_target: bool,
    show_source_location: bool,
    color: bool,
}

impl Default for HumanFormat {
    fn default() -> Self {
        Self::new()
    }
}

impl HumanFormat {
    /// New formatter: target shown, source location hidden, no colour.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            show_target: true,
            show_source_location: false,
            color: false,
        }
    }

    /// Toggle the `[target]` column. The column is always omitted for
    /// records with an empty target.
    #[must_use]
    pub const fn show_target(mut self, enabled: bool) -> Self {
        self.show_target = enabled;
        self
    }

    /// Toggle the ` (file:line)` suffix for records that carry a source
    /// location.
    #[must_use]
    pub const fn show_source_location(mut self, enabled: bool) -> Self {
        self.show_source_location = enabled;
        self
    }

    /// Toggle ANSI colour: the level is coloured (ERROR red, WARN
    /// yellow, INFO green, DEBUG blue, TRACE magenta) and the target is
    /// dimmed. Only the formatter emits escape sequences; user text is
    /// always escaped, so it cannot add its own.
    #[must_use]
    pub const fn color(mut self, enabled: bool) -> Self {
        self.color = enabled;
        self
    }

    /// Whether ANSI colour is enabled.
    #[must_use]
    pub const fn is_color(&self) -> bool {
        self.color
    }

    fn write_impl<W: fmt::Write + ?Sized>(&self, record: &Record<'_>, w: &mut W) -> fmt::Result {
        if let Some(ts) = record.metadata.timestamp_unix_nanos {
            super::rfc3339::write_rfc3339_nanos(ts, w)?;
            w.write_char(' ')?;
        }
        let level = record.metadata.level;
        if self.color {
            w.write_str(level_color(level))?;
            write!(w, "{:<5}", level.as_str_upper())?;
            w.write_str(RESET)?;
            w.write_char(' ')?;
        } else {
            write!(w, "{:<5} ", level.as_str_upper())?;
        }
        if self.show_target && !record.metadata.target.is_empty() {
            if self.color {
                w.write_str(DIM)?;
            }
            w.write_char('[')?;
            write_human_escaped(w, record.metadata.target, false)?;
            w.write_char(']')?;
            if self.color {
                w.write_str(RESET)?;
            }
            w.write_char(' ')?;
        }
        write_human_escaped(w, record.message, false)?;
        for field in record.all_fields() {
            w.write_char(' ')?;
            super::escape::write_sanitized_key(w, field.key)?;
            w.write_char('=')?;
            write_value(w, field.value)?;
        }
        if self.show_source_location {
            if let (Some(file), Some(line)) = (record.metadata.file, record.metadata.line) {
                w.write_str(" (")?;
                write_human_escaped(w, file, false)?;
                write!(w, ":{line})")?;
            }
        }
        w.write_char('\n')
    }
}

impl Format for HumanFormat {
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

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

const fn level_color(level: Level) -> &'static str {
    match level {
        Level::Error => "\x1b[31m",
        Level::Warn => "\x1b[33m",
        Level::Info => "\x1b[32m",
        Level::Debug => "\x1b[34m",
        Level::Trace => "\x1b[35m",
        Level::Off => "",
    }
}

#[inline]
fn write_str_value<W: fmt::Write + ?Sized>(w: &mut W, s: &str) -> fmt::Result {
    if value_needs_quotes(s) {
        w.write_char('"')?;
        write_human_escaped(w, s, true)?;
        w.write_char('"')
    } else {
        w.write_str(s)
    }
}

fn write_value<W: fmt::Write + ?Sized>(w: &mut W, v: Value<'_>) -> fmt::Result {
    match v {
        Value::Null => w.write_str("<null>"),
        Value::Bool(b) => write!(w, "{b}"),
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
            let quoted = QuoteScan::needs_quotes(&lazy);
            if quoted {
                w.write_char('"')?;
            }
            write!(
                HumanEscaper {
                    inner: &mut *w,
                    quoted,
                },
                "{lazy}"
            )?;
            if quoted {
                w.write_char('"')?;
            }
            Ok(())
        }
    }
}

/// When to colour [`HumanFormat`] output.
///
/// Resolved once, when a sink is built, into a plain on/off setting
/// for [`HumanFormat::color`].
///
/// # Examples
///
/// ```
/// use log_io::format::{ColorChoice, HumanFormat};
///
/// let fmt = HumanFormat::new().color(ColorChoice::Auto.enabled_for_stderr());
/// # let _ = fmt;
/// assert!(ColorChoice::Always.resolve(false));
/// assert!(!ColorChoice::Never.resolve(true));
/// ```
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ColorChoice {
    /// Colour when the destination is a terminal, honouring the
    /// conventional environment variables:
    ///
    /// * `NO_COLOR` set to a non-empty value disables colour;
    /// * `CLICOLOR_FORCE` set to a non-empty value other than `0`
    ///   enables colour even when not a terminal;
    /// * `TERM=dumb` disables colour.
    ///
    /// On Windows, colour is only enabled automatically inside
    /// terminals known to interpret ANSI sequences without extra setup
    /// (Windows Terminal, ConEmu, or any environment that sets `TERM`,
    /// such as Git Bash or MSYS2). The classic console host needs the
    /// virtual-terminal mode switched on through the Windows API, which
    /// this crate does not do because it forbids `unsafe` code; use
    /// [`ColorChoice::Always`] if your program enables that mode itself.
    #[default]
    Auto,
    /// Always colour.
    Always,
    /// Never colour.
    Never,
}

#[cfg(feature = "std")]
impl ColorChoice {
    /// Resolve against an explicit "is the destination a terminal"
    /// answer and the process environment.
    #[must_use]
    pub fn resolve(self, is_terminal: bool) -> bool {
        self.resolve_with(is_terminal, cfg!(windows), |key| std::env::var_os(key))
    }

    /// Resolve for the process's standard output.
    #[must_use]
    pub fn enabled_for_stdout(self) -> bool {
        use std::io::IsTerminal;
        self.resolve(std::io::stdout().is_terminal())
    }

    /// Resolve for the process's standard error.
    #[must_use]
    pub fn enabled_for_stderr(self) -> bool {
        use std::io::IsTerminal;
        self.resolve(std::io::stderr().is_terminal())
    }

    fn resolve_with(
        self,
        is_terminal: bool,
        windows: bool,
        env: impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> bool {
        let set = |key: &str| env(key).is_some_and(|v| !v.is_empty());
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => {
                if set("NO_COLOR") {
                    return false;
                }
                if env("CLICOLOR_FORCE").is_some_and(|v| !v.is_empty() && v != "0") {
                    return true;
                }
                if !is_terminal || env("TERM").is_some_and(|t| t == "dumb") {
                    return false;
                }
                if windows {
                    set("WT_SESSION") || set("TERM") || env("ConEmuANSI").is_some_and(|v| v == "ON")
                } else {
                    true
                }
            }
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::record::{Field, Metadata};

    fn fmt_one(fmt: HumanFormat, record: &Record<'_>) -> String {
        let mut s = String::new();
        fmt.write_record_generic(record, &mut s).unwrap();
        let mut d = String::new();
        fmt.write_record(record, &mut d).unwrap();
        assert_eq!(s, d);
        s
    }

    fn field_human(v: Value<'_>) -> String {
        let fields = [Field::new("v", v)];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = fmt_one(HumanFormat::new(), &record);
        let start = out.find(" v=").unwrap() + 3;
        out[start..out.len() - 1].to_string()
    }

    #[test]
    fn test_human_level_is_uppercase_padded() {
        let record = Record::new(Metadata::new(Level::Info, "app"), "hi", &[]);
        assert_eq!(fmt_one(HumanFormat::new(), &record), "INFO  [app] hi\n");
        let record = Record::new(Metadata::new(Level::Error, "app"), "hi", &[]);
        assert_eq!(fmt_one(HumanFormat::new(), &record), "ERROR [app] hi\n");
    }

    #[test]
    fn test_human_empty_target_omits_brackets() {
        let record = Record::new(Metadata::new(Level::Info, ""), "hi", &[]);
        assert_eq!(
            fmt_one(HumanFormat::new(), &record),
            "INFO  hi
"
        );
    }

    #[test]
    fn test_human_target_can_be_hidden() {
        let record = Record::new(Metadata::new(Level::Warn, "app"), "hi", &[]);
        assert_eq!(
            fmt_one(HumanFormat::new().show_target(false), &record),
            "WARN  hi\n"
        );
    }

    #[test]
    fn test_human_source_location_optional() {
        let metadata = Metadata::new(Level::Info, "app").with_location("src/lib.rs", 42);
        let record = Record::new(metadata, "ok", &[]);
        let with = fmt_one(HumanFormat::new().show_source_location(true), &record);
        let without = fmt_one(HumanFormat::new(), &record);
        assert_eq!(with, "INFO  [app] ok (src/lib.rs:42)\n");
        assert_eq!(without, "INFO  [app] ok\n");
    }

    #[test]
    fn test_human_timestamp_prepended_when_set() {
        let metadata = Metadata::new(Level::Info, "app").with_timestamp(1_700_000_000_000_000_000);
        let record = Record::new(metadata, "ok", &[]);
        assert_eq!(
            fmt_one(HumanFormat::new(), &record),
            "2023-11-14T22:13:20.000000000Z INFO  [app] ok\n"
        );
    }

    #[test]
    fn test_human_values_render_per_variant() {
        assert_eq!(field_human(Value::Null), "<null>");
        assert_eq!(field_human(Value::Bool(false)), "false");
        assert_eq!(field_human(Value::I64(-4)), "-4");
        assert_eq!(field_human(Value::U64(4)), "4");
        assert_eq!(field_human(Value::I128(-5)), "-5");
        assert_eq!(field_human(Value::U128(5)), "5");
        assert_eq!(field_human(Value::F64(2.0)), "2.0");
        assert_eq!(field_human(Value::F64(f64::NAN)), "NaN");
        assert_eq!(field_human(Value::Str("")), "\"\"");
        assert_eq!(field_human(Value::Str("plain")), "plain");
        assert_eq!(field_human(Value::Str("two words")), "\"two words\"");
        assert_eq!(field_human(Value::Str("a\"b")), "\"a\\\"b\"");
        assert_eq!(field_human(Value::Str("a\\b c")), "\"a\\\\b c\"");
        assert_eq!(field_human(Value::Str("a=b")), "\"a=b\"");
        assert_eq!(field_human(Value::Str("x\u{1b}y")), "\"x\\x1by\"");
        assert_eq!(field_human(Value::Char('\n')), "\"\\n\"");
        assert_eq!(field_human(Value::Char('z')), "z");
        assert_eq!(field_human(Value::Bytes(&[])), "\"\"");
        assert_eq!(field_human(Value::Bytes(&[0xab])), "ab");
        assert_eq!(field_human(Value::Display(&7)), "7");
        assert_eq!(field_human(Value::Display(&"a b")), "\"a b\"");
        assert_eq!(field_human(Value::Debug(&"q")), "\"\\\"q\\\"\"");
        let e = std::io::Error::other("bad\nthing");
        assert_eq!(field_human(Value::Error(&e)), "\"bad\\nthing\"");
    }

    #[test]
    fn test_human_injection_is_neutralized() {
        // The audit's R04 reproduction.
        let fields = [
            Field::new("c", Value::Char('\n')),
            Field::new("k\u{1b}[2J", 1_u32),
        ];
        let record = Record::new(
            Metadata::new(Level::Info, "t\u{1b}]0;pwn\u{7}"),
            "msg \u{1b}[31mRED\u{b}VT\u{2028}LS\u{85}NEL",
            &fields,
        );
        let out = fmt_one(HumanFormat::new(), &record);
        assert_eq!(out.matches('\n').count(), 1, "{out:?}");
        assert!(!out.contains('\u{1b}'), "{out:?}");
        assert!(!out.contains('\u{7}'), "{out:?}");
        assert!(!out.contains('\u{2028}'), "{out:?}");
        assert!(!out.contains('\u{85}'), "{out:?}");
        assert_eq!(
            out,
            "INFO  [t\\x1b]0;pwn\\x07] msg \\x1b[31mRED\\x0bVT\\u{2028}LS\\u{85}NEL \
             c=\"\\n\" k_[2J=1\n"
        );
    }

    #[test]
    fn test_human_color_wraps_level_and_target() {
        let record = Record::new(Metadata::new(Level::Error, "app"), "x", &[]);
        let out = fmt_one(HumanFormat::new().color(true), &record);
        assert_eq!(out, "\x1b[31mERROR\x1b[0m \x1b[2m[app]\x1b[0m x\n");
        assert!(HumanFormat::new().color(true).is_color());
        assert!(!HumanFormat::new().is_color());
        for (level, code) in [
            (Level::Warn, "\x1b[33m"),
            (Level::Info, "\x1b[32m"),
            (Level::Debug, "\x1b[34m"),
            (Level::Trace, "\x1b[35m"),
        ] {
            let record = Record::new(Metadata::new(level, "a"), "x", &[]);
            let out = fmt_one(HumanFormat::new().color(true).show_target(false), &record);
            assert!(out.starts_with(code), "{out:?}");
        }
    }

    fn env_of(
        vars: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    }

    #[test]
    fn test_color_choice_explicit_choices() {
        assert!(ColorChoice::Always.resolve_with(false, false, env_of(&[])));
        assert!(!ColorChoice::Never.resolve_with(true, false, env_of(&[])));
    }

    #[test]
    fn test_color_choice_auto_unix_rules() {
        let auto = ColorChoice::Auto;
        assert!(auto.resolve_with(true, false, env_of(&[])));
        assert!(!auto.resolve_with(false, false, env_of(&[])));
        assert!(!auto.resolve_with(true, false, env_of(&[("NO_COLOR", "1")])));
        assert!(auto.resolve_with(true, false, env_of(&[("NO_COLOR", "")])));
        assert!(auto.resolve_with(false, false, env_of(&[("CLICOLOR_FORCE", "1")])));
        assert!(!auto.resolve_with(false, false, env_of(&[("CLICOLOR_FORCE", "0")])));
        assert!(!auto.resolve_with(
            false,
            false,
            env_of(&[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")])
        ));
        assert!(!auto.resolve_with(true, false, env_of(&[("TERM", "dumb")])));
    }

    #[test]
    fn test_color_choice_auto_windows_rules() {
        let auto = ColorChoice::Auto;
        assert!(!auto.resolve_with(true, true, env_of(&[])));
        assert!(auto.resolve_with(true, true, env_of(&[("WT_SESSION", "abc")])));
        assert!(auto.resolve_with(true, true, env_of(&[("TERM", "xterm-256color")])));
        assert!(auto.resolve_with(true, true, env_of(&[("ConEmuANSI", "ON")])));
        assert!(!auto.resolve_with(false, true, env_of(&[("WT_SESSION", "abc")])));
    }

    #[test]
    fn test_color_choice_default_is_auto() {
        assert_eq!(ColorChoice::default(), ColorChoice::Auto);
        // Smoke-test the process-environment entry points.
        let _stdout = ColorChoice::Never.enabled_for_stdout();
        let _stderr = ColorChoice::Never.enabled_for_stderr();
        assert!(!ColorChoice::Never.resolve(true));
    }
}

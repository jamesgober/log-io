//! Escaping shared by the built-in formats.
//!
//! Every format applies the same policy to untrusted text (messages,
//! targets, keys, string values): any character that can alter
//! terminal state or the line structure of the output is escaped. That
//! set is:
//!
//! * C0 controls (U+0000..=U+001F), including `\n`, `\r`, `\t` and ESC;
//! * DEL (U+007F);
//! * C1 controls (U+0080..=U+009F), including NEL (U+0085) and the
//!   8-bit CSI (U+009B);
//! * the Unicode line and paragraph separators (U+2028, U+2029);
//! * the bidirectional embedding, override and isolate controls
//!   (U+202A..=U+202E, U+2066..=U+2069) used in "Trojan Source" style
//!   attacks.
//!
//! The scanner works on bytes. Everything special is either a single
//! ASCII byte or a multi-byte sequence starting with `0xC2` or `0xE2`,
//! so clean text is copied in contiguous runs with a single table
//! lookup per byte.

use core::fmt;

/// Mark C0, DEL, the two lead bytes that can start a special
/// multi-byte sequence, and any format-specific ASCII bytes.
const fn build_table(extra: &[u8]) -> [bool; 256] {
    let mut t = [false; 256];
    let mut i = 0;
    while i < 0x20 {
        t[i] = true;
        i += 1;
    }
    t[0x7F] = true;
    t[0xC2] = true;
    t[0xE2] = true;
    let mut j = 0;
    while j < extra.len() {
        t[extra[j] as usize] = true;
        j += 1;
    }
    t
}

/// Bytes that start an escape inside JSON strings and quoted logfmt or
/// human values: controls plus `"` and `\`.
static QUOTED: [bool; 256] = build_table(b"\"\\");

/// Bytes that start an escape in unquoted human text (message, target):
/// controls only. Quotes and backslashes are left as-is.
#[cfg(feature = "human")]
static UNQUOTED: [bool; 256] = build_table(b"");

/// Length of the special multi-byte sequence starting at `i`, if any.
#[inline]
fn special_multibyte(bytes: &[u8], i: usize) -> Option<usize> {
    match bytes.get(i) {
        Some(0xC2) => match bytes.get(i + 1) {
            Some(0x80..=0x9F) => Some(2),
            _ => None,
        },
        Some(0xE2) => match (bytes.get(i + 1), bytes.get(i + 2)) {
            (Some(0x80), Some(0xA8..=0xAE)) | (Some(0x81), Some(0xA6..=0xA9)) => Some(3),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(any(feature = "logfmt", feature = "human"))]
/// Returns `true` for the non-ASCII characters the policy escapes.
#[inline]
pub(crate) const fn is_special_unicode(c: char) -> bool {
    matches!(c, '\u{80}'..='\u{9f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

#[cfg(any(feature = "logfmt", feature = "human"))]
/// Returns `true` if `c` must be escaped under the shared policy
/// (ignoring format-specific ASCII such as quotes).
#[inline]
pub(crate) const fn is_special(c: char) -> bool {
    (c as u32) < 0x20 || c == '\u{7f}' || is_special_unicode(c)
}

/// Copy `s` to `w`, routing every special character through `esc`.
#[inline]
fn escape_with<W, E>(w: &mut W, s: &str, table: &[bool; 256], mut esc: E) -> fmt::Result
where
    W: fmt::Write + ?Sized,
    E: FnMut(&mut W, char) -> fmt::Result,
{
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if !table[usize::from(b)] {
            i += 1;
            continue;
        }
        let len = if b < 0x80 {
            1
        } else {
            match special_multibyte(bytes, i) {
                Some(n) => n,
                None => {
                    i += 1;
                    continue;
                }
            }
        };
        // `i` is a char boundary: it is either an ASCII byte or a UTF-8
        // lead byte (0xC2 / 0xE2), and `start` always sits on one too.
        if let Some(run) = s.get(start..i) {
            w.write_str(run)?;
        }
        if let Some(c) = s.get(i..).and_then(|rest| rest.chars().next()) {
            esc(w, c)?;
        }
        i += len;
        start = i;
    }
    if let Some(run) = s.get(start..) {
        w.write_str(run)?;
    }
    Ok(())
}

#[cfg(any(feature = "json", feature = "logfmt"))]
/// JSON string escape for one special character.
fn json_escape_char<W: fmt::Write + ?Sized>(w: &mut W, c: char) -> fmt::Result {
    match c {
        '"' => w.write_str("\\\""),
        '\\' => w.write_str("\\\\"),
        '\n' => w.write_str("\\n"),
        '\r' => w.write_str("\\r"),
        '\t' => w.write_str("\\t"),
        '\u{08}' => w.write_str("\\b"),
        '\u{0c}' => w.write_str("\\f"),
        // Every other special character lives in the Basic Multilingual
        // Plane, so four hex digits always suffice.
        c => write!(w, "\\u{:04x}", u32::from(c)),
    }
}

#[cfg(feature = "human")]
/// Human-format escape for one special character: C-style for ASCII,
/// Rust-style `\u{..}` for the rest.
fn human_escape_char<W: fmt::Write + ?Sized>(w: &mut W, c: char) -> fmt::Result {
    match c {
        '"' => w.write_str("\\\""),
        '\\' => w.write_str("\\\\"),
        '\n' => w.write_str("\\n"),
        '\r' => w.write_str("\\r"),
        '\t' => w.write_str("\\t"),
        c if c.is_ascii() => write!(w, "\\x{:02x}", u32::from(c)),
        c => write!(w, "\\u{{{:x}}}", u32::from(c)),
    }
}

#[cfg(any(feature = "json", feature = "logfmt"))]
/// Write the body of a JSON string (no surrounding quotes). Also used
/// for quoted logfmt values: the escapes are JSON-compatible, which is
/// what common logfmt decoders (for example go-logfmt) accept.
#[inline]
pub(crate) fn write_json_escaped<W: fmt::Write + ?Sized>(w: &mut W, s: &str) -> fmt::Result {
    escape_with(w, s, &QUOTED, json_escape_char)
}

#[cfg(feature = "human")]
/// Write human-format text. With `quoted`, `"` and `\` are escaped as
/// well, for use between double quotes.
#[inline]
pub(crate) fn write_human_escaped<W: fmt::Write + ?Sized>(
    w: &mut W,
    s: &str,
    quoted: bool,
) -> fmt::Result {
    let table = if quoted { &QUOTED } else { &UNQUOTED };
    escape_with(w, s, table, human_escape_char)
}

#[cfg(any(feature = "logfmt", feature = "human"))]
/// Returns `true` if a logfmt or human value must be quoted: it is
/// empty, or contains whitespace, `=`, `"`, `\`, or a special
/// character.
#[inline]
pub(crate) fn value_needs_quotes(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    if s.is_ascii() {
        s.bytes()
            .any(|b| b <= b' ' || b == 0x7f || matches!(b, b'=' | b'"' | b'\\'))
    } else {
        s.chars()
            .any(|c| c.is_whitespace() || is_special(c) || matches!(c, '=' | '"' | '\\'))
    }
}

#[cfg(any(feature = "json", feature = "logfmt"))]
pub(crate) use crate::record::{RESERVED_PREFIX, is_reserved_key};

#[cfg(any(feature = "logfmt", feature = "human"))]
/// Write a key for logfmt or the human format. Characters that would
/// break `key=value` parsing (whitespace, `=`, `"`) and every special
/// character are replaced with `_`; an empty key becomes `_`.
pub(crate) fn write_sanitized_key<W: fmt::Write + ?Sized>(w: &mut W, key: &str) -> fmt::Result {
    if key.is_empty() {
        return w.write_char('_');
    }
    let clean = key
        .bytes()
        .all(|b| b > b' ' && b < 0x7f && b != b'=' && b != b'"');
    if clean {
        return w.write_str(key);
    }
    for c in key.chars() {
        if c.is_whitespace() || is_special(c) || c == '=' || c == '"' {
            w.write_char('_')?;
        } else {
            w.write_char(c)?;
        }
    }
    Ok(())
}

#[cfg(any(feature = "json", feature = "logfmt"))]
/// `fmt::Write` adapter that JSON-escapes everything written through it.
pub(crate) struct JsonEscaper<'w, W: ?Sized>(pub(crate) &'w mut W);

#[cfg(any(feature = "json", feature = "logfmt"))]
impl<W: fmt::Write + ?Sized> fmt::Write for JsonEscaper<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_json_escaped(self.0, s)
    }
}

#[cfg(feature = "human")]
/// `fmt::Write` adapter that applies the human-format escapes.
pub(crate) struct HumanEscaper<'w, W: ?Sized> {
    pub(crate) inner: &'w mut W,
    pub(crate) quoted: bool,
}

#[cfg(feature = "human")]
impl<W: fmt::Write + ?Sized> fmt::Write for HumanEscaper<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_human_escaped(self.inner, s, self.quoted)
    }
}

#[cfg(all(any(feature = "logfmt", feature = "human"), feature = "std"))]
std::thread_local! {
    /// Scratch buffer for [`render_once`].
    static RENDER_BUF: core::cell::RefCell<std::string::String> =
        const { core::cell::RefCell::new(std::string::String::new()) };
}

/// Capacity [`render_once`]'s scratch buffer may keep between values.
#[cfg(all(any(feature = "logfmt", feature = "human"), feature = "std"))]
const MAX_RENDER_CAPACITY: usize = 16 * 1024;

#[cfg(any(feature = "logfmt", feature = "human"))]
/// Render a lazily formatted value (`Display` / `Debug` / `Error`)
/// exactly once and pass the text to `write`.
///
/// logfmt and the human format decide on quoting from the text, then
/// write that same text. Up to 2.1 they rendered the value twice (once
/// to decide, once to write) and logfmt wrote the second render
/// unescaped when the first needed no quotes: a value that rendered
/// differently the second time could forge whole log lines.
///
/// With `std` the text goes into a thread-local buffer (a fresh one when
/// a value's `Display` itself logs). Without `std` it goes into a
/// 256-byte stack buffer; a longer value calls `write` with `None`, and
/// the caller then writes it quoted, streamed through the escaper.
pub(crate) fn render_once(
    value: &dyn fmt::Display,
    write: impl FnOnce(Option<&str>) -> fmt::Result,
) -> fmt::Result {
    #[cfg(feature = "std")]
    {
        if RENDER_BUF.try_with(|_| ()).is_ok() {
            RENDER_BUF.with(|cell| match cell.try_borrow_mut() {
                Ok(mut buf) => render_into(&mut buf, value, write),
                // A value's `Display` that logs: use a temporary buffer.
                Err(_) => render_into(&mut std::string::String::new(), value, write),
            })
        } else {
            // Thread-local storage is being torn down.
            render_into(&mut std::string::String::new(), value, write)
        }
    }
    #[cfg(not(feature = "std"))]
    {
        let mut buf = StackBuf {
            bytes: [0; STACK_RENDER],
            len: 0,
            overflow: false,
        };
        match fmt::write(&mut buf, format_args!("{value}")) {
            Ok(()) => match core::str::from_utf8(&buf.bytes[..buf.len]) {
                Ok(text) => write(Some(text)),
                Err(_) => write(None),
            },
            Err(_) if buf.overflow => write(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(all(any(feature = "logfmt", feature = "human"), feature = "std"))]
fn render_into(
    buf: &mut std::string::String,
    value: &dyn fmt::Display,
    write: impl FnOnce(Option<&str>) -> fmt::Result,
) -> fmt::Result {
    buf.clear();
    let result = match fmt::write(buf, format_args!("{value}")) {
        Ok(()) => write(Some(buf.as_str())),
        Err(e) => Err(e),
    };
    buf.clear();
    if buf.capacity() > MAX_RENDER_CAPACITY {
        buf.shrink_to(MAX_RENDER_CAPACITY);
    }
    result
}

#[cfg(all(any(feature = "logfmt", feature = "human"), not(feature = "std")))]
const STACK_RENDER: usize = 256;

#[cfg(all(any(feature = "logfmt", feature = "human"), not(feature = "std")))]
struct StackBuf {
    bytes: [u8; STACK_RENDER],
    len: usize,
    overflow: bool,
}

#[cfg(all(any(feature = "logfmt", feature = "human"), not(feature = "std")))]
impl fmt::Write for StackBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        match self.bytes.get_mut(self.len..end) {
            Some(slot) => {
                slot.copy_from_slice(s.as_bytes());
                self.len = end;
                Ok(())
            }
            None => {
                self.overflow = true;
                Err(fmt::Error)
            }
        }
    }
}

#[cfg(all(
    test,
    feature = "std",
    feature = "json",
    feature = "logfmt",
    feature = "human"
))]
mod tests {
    use super::*;

    fn json(s: &str) -> String {
        let mut out = String::new();
        write_json_escaped(&mut out, s).unwrap();
        out
    }

    fn human(s: &str, quoted: bool) -> String {
        let mut out = String::new();
        write_human_escaped(&mut out, s, quoted).unwrap();
        out
    }

    fn key(s: &str) -> String {
        let mut out = String::new();
        write_sanitized_key(&mut out, s).unwrap();
        out
    }

    #[test]
    fn test_json_escape_short_forms() {
        assert_eq!(json("\"\\\n\r\t\u{8}\u{c}"), r#"\"\\\n\r\t\b\f"#);
    }

    #[test]
    fn test_json_escape_controls_c1_and_separators() {
        assert_eq!(json("\u{1}"), "\\u0001");
        assert_eq!(json("\u{1b}[31m"), "\\u001b[31m");
        assert_eq!(json("\u{7f}"), "\\u007f");
        assert_eq!(json("\u{85}\u{9b}"), "\\u0085\\u009b");
        assert_eq!(json("a\u{2028}b\u{2029}c"), "a\\u2028b\\u2029c");
        assert_eq!(json("\u{202e}\u{2066}\u{2069}"), "\\u202e\\u2066\\u2069");
    }

    #[test]
    fn test_json_escape_preserves_clean_multibyte_text() {
        // 0xC2 / 0xE2 lead bytes that do not start a special sequence.
        let s = "\u{a9} \u{a0} \u{2014} \u{2026} \u{20ac} \u{2070} \u{6f22} \u{1f680}";
        assert_eq!(json(s), s);
    }

    #[test]
    fn test_json_escape_at_string_edges() {
        assert_eq!(json(""), "");
        assert_eq!(json("\n"), "\\n");
        assert_eq!(json("x\u{85}"), "x\\u0085");
        assert_eq!(json("\u{85}x"), "\\u0085x");
    }

    #[test]
    fn test_human_escape_unquoted_leaves_quotes_and_backslashes() {
        assert_eq!(human("a\"b\\c", false), "a\"b\\c");
        assert_eq!(human("a\nb", false), "a\\nb");
        assert_eq!(human("\u{1b}]0;x\u{7}", false), "\\x1b]0;x\\x07");
        assert_eq!(
            human("\u{85}\u{2028}\u{202e}", false),
            "\\u{85}\\u{2028}\\u{202e}"
        );
        assert_eq!(human("\u{7f}", false), "\\x7f");
    }

    #[test]
    fn test_human_escape_quoted_escapes_quotes_and_backslashes() {
        assert_eq!(human("a\"b\\c", true), "a\\\"b\\\\c");
        assert_eq!(human("tab\there", true), "tab\\there");
    }

    #[test]
    fn test_value_needs_quotes_rules() {
        assert!(value_needs_quotes(""));
        assert!(value_needs_quotes("a b"));
        assert!(value_needs_quotes("a=b"));
        assert!(value_needs_quotes("a\"b"));
        assert!(value_needs_quotes("a\\b"));
        assert!(value_needs_quotes("a\u{7f}"));
        assert!(value_needs_quotes("a\u{a0}b"));
        assert!(value_needs_quotes("a\u{85}"));
        assert!(value_needs_quotes("a\u{202e}"));
        assert!(!value_needs_quotes("plain"));
        assert!(!value_needs_quotes("caf\u{e9}"));
        assert!(!value_needs_quotes("/api/v1?x"));
    }

    #[test]
    fn test_sanitized_key_replaces_breaking_characters() {
        assert_eq!(key(""), "_");
        assert_eq!(key("plain.key"), "plain.key");
        assert_eq!(key("the key"), "the_key");
        assert_eq!(key("a=b"), "a_b");
        assert_eq!(key("a\"b"), "a_b");
        assert_eq!(key("k\u{1b}[0m"), "k_[0m");
        assert_eq!(key("k\u{9b}"), "k_");
        assert_eq!(key("caf\u{e9}"), "caf\u{e9}");
    }

    #[test]
    fn test_sanitized_key_rejects_keys_made_only_of_breaking_characters() {
        // Each of these must leave the clean-ASCII fast path; keys that
        // consist of nothing but one bad character exercise every
        // condition of that check on its own.
        for bad in [
            "\n", " ", "\u{7f}", "=", "\"", "\u{85}", "\u{202e}", "\u{1}",
        ] {
            let n = bad.chars().count();
            assert_eq!(key(bad), "_".repeat(n), "{bad:?}");
            assert_eq!(key(&bad.repeat(3)), "_".repeat(3 * n), "{bad:?}");
        }
        assert_eq!(key("!~"), "!~", "printable ASCII bounds stay clean");
    }

    #[test]
    fn test_reserved_keys() {
        for k in ["timestamp", "level", "target", "message", "file", "line"] {
            assert!(is_reserved_key(k));
        }
        assert!(!is_reserved_key("levels"));
        assert!(!is_reserved_key("Level"));
        assert!(!is_reserved_key(""));
    }

    #[test]
    fn test_render_once_passes_the_rendered_text() {
        let mut seen = String::new();
        render_once(&42, |text| {
            seen.push_str(text.unwrap_or("<none>"));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, "42");
    }

    #[test]
    fn test_render_once_is_reentrant() {
        struct Nested;
        impl fmt::Display for Nested {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // A `Display` that formats another lazy value, as one that
                // logs would.
                let mut inner = String::new();
                render_once(&"inner", |text| {
                    inner.push_str(text.unwrap_or(""));
                    Ok(())
                })?;
                f.write_str(&inner)
            }
        }
        let mut seen = String::new();
        render_once(&Nested, |text| {
            seen.push_str(text.unwrap_or("<none>"));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, "inner");
    }

    #[test]
    fn test_is_special_boundaries() {
        assert!(is_special('\u{0}'));
        assert!(is_special('\u{1f}'));
        assert!(!is_special(' '));
        assert!(is_special('\u{7f}'));
        assert!(is_special('\u{80}'));
        assert!(is_special('\u{9f}'));
        assert!(!is_special('\u{a0}'));
        assert!(!is_special('\u{2027}'));
        assert!(is_special('\u{2028}'));
        assert!(is_special('\u{202e}'));
        assert!(!is_special('\u{202f}'));
        assert!(!is_special('\u{2065}'));
        assert!(is_special('\u{2066}'));
        assert!(is_special('\u{2069}'));
        assert!(!is_special('\u{206a}'));
    }
}

//! Field values.
//!
//! [`Value`] is a borrowed sum type for the data attached to a log
//! [`crate::Field`]. Borrowing keeps the fast path allocation-free: a
//! record built from `Value`s lives on the stack and points at data the
//! caller already owns.

use core::fmt;

/// A field value attached to a log record.
///
/// Every variant borrows (or copies) its data, so constructing a
/// `Value` never allocates. The lazy variants ([`Self::Display`],
/// [`Self::Debug`], [`Self::Error`]) defer formatting until a sink
/// actually serializes the record, so a record rejected by the filter
/// never pays for them.
///
/// The enum is `#[non_exhaustive]`: new variants may be added in minor
/// releases, so `match` statements outside this crate need a wildcard
/// arm.
///
/// # Serialization
///
/// | Variant | JSON | logfmt / human |
/// |---|---|---|
/// | `Null` | `null` | empty / `<null>` |
/// | `Bool` | `true` / `false` | `true` / `false` |
/// | `I64`, `U64`, `I128`, `U128` | number | decimal |
/// | `F64` (finite) | number, always with `.` or exponent (`1.0`, `1e300`) | same text |
/// | `F64` (NaN / infinite) | `null` | `NaN`, `inf`, `-inf` |
/// | `Str`, `Char` | escaped string | escaped, quoted when needed |
/// | `Bytes` | lowercase hex string | lowercase hex |
/// | `Display`, `Debug` | escaped string of the rendered text | rendered text, escaped |
/// | `Error` | the error, then `: source` for each source, as a string | same text |
///
/// # Examples
///
/// ```
/// use log_io::Value;
///
/// let port: Value<'_> = 8080_u16.into();
/// assert_eq!(port, Value::U64(8080));
///
/// let name = String::from("alice");
/// let v: Value<'_> = (&name).into();
/// assert_eq!(v, Value::Str("alice"));
///
/// // Lazy formatting: nothing is rendered until a sink serializes it.
/// let addr = std::net::Ipv4Addr::LOCALHOST;
/// let v = Value::Display(&addr);
/// assert_eq!(v.to_string(), "127.0.0.1");
/// ```
#[derive(Clone, Copy, Default)]
#[non_exhaustive]
pub enum Value<'a> {
    /// Absence of a value.
    #[default]
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer.
    I64(i64),
    /// An unsigned 64-bit integer.
    U64(u64),
    /// A signed 128-bit integer. JSON consumers that parse numbers as
    /// IEEE doubles lose precision above 2^53.
    I128(i128),
    /// An unsigned 128-bit integer. Same precision caveat as
    /// [`Self::I128`].
    U128(u128),
    /// A 64-bit float. Finite values are written in Rust's
    /// round-trippable `{:?}` form, which always carries a decimal
    /// point or an exponent so consumers never mistake `1.0` for an
    /// integer. `NaN` and infinities become JSON `null` because JSON
    /// has no representation for them.
    F64(f64),
    /// A borrowed string slice.
    Str(&'a str),
    /// A single character, serialized as a one-character string.
    Char(char),
    /// Raw bytes, serialized as lowercase hexadecimal.
    Bytes(&'a [u8]),
    /// A value rendered through its [`fmt::Display`] implementation
    /// when (and only when) the record is serialized.
    Display(&'a dyn fmt::Display),
    /// A value rendered through its [`fmt::Debug`] implementation
    /// when (and only when) the record is serialized.
    Debug(&'a dyn fmt::Debug),
    /// An error, rendered as its `Display` text followed by
    /// `": <source>"` for every error in its [`source`] chain.
    ///
    /// [`source`]: core::error::Error::source
    Error(&'a (dyn core::error::Error + 'static)),
}

impl Value<'_> {
    /// Returns `true` if this value is [`Self::Null`].
    ///
    /// # Examples
    ///
    /// ```
    /// use log_io::Value;
    /// assert!(Value::Null.is_null());
    /// assert!(!Value::Bool(false).is_null());
    /// ```
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The static name of the variant, for diagnostics.
    ///
    /// # Examples
    ///
    /// ```
    /// use log_io::Value;
    /// assert_eq!(Value::U64(1).variant_name(), "u64");
    /// assert_eq!(Value::Str("x").variant_name(), "str");
    /// ```
    #[must_use]
    pub const fn variant_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::I64(_) => "i64",
            Self::U64(_) => "u64",
            Self::I128(_) => "i128",
            Self::U128(_) => "u128",
            Self::F64(_) => "f64",
            Self::Str(_) => "str",
            Self::Char(_) => "char",
            Self::Bytes(_) => "bytes",
            Self::Display(_) => "display",
            Self::Debug(_) => "debug",
            Self::Error(_) => "error",
        }
    }
}

/// Write `err` followed by `": <source>"` for each error in its chain.
pub(crate) fn write_error_chain<W: fmt::Write + ?Sized>(
    w: &mut W,
    err: &(dyn core::error::Error + 'static),
) -> fmt::Result {
    write!(w, "{err}")?;
    let mut source = err.source();
    while let Some(s) = source {
        write!(w, ": {s}")?;
        source = s.source();
    }
    Ok(())
}

/// Write `bytes` as lowercase hexadecimal.
pub(crate) fn write_hex<W: fmt::Write + ?Sized>(w: &mut W, bytes: &[u8]) -> fmt::Result {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for &b in bytes {
        w.write_char(char::from(DIGITS[usize::from(b >> 4)]))?;
        w.write_char(char::from(DIGITS[usize::from(b & 0x0f)]))?;
    }
    Ok(())
}

/// Write a float the way every built-in format renders it: `{:?}` for
/// finite values (always round-trippable, always has `.` or `e`), and
/// `NaN` / `inf` / `-inf` otherwise.
pub(crate) fn write_f64<W: fmt::Write + ?Sized>(w: &mut W, n: f64) -> fmt::Result {
    if n.is_finite() {
        write!(w, "{n:?}")
    } else if n.is_nan() {
        w.write_str("NaN")
    } else if n.is_sign_positive() {
        w.write_str("inf")
    } else {
        w.write_str("-inf")
    }
}

impl fmt::Display for Value<'_> {
    /// Renders the value's plain text, without any format-specific
    /// quoting or escaping. Floats use the same text as the built-in
    /// formats.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Null => f.write_str("null"),
            Self::Bool(b) => fmt::Display::fmt(&b, f),
            Self::I64(n) => fmt::Display::fmt(&n, f),
            Self::U64(n) => fmt::Display::fmt(&n, f),
            Self::I128(n) => fmt::Display::fmt(&n, f),
            Self::U128(n) => fmt::Display::fmt(&n, f),
            Self::F64(n) => write_f64(f, n),
            Self::Str(s) => f.write_str(s),
            Self::Char(c) => fmt::Display::fmt(&c, f),
            Self::Bytes(b) => write_hex(f, b),
            Self::Display(d) => fmt::Display::fmt(d, f),
            Self::Debug(d) => fmt::Debug::fmt(d, f),
            Self::Error(e) => write_error_chain(f, e),
        }
    }
}

impl fmt::Debug for Value<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Null => f.write_str("Null"),
            Self::Bool(b) => f.debug_tuple("Bool").field(&b).finish(),
            Self::I64(n) => f.debug_tuple("I64").field(&n).finish(),
            Self::U64(n) => f.debug_tuple("U64").field(&n).finish(),
            Self::I128(n) => f.debug_tuple("I128").field(&n).finish(),
            Self::U128(n) => f.debug_tuple("U128").field(&n).finish(),
            Self::F64(n) => f.debug_tuple("F64").field(&n).finish(),
            Self::Str(s) => f.debug_tuple("Str").field(&s).finish(),
            Self::Char(c) => f.debug_tuple("Char").field(&c).finish(),
            Self::Bytes(b) => f.debug_tuple("Bytes").field(&b).finish(),
            Self::Display(d) => f
                .debug_tuple("Display")
                .field(&format_args!("{d}"))
                .finish(),
            Self::Debug(d) => f.debug_tuple("Debug").field(&d).finish(),
            Self::Error(e) => f.debug_tuple("Error").field(&format_args!("{e}")).finish(),
        }
    }
}

impl PartialEq for Value<'_> {
    /// Structural equality for data variants. The lazy variants
    /// ([`Value::Display`], [`Value::Debug`], [`Value::Error`]) compare
    /// equal only when they point at the same object; their rendered
    /// text is never computed for comparison. Floats follow IEEE
    /// semantics (`NaN != NaN`).
    fn eq(&self, other: &Self) -> bool {
        match (*self, *other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::I64(a), Self::I64(b)) => a == b,
            (Self::U64(a), Self::U64(b)) => a == b,
            (Self::I128(a), Self::I128(b)) => a == b,
            (Self::U128(a), Self::U128(b)) => a == b,
            (Self::F64(a), Self::F64(b)) => a == b,
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Char(a), Self::Char(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Display(a), Self::Display(b)) => core::ptr::addr_eq(a, b),
            (Self::Debug(a), Self::Debug(b)) => core::ptr::addr_eq(a, b),
            (Self::Error(a), Self::Error(b)) => core::ptr::addr_eq(a, b),
            _ => false,
        }
    }
}

// Ergonomic conversions for call sites that don't want to spell out
// the variant.

impl<'a> From<&'a str> for Value<'a> {
    fn from(value: &'a str) -> Self {
        Self::Str(value)
    }
}

impl<'a> From<&'a [u8]> for Value<'a> {
    fn from(value: &'a [u8]) -> Self {
        Self::Bytes(value)
    }
}

impl From<bool> for Value<'_> {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<char> for Value<'_> {
    fn from(value: char) -> Self {
        Self::Char(value)
    }
}

macro_rules! from_lossless_int {
    ($($ty:ty => $variant:ident),* $(,)?) => {
        $(
            impl From<$ty> for Value<'_> {
                fn from(value: $ty) -> Self {
                    Self::$variant(value.into())
                }
            }
        )*
    };
}

from_lossless_int!(
    i8 => I64, i16 => I64, i32 => I64, i64 => I64,
    u8 => U64, u16 => U64, u32 => U64, u64 => U64,
    i128 => I128, u128 => U128,
);

impl From<isize> for Value<'_> {
    fn from(value: isize) -> Self {
        // `isize` is at most 64 bits on every supported target; the
        // 128-bit fallback keeps the conversion lossless regardless.
        i64::try_from(value).map_or(Self::I128(value as i128), Self::I64)
    }
}

impl From<usize> for Value<'_> {
    fn from(value: usize) -> Self {
        u64::try_from(value).map_or(Self::U128(value as u128), Self::U64)
    }
}

impl From<f32> for Value<'_> {
    fn from(value: f32) -> Self {
        Self::F64(f64::from(value))
    }
}

impl From<f64> for Value<'_> {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}

impl<'a, T> From<Option<T>> for Value<'a>
where
    T: Into<Value<'a>>,
{
    fn from(value: Option<T>) -> Self {
        match value {
            Some(v) => v.into(),
            None => Self::Null,
        }
    }
}

#[cfg(feature = "std")]
impl<'a> From<&'a String> for Value<'a> {
    fn from(value: &'a String) -> Self {
        Self::Str(value.as_str())
    }
}

#[cfg(feature = "std")]
impl<'a> From<&'a std::borrow::Cow<'a, str>> for Value<'a> {
    fn from(value: &'a std::borrow::Cow<'a, str>) -> Self {
        Self::Str(value.as_ref())
    }
}

#[cfg(feature = "std")]
impl<'a> From<&'a Vec<u8>> for Value<'a> {
    fn from(value: &'a Vec<u8>) -> Self {
        Self::Bytes(value.as_slice())
    }
}

/// Owned counterpart of [`Value`], used wherever a value must outlive
/// the call that produced it: logger default fields, thread-local
/// context, and captured context snapshots. Lazy variants are rendered
/// to text at capture time.
#[cfg(feature = "std")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum OwnedValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    F64(f64),
    Str(Box<str>),
    Char(char),
    Bytes(Box<[u8]>),
}

#[cfg(feature = "std")]
impl OwnedValue {
    pub(crate) fn from_value(v: Value<'_>) -> Self {
        match v {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Bool(b),
            Value::I64(n) => Self::I64(n),
            Value::U64(n) => Self::U64(n),
            Value::I128(n) => Self::I128(n),
            Value::U128(n) => Self::U128(n),
            Value::F64(n) => Self::F64(n),
            Value::Str(s) => Self::Str(s.into()),
            Value::Char(c) => Self::Char(c),
            Value::Bytes(b) => Self::Bytes(b.into()),
            other @ (Value::Display(_) | Value::Debug(_) | Value::Error(_)) => {
                Self::Str(other.to_string().into_boxed_str())
            }
        }
    }

    pub(crate) fn as_value(&self) -> Value<'_> {
        match self {
            Self::Null => Value::Null,
            Self::Bool(b) => Value::Bool(*b),
            Self::I64(n) => Value::I64(*n),
            Self::U64(n) => Value::U64(*n),
            Self::I128(n) => Value::I128(*n),
            Self::U128(n) => Value::U128(*n),
            Self::F64(n) => Value::F64(*n),
            Self::Str(s) => Value::Str(s),
            Self::Char(c) => Value::Char(*c),
            Self::Bytes(b) => Value::Bytes(b),
        }
    }
}

/// Owned counterpart of [`crate::Field`].
#[cfg(feature = "std")]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnedField {
    pub(crate) key: Box<str>,
    pub(crate) value: OwnedValue,
}

#[cfg(feature = "std")]
impl OwnedField {
    pub(crate) fn new(key: &str, value: Value<'_>) -> Self {
        Self {
            key: key.into(),
            value: OwnedValue::from_value(value),
        }
    }

    pub(crate) fn as_field(&self) -> crate::Field<'_> {
        crate::Field {
            key: &self.key,
            value: self.value.as_value(),
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Outer(Inner);
    #[derive(Debug)]
    struct Inner;
    impl fmt::Display for Outer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("outer failed")
        }
    }
    impl fmt::Display for Inner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("inner cause")
        }
    }
    impl core::error::Error for Outer {
        fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    impl core::error::Error for Inner {}

    #[test]
    fn test_value_from_str_is_borrowed() {
        let s = String::from("hello");
        let v: Value<'_> = s.as_str().into();
        assert_eq!(v, Value::Str("hello"));
        let v: Value<'_> = (&s).into();
        assert_eq!(v, Value::Str("hello"));
        let c = std::borrow::Cow::Borrowed("cow");
        assert_eq!(Value::from(&c), Value::Str("cow"));
        let bytes = vec![1_u8, 2];
        assert_eq!(Value::from(&bytes), Value::Bytes(&[1, 2]));
    }

    #[test]
    fn test_value_integer_conversions_are_lossless() {
        assert_eq!(Value::from(42_i32), Value::I64(42));
        assert_eq!(Value::from(42_u32), Value::U64(42));
        assert_eq!(Value::from(-1_i64), Value::I64(-1));
        assert_eq!(Value::from(-1_isize), Value::I64(-1));
        assert_eq!(Value::from(7_usize), Value::U64(7));
        assert_eq!(Value::from(u128::MAX), Value::U128(u128::MAX));
        assert_eq!(Value::from(i128::MIN), Value::I128(i128::MIN));
        assert_eq!(Value::from(1.5_f32), Value::F64(1.5));
    }

    #[test]
    fn test_value_option_none_is_null() {
        let some: Value<'_> = Some(7_u32).into();
        assert_eq!(some, Value::U64(7));
        let none: Value<'_> = Option::<u32>::None.into();
        assert!(none.is_null());
    }

    #[test]
    fn test_value_variant_name_is_unique_per_variant() {
        let e = Outer(Inner);
        let names: Vec<&str> = [
            Value::Null,
            Value::Bool(true),
            Value::I64(0),
            Value::U64(0),
            Value::I128(0),
            Value::U128(0),
            Value::F64(0.0),
            Value::Str(""),
            Value::Char('a'),
            Value::Bytes(&[]),
            Value::Display(&1),
            Value::Debug(&1),
            Value::Error(&e),
        ]
        .iter()
        .map(Value::variant_name)
        .collect();
        let mut dedup = names.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(dedup.len(), names.len(), "variant names must be unique");
    }

    #[test]
    fn test_value_display_renders_each_variant() {
        let e = Outer(Inner);
        assert_eq!(Value::Null.to_string(), "null");
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::I64(-5).to_string(), "-5");
        assert_eq!(Value::U64(5).to_string(), "5");
        assert_eq!(Value::I128(-5).to_string(), "-5");
        assert_eq!(Value::U128(5).to_string(), "5");
        assert_eq!(Value::F64(1.0).to_string(), "1.0");
        assert_eq!(Value::F64(f64::NAN).to_string(), "NaN");
        assert_eq!(Value::F64(f64::INFINITY).to_string(), "inf");
        assert_eq!(Value::F64(f64::NEG_INFINITY).to_string(), "-inf");
        assert_eq!(Value::Str("x").to_string(), "x");
        assert_eq!(Value::Char('y').to_string(), "y");
        assert_eq!(Value::Bytes(&[0x00, 0xab, 0xff]).to_string(), "00abff");
        assert_eq!(Value::Display(&42).to_string(), "42");
        assert_eq!(Value::Debug(&"q").to_string(), "\"q\"");
        assert_eq!(Value::Error(&e).to_string(), "outer failed: inner cause");
    }

    #[test]
    fn test_value_debug_is_readable() {
        assert_eq!(format!("{:?}", Value::U64(3)), "U64(3)");
        assert_eq!(format!("{:?}", Value::Display(&7)), "Display(7)");
        assert_eq!(format!("{:?}", Value::Debug(&7)), "Debug(7)");
        assert_eq!(format!("{:?}", Value::Null), "Null");
        let e = Outer(Inner);
        assert_eq!(format!("{:?}", Value::Error(&e)), "Error(outer failed)");
    }

    #[test]
    fn test_value_eq_lazy_variants_compare_by_identity() {
        let a = 5;
        let b = 5;
        assert_eq!(Value::Display(&a), Value::Display(&a));
        assert_ne!(Value::Display(&a), Value::Display(&b));
        assert_eq!(Value::Debug(&a), Value::Debug(&a));
        assert_ne!(Value::F64(f64::NAN), Value::F64(f64::NAN));
        assert_ne!(Value::U64(1), Value::I64(1));
        let e = Outer(Inner);
        assert_eq!(Value::Error(&e), Value::Error(&e));
    }

    #[test]
    fn test_owned_value_renders_lazy_variants_to_text() {
        let e = Outer(Inner);
        let owned = OwnedValue::from_value(Value::Error(&e));
        assert_eq!(owned.as_value(), Value::Str("outer failed: inner cause"));
        let owned = OwnedValue::from_value(Value::Bytes(&[1, 2]));
        assert_eq!(owned.as_value(), Value::Bytes(&[1, 2]));
        for v in [
            Value::Null,
            Value::Bool(true),
            Value::I64(-1),
            Value::U64(1),
            Value::I128(-2),
            Value::U128(2),
            Value::F64(0.5),
            Value::Str("s"),
            Value::Char('c'),
        ] {
            assert_eq!(OwnedValue::from_value(v).as_value(), v);
        }
    }

    #[test]
    fn test_float_text_is_round_trippable() {
        for x in [1.0, 0.1, 1e300, f64::MAX, 5e-324, -0.0, 1e21, 1e-7] {
            let s = Value::F64(x).to_string();
            assert!(s.contains('.') || s.contains('e'), "{s}");
            assert_eq!(s.parse::<f64>().unwrap().to_bits(), x.to_bits(), "{s}");
        }
    }
}

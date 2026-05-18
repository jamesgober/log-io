//! Field values.
//!
//! [`Value`] is a borrowed sum type for the data attached to a log
//! [`crate::Field`]. Borrowing keeps the fast path allocation-free; if
//! a caller needs to construct a value from owned data, it should
//! borrow at the call site.

use core::fmt;

/// A field value attached to a log record.
///
/// All variants borrow their data. Construct a `Value` at the call site
/// from a borrowed source so the record carries no heap allocation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Value<'a> {
    /// Absence of a value. Serializes as `null` in JSON, the empty
    /// string in logfmt, and `<null>` in the human-readable format.
    #[default]
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer.
    I64(i64),
    /// An unsigned 64-bit integer.
    U64(u64),
    /// A 64-bit float. `NaN` and `inf` are serialized as JSON `null`
    /// per RFC 8259 since they have no JSON representation; logfmt and
    /// the human format emit the lowercase debug representation.
    F64(f64),
    /// A borrowed string slice.
    Str(&'a str),
    /// A borrowed character. Serialized as a one-character string.
    Char(char),
}

impl<'a> Value<'a> {
    /// Returns `true` if this value is [`Self::Null`].
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Returns the static name of the variant. Useful in diagnostics.
    pub const fn variant_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::I64(_) => "i64",
            Self::U64(_) => "u64",
            Self::F64(_) => "f64",
            Self::Str(_) => "str",
            Self::Char(_) => "char",
        }
    }
}

impl<'a> fmt::Display for Value<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Bool(b) => fmt::Display::fmt(b, f),
            Self::I64(n) => fmt::Display::fmt(n, f),
            Self::U64(n) => fmt::Display::fmt(n, f),
            Self::F64(n) => fmt::Display::fmt(n, f),
            Self::Str(s) => f.write_str(s),
            Self::Char(c) => fmt::Display::fmt(c, f),
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

macro_rules! from_int {
    ($($ty:ty => $variant:ident),* $(,)?) => {
        $(
            impl From<$ty> for Value<'_> {
                fn from(value: $ty) -> Self {
                    #[allow(clippy::cast_lossless)]
                    Self::$variant(value as _)
                }
            }
        )*
    };
}

from_int!(
    i8 => I64, i16 => I64, i32 => I64, i64 => I64, isize => I64,
    u8 => U64, u16 => U64, u32 => U64, u64 => U64, usize => U64,
);

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

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn from_str_is_borrowed() {
        let s = String::from("hello");
        let v: Value<'_> = s.as_str().into();
        assert_eq!(v, Value::Str("hello"));
    }

    #[test]
    fn integer_conversions() {
        let v: Value<'_> = 42_i32.into();
        assert_eq!(v, Value::I64(42));
        let v: Value<'_> = 42_u32.into();
        assert_eq!(v, Value::U64(42));
        let v: Value<'_> = (-1_i64).into();
        assert_eq!(v, Value::I64(-1));
    }

    #[test]
    fn option_into_value() {
        let some: Value<'_> = Some(7_u32).into();
        assert_eq!(some, Value::U64(7));
        let none: Value<'_> = Option::<u32>::None.into();
        assert!(none.is_null());
    }

    #[test]
    fn variant_name_covers_all() {
        for v in [
            Value::Null,
            Value::Bool(true),
            Value::I64(0),
            Value::U64(0),
            Value::F64(0.0),
            Value::Str(""),
            Value::Char('a'),
        ] {
            assert!(!v.variant_name().is_empty());
        }
    }

    #[test]
    fn display_renders_each_variant() {
        assert_eq!(Value::Null.to_string(), "null");
        assert_eq!(Value::Bool(true).to_string(), "true");
        assert_eq!(Value::I64(-5).to_string(), "-5");
        assert_eq!(Value::U64(5).to_string(), "5");
        assert_eq!(Value::Str("x").to_string(), "x");
        assert_eq!(Value::Char('y').to_string(), "y");
    }
}

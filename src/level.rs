//! Severity levels.

use core::cmp::Ordering;
use core::fmt;
use core::str::FromStr;

/// Severity of a log record.
///
/// Levels are ordered from least to most severe: [`Self::Trace`] <
/// [`Self::Debug`] < [`Self::Info`] < [`Self::Warn`] < [`Self::Error`].
/// [`Self::Off`] is greater than all real severities and is used
/// exclusively to disable a filter or logger entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Level {
    /// Fine-grained diagnostic information for tracing execution flow.
    Trace = 0,
    /// Diagnostic information useful while debugging.
    Debug = 1,
    /// Informational messages about normal operation.
    Info = 2,
    /// A condition that may require attention but is not an error.
    Warn = 3,
    /// An error condition that prevented an operation from completing.
    Error = 4,
    /// Disable logging entirely. Never attached to a real [`crate::Record`].
    /// Used only as a filter threshold sentinel meaning "admit nothing".
    Off = 255,
}

impl Level {
    /// The lowercase canonical string for this level.
    ///
    /// Returns `"off"` for [`Self::Off`]. This is the form emitted by
    /// formatters by default.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// The uppercase canonical string for this level.
    ///
    /// Useful for human-readable formats that conventionally upper-case
    /// the severity column.
    pub const fn as_str_upper(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    /// Returns `true` if this level is at least as severe as `other`.
    ///
    /// Used by filters: a record at level `self` passes a threshold of
    /// `other` when `self.is_enabled_at(other)` is `true`.
    pub const fn is_enabled_at(self, threshold: Self) -> bool {
        (self as u8) >= (threshold as u8)
    }

    /// All real severities, ordered from least to most severe.
    ///
    /// Excludes [`Self::Off`]. Useful for iteration in tests.
    pub const ALL: [Self; 5] = [
        Self::Trace,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
    ];
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialOrd for Level {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Level {
    fn cmp(&self, other: &Self) -> Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

/// Error returned when a string cannot be parsed as a [`Level`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseLevelError;

impl fmt::Display for ParseLevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid log level (expected one of: off, trace, debug, info, warn, error)")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseLevelError {}

impl FromStr for Level {
    type Err = ParseLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Case-insensitive without allocating: compare byte-by-byte.
        if s.eq_ignore_ascii_case("off") {
            Ok(Self::Off)
        } else if s.eq_ignore_ascii_case("trace") {
            Ok(Self::Trace)
        } else if s.eq_ignore_ascii_case("debug") {
            Ok(Self::Debug)
        } else if s.eq_ignore_ascii_case("info") {
            Ok(Self::Info)
        } else if s.eq_ignore_ascii_case("warn") || s.eq_ignore_ascii_case("warning") {
            Ok(Self::Warn)
        } else if s.eq_ignore_ascii_case("error") || s.eq_ignore_ascii_case("err") {
            Ok(Self::Error)
        } else {
            Err(ParseLevelError)
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_total_and_severity_aligned() {
        assert!(Level::Trace < Level::Debug);
        assert!(Level::Debug < Level::Info);
        assert!(Level::Info < Level::Warn);
        assert!(Level::Warn < Level::Error);
        assert!(Level::Error < Level::Off);
    }

    #[test]
    fn is_enabled_at_respects_threshold() {
        assert!(Level::Error.is_enabled_at(Level::Info));
        assert!(Level::Info.is_enabled_at(Level::Info));
        assert!(!Level::Debug.is_enabled_at(Level::Info));
        assert!(!Level::Trace.is_enabled_at(Level::Warn));
    }

    #[test]
    fn parse_round_trip_canonical_forms() {
        for level in Level::ALL {
            let parsed: Level = level.as_str().parse().unwrap();
            assert_eq!(parsed, level);
            let parsed_upper: Level = level.as_str_upper().parse().unwrap();
            assert_eq!(parsed_upper, level);
        }
        assert_eq!("off".parse::<Level>().unwrap(), Level::Off);
        assert_eq!("OFF".parse::<Level>().unwrap(), Level::Off);
    }

    #[test]
    fn parse_aliases() {
        assert_eq!("warning".parse::<Level>().unwrap(), Level::Warn);
        assert_eq!("err".parse::<Level>().unwrap(), Level::Error);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!("verbose".parse::<Level>().is_err());
        assert!("".parse::<Level>().is_err());
    }

    #[test]
    fn display_lowercase() {
        assert_eq!(Level::Info.to_string(), "info");
    }

    #[test]
    fn upper_form() {
        assert_eq!(Level::Warn.as_str_upper(), "WARN");
    }
}

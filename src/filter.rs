//! Per-target severity filtering.
//!
//! A [`Filter`] is a default threshold plus a set of per-target
//! [`FilterRule`]s. For each record the **most specific** matching
//! rule wins: the rule with the longest target that is a prefix of the
//! record's target at a module boundary. If no rule matches, the
//! default applies. This matches the semantics of `RUST_LOG` in
//! `env_logger` and `tracing-subscriber`, so directive order does not
//! matter.
//!
//! A rule for `app::db` matches the targets `app::db`, `app::db::pool`
//! and `app::db.pool` (both `::` and `.` count as boundaries), but not
//! `app::dbx`.

use crate::level::Level;

/// One rule in a [`Filter`]: records whose target is (or is nested
/// under) `target` are admitted when their level is at least `level`.
///
/// # Examples
///
/// ```
/// use log_io::{FilterRule, Level};
///
/// let rule = FilterRule::new("app::db", Level::Debug);
/// assert_eq!(rule.target, "app::db");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterRule {
    /// Target prefix the rule applies to.
    pub target: String,
    /// Minimum severity allowed through.
    pub level: Level,
}

impl FilterRule {
    /// Build a rule from a target prefix and a minimum severity.
    pub fn new(target: impl Into<String>, level: Level) -> Self {
        Self {
            target: target.into(),
            level,
        }
    }
}

/// A target-aware severity gate.
///
/// # Examples
///
/// ```
/// use log_io::{Filter, Level};
///
/// // Order does not matter: the most specific rule always wins.
/// let f = Filter::parse("info, app::db=trace, app=warn")?;
/// assert!(f.is_enabled("app::db::pool", Level::Trace));
/// assert!(!f.is_enabled("app::http", Level::Info));
/// assert!(f.is_enabled("hyper", Level::Info));
/// # Ok::<(), log_io::ParseFilterError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    default_level: Level,
    /// Sorted by descending target length, so the first match is the
    /// most specific. Targets are unique.
    rules: Vec<FilterRule>,
    /// The lowest threshold anywhere in the filter; anything below it
    /// is rejected without scanning the rules.
    min_level: Level,
}

impl Filter {
    /// New filter with a default minimum severity and no overrides.
    #[must_use]
    pub fn new(default_level: Level) -> Self {
        Self {
            default_level,
            rules: Vec::new(),
            min_level: default_level,
        }
    }

    /// Add (or replace) the rule for `target`.
    ///
    /// Rules may be added in any order; matching always picks the most
    /// specific one. Adding a second rule for the same target replaces
    /// the first. Unlike [`Filter::parse`], the target is not validated:
    /// an empty target matches only records whose target is empty
    /// (which is what [`crate::Logger::log`] uses unless a default
    /// target is configured).
    #[must_use]
    pub fn with_rule(mut self, target: impl Into<String>, level: Level) -> Self {
        self.insert(FilterRule::new(target, level));
        self
    }

    fn insert(&mut self, rule: FilterRule) {
        if let Some(existing) = self.rules.iter_mut().find(|r| r.target == rule.target) {
            existing.level = rule.level;
        } else {
            // Keep the vector sorted by descending length; equal-length
            // targets cannot both match one record, so their relative
            // order is irrelevant.
            let at = self
                .rules
                .partition_point(|r| r.target.len() >= rule.target.len());
            self.rules.insert(at, rule);
        }
        self.recompute_min();
    }

    fn recompute_min(&mut self) {
        self.min_level = self
            .rules
            .iter()
            .map(|r| r.level)
            .fold(self.default_level, core::cmp::min);
    }

    /// The default severity, applied when no rule matches.
    #[must_use]
    pub fn default_level(&self) -> Level {
        self.default_level
    }

    /// Replace the default severity, keeping every rule.
    #[must_use]
    pub fn with_default_level(mut self, level: Level) -> Self {
        self.default_level = level;
        self.recompute_min();
        self
    }

    /// The configured rules, most specific (longest target) first.
    #[must_use]
    pub fn rules(&self) -> &[FilterRule] {
        &self.rules
    }

    /// The lowest severity this filter can admit for any target.
    /// Records below it are rejected without examining their target.
    #[must_use]
    pub fn min_level(&self) -> Level {
        self.min_level
    }

    /// Decide whether a record at `level` for `target` is admitted.
    #[inline]
    #[must_use]
    pub fn is_enabled(&self, target: &str, level: Level) -> bool {
        if level < self.min_level || level == Level::Off {
            return false;
        }
        level.is_enabled_at(self.threshold_for(target))
    }

    /// The threshold that applies to `target`: the level of the most
    /// specific matching rule, or the default.
    #[must_use]
    pub fn threshold_for(&self, target: &str) -> Level {
        self.rules
            .iter()
            .find(|rule| target_matches(target, &rule.target))
            .map_or(self.default_level, |rule| rule.level)
    }

    /// Parse a directive string such as `info,app::auth=debug,hyper=off`.
    ///
    /// The grammar is a comma-separated list of segments. Each segment
    /// is either a bare level (sets the default) or `target=level` (adds
    /// a rule). Whitespace around segments, targets and levels is
    /// ignored, and empty segments (`a=info,,b=warn`) are skipped. Level
    /// names are case-insensitive and accept the aliases `warning` and
    /// `err`. When a level or a target appears more than once, the last
    /// occurrence wins. If no bare level is given, the default is
    /// [`Level::Off`]: only the listed targets are logged.
    ///
    /// Differences from `env_logger`: a bare word must be a level (a
    /// bare target such as `my_crate` is rejected rather than meaning
    /// "everything for my_crate", so a misspelt level is reported
    /// instead of silently becoming a target), and the `/regex` message
    /// filter is not supported.
    ///
    /// # Errors
    ///
    /// Returns [`ParseFilterError`] identifying the offending segment
    /// when the input is empty or blank, a level is unknown, a target is
    /// empty (`=debug`), a target contains whitespace, or a target ends
    /// with a separator (`app::=debug`, `app.=debug`).
    ///
    /// # Examples
    ///
    /// ```
    /// use log_io::{Filter, Level, ParseFilterErrorKind};
    ///
    /// let f = Filter::parse("warn,app=debug")?;
    /// assert_eq!(f.default_level(), Level::Warn);
    ///
    /// let err = Filter::parse("info,app=loud").unwrap_err();
    /// assert_eq!(err.kind(), ParseFilterErrorKind::InvalidLevel);
    /// assert_eq!(err.segment(), "app=loud");
    /// # Ok::<(), log_io::ParseFilterError>(())
    /// ```
    pub fn parse(directive: &str) -> Result<Self, ParseFilterError> {
        if directive.trim().is_empty() {
            return Err(ParseFilterError::new(
                ParseFilterErrorKind::Empty,
                directive,
            ));
        }
        let mut filter = Self::new(Level::Off);
        for raw in directive.split(',') {
            let segment = raw.trim();
            if segment.is_empty() {
                continue;
            }
            match segment.split_once('=') {
                Some((target, level)) => {
                    let target = target.trim();
                    validate_target(target, segment)?;
                    let level: Level = level.trim().parse().map_err(|_| {
                        ParseFilterError::new(ParseFilterErrorKind::InvalidLevel, segment)
                    })?;
                    filter.insert(FilterRule::new(target, level));
                }
                None => {
                    let level: Level = segment.parse().map_err(|_| {
                        ParseFilterError::new(ParseFilterErrorKind::InvalidLevel, segment)
                    })?;
                    filter.default_level = level;
                }
            }
        }
        filter.recompute_min();
        Ok(filter)
    }
}

impl core::str::FromStr for Filter {
    type Err = ParseFilterError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

fn validate_target(target: &str, segment: &str) -> Result<(), ParseFilterError> {
    let kind = if target.is_empty() {
        ParseFilterErrorKind::EmptyTarget
    } else if target.ends_with(':') || target.ends_with('.') {
        ParseFilterErrorKind::TrailingSeparator
    } else if target.chars().any(char::is_whitespace) {
        ParseFilterErrorKind::InvalidTarget
    } else {
        return Ok(());
    };
    Err(ParseFilterError::new(kind, segment))
}

/// What was wrong with a directive passed to [`Filter::parse`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseFilterErrorKind {
    /// The directive was empty or contained only whitespace and commas.
    Empty,
    /// A level name was not recognized.
    InvalidLevel,
    /// A `target=level` segment had an empty target.
    EmptyTarget,
    /// A target ended with `::`, `:` or `.`, so it could never match.
    TrailingSeparator,
    /// A target contained whitespace.
    InvalidTarget,
    /// The directive came from an environment variable whose value is
    /// not valid Unicode.
    NotUnicode,
}

/// Error returned by [`Filter::parse`].
///
/// Carries the kind of problem and the offending directive segment so
/// the message can point at the exact mistake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFilterError {
    kind: ParseFilterErrorKind,
    segment: String,
}

impl ParseFilterError {
    fn new(kind: ParseFilterErrorKind, segment: &str) -> Self {
        Self {
            kind,
            segment: segment.to_owned(),
        }
    }

    pub(crate) fn not_unicode(raw: &std::ffi::OsStr) -> Self {
        Self {
            kind: ParseFilterErrorKind::NotUnicode,
            segment: raw.to_string_lossy().into_owned(),
        }
    }

    /// What was wrong.
    #[must_use]
    pub fn kind(&self) -> ParseFilterErrorKind {
        self.kind
    }

    /// The offending segment (the whole directive for
    /// [`ParseFilterErrorKind::Empty`], and a lossy rendering of the
    /// value for [`ParseFilterErrorKind::NotUnicode`]).
    #[must_use]
    pub fn segment(&self) -> &str {
        &self.segment
    }
}

impl core::fmt::Display for ParseFilterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let what = match self.kind {
            ParseFilterErrorKind::Empty => "directive is empty",
            ParseFilterErrorKind::InvalidLevel => {
                "unknown level (expected off, trace, debug, info, warn or error)"
            }
            ParseFilterErrorKind::EmptyTarget => "target is empty",
            ParseFilterErrorKind::TrailingSeparator => "target ends with a separator",
            ParseFilterErrorKind::InvalidTarget => "target contains whitespace",
            ParseFilterErrorKind::NotUnicode => "value is not valid Unicode",
        };
        write!(f, "invalid filter directive {:?}: {what}", self.segment)
    }
}

impl std::error::Error for ParseFilterError {}

/// `true` when `target` equals `prefix` or continues it after a `::`
/// or `.` boundary.
#[inline]
fn target_matches(target: &str, prefix: &str) -> bool {
    match target.strip_prefix(prefix) {
        Some("") => true,
        Some(rest) => !prefix.is_empty() && (rest.starts_with("::") || rest.starts_with('.')),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_default_applies_when_no_rules_match() {
        let f = Filter::new(Level::Warn);
        assert!(f.is_enabled("anything", Level::Warn));
        assert!(!f.is_enabled("anything", Level::Info));
    }

    #[test]
    fn test_filter_rule_overrides_default_for_matching_prefix() {
        let f = Filter::new(Level::Warn).with_rule("app::auth", Level::Debug);
        assert!(f.is_enabled("app::auth", Level::Debug));
        assert!(f.is_enabled("app::auth::login", Level::Debug));
        assert!(!f.is_enabled("app::auth", Level::Trace));
        assert!(!f.is_enabled("hyper", Level::Info));
    }

    #[test]
    fn test_filter_prefix_match_respects_module_boundary() {
        let f = Filter::new(Level::Off).with_rule("app", Level::Info);
        assert!(f.is_enabled("app", Level::Info));
        assert!(f.is_enabled("app::sub", Level::Info));
        assert!(f.is_enabled("app.sub", Level::Info));
        assert!(!f.is_enabled("application", Level::Info));
        assert!(!f.is_enabled("ap", Level::Info));
        assert!(!f.is_enabled("app:", Level::Info));
        assert!(!f.is_enabled("app:x", Level::Info));
    }

    #[test]
    fn test_filter_most_specific_rule_wins_regardless_of_order() {
        // The audit's R03 reproduction.
        let f = Filter::parse("info,app=warn,app::db=trace").unwrap();
        assert!(f.is_enabled("app::db", Level::Trace));
        assert!(f.is_enabled("app::db::pool", Level::Trace));
        assert!(!f.is_enabled("app::http", Level::Info));
        assert!(f.is_enabled("other", Level::Info));
        let reversed = Filter::parse("app::db=trace,app=warn,info").unwrap();
        assert_eq!(f, reversed);
        let built = Filter::new(Level::Info)
            .with_rule("app", Level::Warn)
            .with_rule("app::db", Level::Trace);
        assert_eq!(built.threshold_for("app::db::x"), Level::Trace);
    }

    #[test]
    fn test_filter_same_target_last_wins() {
        let f = Filter::parse("app=error,app=debug").unwrap();
        assert_eq!(f.rules().len(), 1);
        assert_eq!(f.threshold_for("app"), Level::Debug);
        let f = Filter::new(Level::Info)
            .with_rule("x", Level::Error)
            .with_rule("x", Level::Trace);
        assert_eq!(f.rules().len(), 1);
        assert_eq!(f.threshold_for("x"), Level::Trace);
        let f = Filter::parse("info,warn").unwrap();
        assert_eq!(f.default_level(), Level::Warn);
    }

    #[test]
    fn test_filter_rules_are_ordered_most_specific_first() {
        let f = Filter::parse("a=info,a::b::c=debug,a::b=warn").unwrap();
        let targets: Vec<&str> = f.rules().iter().map(|r| r.target.as_str()).collect();
        assert_eq!(targets, ["a::b::c", "a::b", "a"]);
    }

    #[test]
    fn test_filter_min_level_fast_path() {
        let f = Filter::new(Level::Warn).with_rule("loud", Level::Trace);
        assert_eq!(f.min_level(), Level::Trace);
        assert!(f.is_enabled("loud", Level::Trace));
        let quiet = Filter::new(Level::Error);
        assert_eq!(quiet.min_level(), Level::Error);
        assert!(!quiet.is_enabled("x", Level::Warn));
        let raised = f.with_default_level(Level::Debug);
        assert_eq!(raised.default_level(), Level::Debug);
        assert_eq!(raised.min_level(), Level::Trace);
    }

    #[test]
    fn test_filter_never_admits_off_level_records() {
        let f = Filter::new(Level::Trace);
        assert!(!f.is_enabled("x", Level::Off));
    }

    #[test]
    fn test_filter_empty_target_rule_matches_only_empty_target() {
        let f = Filter::new(Level::Off).with_rule("", Level::Info);
        assert!(f.is_enabled("", Level::Info));
        assert!(!f.is_enabled("app", Level::Info));
        assert!(!f.is_enabled("::app", Level::Info));
    }

    #[test]
    fn test_parse_simple_directive() {
        let f = Filter::parse("info").unwrap();
        assert_eq!(f.default_level(), Level::Info);
        assert!(f.rules().is_empty());
        let f: Filter = "DEBUG".parse().unwrap();
        assert_eq!(f.default_level(), Level::Debug);
    }

    #[test]
    fn test_parse_with_overrides_and_whitespace() {
        let f = Filter::parse(" info, app::auth = debug ,hyper=warn,, ").unwrap();
        assert_eq!(f.default_level(), Level::Info);
        assert_eq!(f.rules().len(), 2);
        assert!(f.is_enabled("hyper", Level::Warn));
        assert!(!f.is_enabled("hyper", Level::Info));
        assert!(f.is_enabled("app::auth", Level::Debug));
    }

    #[test]
    fn test_parse_rules_without_default_disable_unmatched() {
        let f = Filter::parse("hyper=warn").unwrap();
        assert_eq!(f.default_level(), Level::Off);
        assert!(f.is_enabled("hyper", Level::Warn));
        assert!(!f.is_enabled("other", Level::Error));
    }

    #[test]
    fn test_parse_rejects_degenerate_input_with_typed_errors() {
        let cases = [
            ("", ParseFilterErrorKind::Empty, ""),
            ("  ", ParseFilterErrorKind::Empty, "  "),
            ("nope", ParseFilterErrorKind::InvalidLevel, "nope"),
            (
                "info,hyper=verbose",
                ParseFilterErrorKind::InvalidLevel,
                "hyper=verbose",
            ),
            ("=debug", ParseFilterErrorKind::EmptyTarget, "=debug"),
            (
                "app::=debug",
                ParseFilterErrorKind::TrailingSeparator,
                "app::=debug",
            ),
            (
                "app.=debug",
                ParseFilterErrorKind::TrailingSeparator,
                "app.=debug",
            ),
            (
                "my app=debug",
                ParseFilterErrorKind::InvalidTarget,
                "my app=debug",
            ),
            ("a=b=c", ParseFilterErrorKind::InvalidLevel, "a=b=c"),
        ];
        for (input, kind, segment) in cases {
            let err = Filter::parse(input).unwrap_err();
            assert_eq!(err.kind(), kind, "{input:?}");
            assert_eq!(err.segment(), segment, "{input:?}");
        }
    }

    #[test]
    fn test_parse_filter_error_display_names_segment_and_cause() {
        let err = Filter::parse("x=loud").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid filter directive \"x=loud\": unknown level (expected off, trace, debug, info, warn or error)"
        );
        for (input, needle) in [
            ("", "empty"),
            ("=x", "target is empty"),
            ("a::=info", "separator"),
            ("a b=info", "whitespace"),
        ] {
            let msg = Filter::parse(input).unwrap_err().to_string();
            assert!(msg.contains(needle), "{input:?}: {msg}");
        }
    }

    #[test]
    fn test_target_matches_boundaries() {
        assert!(target_matches("a", "a"));
        assert!(target_matches("a::b", "a"));
        assert!(target_matches("a.b", "a"));
        assert!(!target_matches("ab", "a"));
        assert!(!target_matches("a:b", "a"));
        assert!(!target_matches("b", "a"));
        assert!(!target_matches("a", "a::b"));
        assert!(target_matches("", ""));
        assert!(!target_matches("::a", ""));
    }
}

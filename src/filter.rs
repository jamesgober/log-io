//! Per-target severity filtering.
//!
//! A [`Filter`] is a list of [`FilterRule`]s plus a default threshold.
//! For each record, the first rule whose target prefix matches is
//! consulted; if none match, the default threshold applies. Matching
//! is prefix-based on `target`, so a rule of `"app::auth"` matches
//! records whose target starts with `app::auth` (and is either equal
//! or followed by `::`).

use crate::level::Level;

/// One rule in a [`Filter`].
///
/// A rule pairs a target prefix with a minimum severity. A record is
/// admitted by a rule when its level is `>=` the rule's level.
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
pub struct Filter {
    default_level: Level,
    rules: Vec<FilterRule>,
}

impl Filter {
    /// New filter with a default minimum severity and no overrides.
    pub fn new(default_level: Level) -> Self {
        Self {
            default_level,
            rules: Vec::new(),
        }
    }

    /// Add a per-target override. Rules are checked in insertion order;
    /// the longest, most specific prefix should be added first.
    pub fn with_rule(mut self, target: impl Into<String>, level: Level) -> Self {
        self.rules.push(FilterRule::new(target, level));
        self
    }

    /// The configured default severity.
    pub fn default_level(&self) -> Level {
        self.default_level
    }

    /// The configured rules in insertion order.
    pub fn rules(&self) -> &[FilterRule] {
        &self.rules
    }

    /// Decide whether a record at `level`/`target` should be admitted.
    ///
    /// Returns `true` when the level is at least as severe as the rule
    /// matched (or the default, if no rule matches).
    pub fn is_enabled(&self, target: &str, level: Level) -> bool {
        level.is_enabled_at(self.threshold_for(target))
    }

    /// Return the configured threshold for a target.
    pub fn threshold_for(&self, target: &str) -> Level {
        for rule in &self.rules {
            if target_matches(target, &rule.target) {
                return rule.level;
            }
        }
        self.default_level
    }

    /// Parse a directive string like `info,app::auth=debug,hyper=warn`.
    ///
    /// Whitespace is trimmed. Unknown levels return an error.
    ///
    /// # Errors
    ///
    /// Returns [`ParseFilterError`] if any segment fails to parse as
    /// `Level` or `target=Level`.
    pub fn parse(directive: &str) -> core::result::Result<Self, ParseFilterError> {
        let mut default_level = Level::Off;
        let mut rules = Vec::new();
        let mut saw_default = false;

        for segment in directive.split(',') {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            if let Some((target, level)) = segment.split_once('=') {
                let level: Level = level.trim().parse().map_err(|_| ParseFilterError)?;
                rules.push(FilterRule::new(target.trim(), level));
            } else {
                let level: Level = segment.parse().map_err(|_| ParseFilterError)?;
                default_level = level;
                saw_default = true;
            }
        }

        if !saw_default && !rules.is_empty() {
            default_level = Level::Off;
        }
        Ok(Self {
            default_level,
            rules,
        })
    }
}

/// Error returned by [`Filter::parse`] when a directive segment is
/// malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseFilterError;

impl core::fmt::Display for ParseFilterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("invalid filter directive")
    }
}

impl std::error::Error for ParseFilterError {}

fn target_matches(target: &str, prefix: &str) -> bool {
    if target == prefix {
        return true;
    }
    if target.len() > prefix.len() && target.starts_with(prefix) {
        let rest = &target.as_bytes()[prefix.len()..];
        // Accept either `::` (Rust module separator) or `.` (dot path).
        return rest.starts_with(b"::") || rest.starts_with(b".");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_applies_when_no_rules_match() {
        let f = Filter::new(Level::Warn);
        assert!(f.is_enabled("anything", Level::Warn));
        assert!(!f.is_enabled("anything", Level::Info));
    }

    #[test]
    fn rule_overrides_default_for_matching_prefix() {
        let f = Filter::new(Level::Warn).with_rule("app::auth", Level::Debug);
        assert!(f.is_enabled("app::auth", Level::Debug));
        assert!(f.is_enabled("app::auth::login", Level::Debug));
        assert!(!f.is_enabled("app::auth", Level::Trace));
        // Unrelated target still hits the default.
        assert!(!f.is_enabled("hyper", Level::Info));
    }

    #[test]
    fn prefix_match_respects_module_boundary() {
        let f = Filter::new(Level::Off).with_rule("app", Level::Info);
        assert!(f.is_enabled("app", Level::Info));
        assert!(f.is_enabled("app::sub", Level::Info));
        // "application" must not match the "app" rule.
        assert!(!f.is_enabled("application", Level::Info));
    }

    #[test]
    fn parse_simple_directive() {
        let f = Filter::parse("info").unwrap();
        assert_eq!(f.default_level(), Level::Info);
        assert!(f.rules().is_empty());
    }

    #[test]
    fn parse_with_overrides() {
        let f = Filter::parse("info, app::auth = debug ,hyper=warn").unwrap();
        assert_eq!(f.default_level(), Level::Info);
        assert_eq!(f.rules().len(), 2);
        assert!(f.is_enabled("hyper", Level::Warn));
        assert!(!f.is_enabled("hyper", Level::Info));
        assert!(f.is_enabled("app::auth", Level::Debug));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(Filter::parse("nope").is_err());
        assert!(Filter::parse("hyper=verbose").is_err());
    }

    #[test]
    fn rules_without_default_disables_unmatched() {
        let f = Filter::parse("hyper=warn").unwrap();
        assert_eq!(f.default_level(), Level::Off);
        assert!(f.is_enabled("hyper", Level::Warn));
        assert!(!f.is_enabled("other", Level::Error));
    }
}

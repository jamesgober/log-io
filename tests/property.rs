//! Property tests: invariants that must hold for any input.
//!
//! * JSON output parses with `serde_json` and round-trips the message,
//!   target and every field value.
//! * logfmt output parses with an independent reference parser (below)
//!   and round-trips every string.
//! * All three formats emit exactly one line per record and never emit
//!   a raw control, C1, line-separator or bidi character.
//! * The filter agrees with a most-specific-prefix oracle.
//! * Thread-local context agrees with a simple model.
//! * RFC 3339 timestamps agree with a naive calendar walk.

use std::str::FromStr;

use log_io::format::{Format, HumanFormat, JsonFormat, LogfmtFormat};
use log_io::{Field, Filter, Level, Metadata, Record, Value, context};
use proptest::prelude::*;

fn arb_level() -> impl Strategy<Value = Level> {
    prop_oneof![
        Just(Level::Trace),
        Just(Level::Debug),
        Just(Level::Info),
        Just(Level::Warn),
        Just(Level::Error),
    ]
}

/// Characters that exercise every escaping path.
fn arb_tricky_char() -> impl Strategy<Value = char> {
    prop_oneof![
        3 => any::<char>(),
        1 => prop::sample::select(vec![
            '\0', '\n', '\r', '\t', '\u{7}', '\u{8}', '\u{b}', '\u{c}', '\u{1b}', '\u{7f}',
            '\u{80}', '\u{85}', '\u{9b}', '\u{9f}', '\u{a0}', '\u{a9}', '\u{2028}', '\u{2029}',
            '\u{202a}', '\u{202e}', '\u{2066}', '\u{2069}', '\u{2014}', '"', '\\', '=', ' ',
            '{', '}', '[', ']', ':', ',', '\u{fffd}', '\u{1f680}',
        ]),
        2 => prop::char::range('a', 'z'),
    ]
}

fn arb_text() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::collection::vec(arb_tricky_char(), 0..24).prop_map(|v| v.into_iter().collect()),
        any::<String>(),
    ]
}

#[derive(Debug, Clone)]
enum Owned {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    I128(i128),
    U128(u128),
    F64(f64),
    Str(String),
    Char(char),
    Bytes(Vec<u8>),
    Display(String),
    Debug(String),
}

impl Owned {
    fn as_value(&self) -> Value<'_> {
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
            Self::Display(s) => Value::Display(s),
            Self::Debug(s) => Value::Debug(s),
        }
    }

    /// The text a string-like value must decode back to.
    fn expected_text(&self) -> Option<String> {
        match self {
            Self::Str(s) | Self::Display(s) => Some(s.clone()),
            Self::Debug(s) => Some(format!("{s:?}")),
            Self::Char(c) => Some(c.to_string()),
            Self::Bytes(b) => Some(b.iter().map(|x| format!("{x:02x}")).collect()),
            _ => None,
        }
    }
}

fn arb_owned() -> impl Strategy<Value = Owned> {
    prop_oneof![
        Just(Owned::Null),
        any::<bool>().prop_map(Owned::Bool),
        any::<i64>().prop_map(Owned::I64),
        any::<u64>().prop_map(Owned::U64),
        any::<i128>().prop_map(Owned::I128),
        any::<u128>().prop_map(Owned::U128),
        any::<f64>().prop_map(Owned::F64),
        arb_text().prop_map(Owned::Str),
        arb_tricky_char().prop_map(Owned::Char),
        prop::collection::vec(any::<u8>(), 0..8).prop_map(Owned::Bytes),
        arb_text().prop_map(Owned::Display),
        arb_text().prop_map(Owned::Debug),
    ]
}

#[derive(Debug, Clone)]
struct Case {
    level: Level,
    target: String,
    message: String,
    timestamp: Option<u128>,
    /// Keys are made unique by a `#<index>` suffix so every field can be
    /// looked up after parsing.
    fields: Vec<(String, Owned)>,
}

fn arb_case() -> impl Strategy<Value = Case> {
    (
        arb_level(),
        arb_text(),
        arb_text(),
        prop::option::of(any::<u128>()),
        prop::collection::vec((arb_text(), arb_owned()), 0..8),
    )
        .prop_map(|(level, target, message, timestamp, raw)| Case {
            level,
            target,
            message,
            timestamp,
            fields: raw
                .into_iter()
                .enumerate()
                .map(|(i, (k, v))| (format!("{k}#{i}"), v))
                .collect(),
        })
}

fn render<F: Format>(format: F, case: &Case) -> String {
    let fields: Vec<Field<'_>> = case
        .fields
        .iter()
        .map(|(k, v)| Field::new(k.as_str(), v.as_value()))
        .collect();
    let mut meta = Metadata::new(case.level, &case.target);
    if let Some(ts) = case.timestamp {
        meta = meta.with_timestamp(ts);
    }
    let record = Record::new(meta, &case.message, &fields);
    let mut out = String::new();
    format.write_record_generic(&record, &mut out).unwrap();
    let mut via_dyn = String::new();
    format.write_record(&record, &mut via_dyn).unwrap();
    assert_eq!(out, via_dyn, "static and dynamic dispatch must agree");
    out
}

fn is_forbidden(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn assert_single_safe_line(out: &str) -> Result<(), TestCaseError> {
    prop_assert!(out.ends_with('\n'), "missing newline: {:?}", out);
    let body = &out[..out.len() - 1];
    prop_assert!(
        !body.chars().any(is_forbidden),
        "raw control or separator in {:?}",
        out
    );
    Ok(())
}

/// A strict logfmt parser for the dialect `LogfmtFormat` writes:
/// `key=value` pairs separated by one space; bare values contain no
/// space, `"` or `=`; quoted values use JSON escapes.
fn parse_logfmt(line: &str) -> Option<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        let mut key = String::new();
        loop {
            match chars.next()? {
                '=' => break,
                ' ' | '"' => return None,
                c => key.push(c),
            }
        }
        if key.is_empty() {
            return None;
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            let _ = chars.next();
            loop {
                match chars.next()? {
                    '"' => break,
                    '\\' => match chars.next()? {
                        '"' => value.push('"'),
                        '\\' => value.push('\\'),
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        'b' => value.push('\u{8}'),
                        'f' => value.push('\u{c}'),
                        'u' => {
                            let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                            value.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                        }
                        _ => return None,
                    },
                    c => value.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ' ' {
                    break;
                }
                if c == '"' || c == '=' {
                    return None;
                }
                value.push(c);
                let _ = chars.next();
            }
        }
        pairs.push((key, value));
        match chars.next() {
            None => return Some(pairs),
            Some(' ') => {}
            Some(_) => return None,
        }
    }
}

fn sanitized_key(key: &str) -> String {
    if key.is_empty() {
        return "_".to_owned();
    }
    key.chars()
        .map(|c| {
            if c.is_whitespace() || is_forbidden(c) || c == '=' || c == '"' {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// 512 cases by default; `PROPTEST_CASES` overrides it for deep runs.
fn config() -> ProptestConfig {
    let mut config = ProptestConfig::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 512;
    }
    config
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn json_is_valid_single_line_and_round_trips(case in arb_case()) {
        let out = render(JsonFormat::new(), &case);
        assert_single_safe_line(&out)?;
        prop_assert_eq!(out.matches('\n').count(), 1);
        let v: serde_json::Value = serde_json::from_str(&out)
            .map_err(|e| TestCaseError::fail(format!("{e}: {out:?}")))?;
        prop_assert_eq!(v["level"].as_str(), Some(case.level.as_str()));
        prop_assert_eq!(v["target"].as_str(), Some(case.target.as_str()));
        prop_assert_eq!(v["message"].as_str(), Some(case.message.as_str()));
        if let Some(ts) = case.timestamp {
            // Numbers beyond u64 come back as floats; check the text.
            let needle = format!("\"timestamp\":{ts}");
            prop_assert!(out.contains(&needle));
        }
        for (key, value) in &case.fields {
            let got = &v[key.as_str()];
            match value {
                Owned::Null => prop_assert!(got.is_null()),
                Owned::Bool(b) => prop_assert_eq!(got.as_bool(), Some(*b)),
                Owned::I64(n) => prop_assert_eq!(got.as_i64(), Some(*n)),
                Owned::U64(n) => prop_assert_eq!(got.as_u64(), Some(*n)),
                Owned::I128(n) => {
                    let needle = format!(":{n}");
                    prop_assert!(out.contains(&needle));
                }
                Owned::U128(n) => {
                    let needle = format!(":{n}");
                    prop_assert!(out.contains(&needle));
                }
                Owned::F64(n) if n.is_finite() => {
                    prop_assert!(got.is_f64(), "{} not a float in {}", n, out);
                    prop_assert_eq!(got.as_f64().map(f64::to_bits), Some(n.to_bits()));
                }
                Owned::F64(_) => prop_assert!(got.is_null()),
                other => {
                    let expected = other.expected_text();
                    prop_assert_eq!(got.as_str(), expected.as_deref());
                }
            }
        }
    }

    #[test]
    fn json_pretty_and_rfc3339_variants_are_valid(case in arb_case()) {
        let out = render(JsonFormat::new().pretty(true).timestamp_rfc3339(true), &case);
        let v: serde_json::Value = serde_json::from_str(&out)
            .map_err(|e| TestCaseError::fail(format!("{e}: {out:?}")))?;
        if case.timestamp.is_some() {
            let ts = v["timestamp"].as_str().unwrap_or_default();
            prop_assert_eq!(ts.len(), 30, "{}", ts);
            prop_assert!(ts.ends_with('Z'));
        }
    }

    #[test]
    fn logfmt_parses_and_round_trips(case in arb_case()) {
        let out = render(LogfmtFormat::new(), &case);
        assert_single_safe_line(&out)?;
        let pairs = parse_logfmt(&out[..out.len() - 1])
            .ok_or_else(|| TestCaseError::fail(format!("unparseable: {out:?}")))?;
        let mut it = pairs.iter();
        if case.timestamp.is_some() {
            prop_assert_eq!(&it.next().unwrap().0, "timestamp");
        }
        let level = it.next().unwrap();
        prop_assert_eq!((level.0.as_str(), level.1.as_str()), ("level", case.level.as_str()));
        let target = it.next().unwrap();
        prop_assert_eq!((target.0.as_str(), target.1.as_str()), ("target", case.target.as_str()));
        let message = it.next().unwrap();
        prop_assert_eq!((message.0.as_str(), message.1.as_str()), ("message", case.message.as_str()));
        for ((key, value), (got_key, got_value)) in case.fields.iter().zip(it) {
            prop_assert_eq!(got_key, &sanitized_key(key));
            if let Some(text) = value.expected_text() {
                prop_assert_eq!(got_value, &text);
            }
        }
    }

    #[test]
    fn human_is_one_safe_line(case in arb_case()) {
        let out = render(HumanFormat::new(), &case);
        assert_single_safe_line(&out)?;
        prop_assert!(out.contains(case.level.as_str_upper()));
        let colored = render(HumanFormat::new().color(true).show_source_location(true), &case);
        prop_assert_eq!(colored.matches('\n').count(), 1);
    }

    #[test]
    fn reserved_keys_never_shadow_metadata(
        key in prop::sample::select(vec!["timestamp", "level", "target", "message", "file", "line"]),
        value in arb_text(),
    ) {
        let fields = [Field::new(key, value.as_str())];
        let record = Record::new(Metadata::new(Level::Warn, "t").with_location("f", 1), "m", &fields);
        let mut out = String::new();
        JsonFormat::new().write_record_generic(&record, &mut out).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        prop_assert_eq!(&v["level"], "warn");
        prop_assert_eq!(&v["message"], "m");
        prop_assert_eq!(v[format!("fields.{key}")].as_str(), Some(value.as_str()));
        out.clear();
        LogfmtFormat::new().write_record_generic(&record, &mut out).unwrap();
        let pairs = parse_logfmt(out.trim_end()).unwrap();
        let prefixed = format!("fields.{key}");
        let user = pairs.iter().find(|(k, _)| *k == prefixed).map(|(_, v)| v.as_str());
        prop_assert_eq!(user, Some(value.as_str()));
        prop_assert!(pairs.iter().filter(|(k, _)| k == key).count() <= 1);
    }

    #[test]
    fn filter_matches_most_specific_prefix_oracle(
        default in arb_level(),
        rules in prop::collection::vec(
            (prop::collection::vec(prop::sample::select(vec!["a", "b", "ab"]), 1..4), arb_level()),
            0..8,
        ),
        query in prop::collection::vec(prop::sample::select(vec!["a", "b", "ab", "c"]), 1..5),
        dotted in any::<bool>(),
        level in arb_level(),
    ) {
        let rules: Vec<(String, Level)> =
            rules.into_iter().map(|(segs, l)| (segs.join("::"), l)).collect();
        let sep = if dotted { "." } else { "::" };
        let target = query.join(sep);

        // Oracle: the longest matching rule; for repeated targets the
        // last occurrence counts.
        let matches = |prefix: &str| {
            target == prefix
                || target
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.starts_with("::") || rest.starts_with('.'))
        };
        let mut best: Option<(usize, Level)> = None;
        for (t, l) in &rules {
            if matches(t) && best.is_none_or(|(len, _)| t.len() >= len) {
                best = Some((t.len(), *l));
            }
        }
        let expected = best.map_or(default, |(_, l)| l);

        let mut built = Filter::new(default);
        for (t, l) in &rules {
            built = built.with_rule(t.clone(), *l);
        }
        let mut directive = vec![default.as_str().to_owned()];
        directive.extend(rules.iter().map(|(t, l)| format!("{t}={}", l.as_str())));
        let parsed = Filter::parse(&directive.join(",")).unwrap();

        prop_assert_eq!(built.threshold_for(&target), expected);
        prop_assert_eq!(parsed.threshold_for(&target), expected);
        prop_assert_eq!(built.is_enabled(&target, level), level >= expected && expected != Level::Off);
        prop_assert_eq!(parsed.is_enabled(&target, level), built.is_enabled(&target, level));
    }

    #[test]
    fn filter_parse_never_panics(s in any::<String>()) {
        let _ = Filter::parse(&s);
    }

    #[test]
    fn level_parse_round_trip(level in arb_level()) {
        prop_assert_eq!(Level::from_str(level.as_str()).unwrap(), level);
        prop_assert_eq!(Level::from_str(level.as_str_upper()).unwrap(), level);
    }

    #[test]
    fn level_from_str_rejects_non_levels(s in "[a-z]{1,10}") {
        let known = ["off", "trace", "debug", "info", "warn", "warning", "error", "err"];
        prop_assert_eq!(Level::from_str(&s).is_ok(), known.contains(&s.as_str()));
    }

    #[test]
    fn context_matches_model(ops in prop::collection::vec((any::<bool>(), 0..4_usize, any::<u8>()), 0..40)) {
        // Model: live entries (key, value) in push order, capped at
        // MAX_CONTEXT_SLOTS by evicting the oldest; visible fields are the
        // newest entry per key, ordered by that entry's position.
        context::clear();
        let keys = ["a", "b", "c", "d"];
        let mut guards: Vec<(u64, context::ContextGuard)> = Vec::new();
        let mut model: Vec<(u64, &str, u8)> = Vec::new();
        let mut next = 0_u64;
        for (push, k, v) in ops {
            if push || guards.is_empty() {
                next += 1;
                guards.push((next, context::with_field(keys[k], v)));
                if model.len() >= context::MAX_CONTEXT_SLOTS {
                    model.remove(0);
                }
                model.push((next, keys[k], v));
            } else {
                let (id, guard) = guards.remove(k % guards.len());
                drop(guard);
                model.retain(|(mid, _, _)| *mid != id);
            }
            let mut expected: Vec<(&str, u8)> = Vec::new();
            for (_, key, value) in model.iter().rev() {
                if !expected.iter().any(|(k, _)| k == key) {
                    expected.push((key, *value));
                }
            }
            expected.reverse();
            let actual: Vec<(String, Value<'static>)> = context::with_snapshot(|f| {
                f.iter().map(|f| (f.key.to_owned(), match f.value {
                    Value::U64(n) => Value::U64(n),
                    _ => Value::Null,
                })).collect()
            });
            let actual: Vec<(&str, u8)> = actual
                .iter()
                .map(|(k, v)| (k.as_str(), match v { Value::U64(n) => *n as u8, _ => 255 }))
                .collect();
            prop_assert_eq!(actual, expected);
        }
        drop(guards);
        prop_assert_eq!(context::len(), 0);
    }

    #[test]
    fn rfc3339_matches_naive_calendar(secs in 0_u64..=253_402_300_799, nanos in 0_u32..1_000_000_000) {
        let ts = u128::from(secs) * 1_000_000_000 + u128::from(nanos);
        let record = Record::new(Metadata::new(Level::Info, "t").with_timestamp(ts), "m", &[]);
        let mut out = String::new();
        HumanFormat::new().write_record_generic(&record, &mut out).unwrap();
        let leap = |y: u64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let mut days = secs / 86_400;
        let mut year = 1970;
        loop {
            let len = if leap(year) { 366 } else { 365 };
            if days < len { break; }
            days -= len;
            year += 1;
        }
        let month_lengths = [31, if leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let mut month = 1;
        for len in month_lengths {
            if days < len { break; }
            days -= len;
            month += 1;
        }
        let rem = secs % 86_400;
        let expected = format!(
            "{year:04}-{month:02}-{:02}T{:02}:{:02}:{:02}.{nanos:09}Z ",
            days + 1, rem / 3600, (rem % 3600) / 60, rem % 60
        );
        prop_assert!(out.starts_with(&expected), "{} vs {}", out, expected);
    }
}

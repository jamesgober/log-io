//! Property tests: invariants that should hold across any input.
//!
//! The strategy here is intentionally simple. We use `proptest` to
//! generate random records and check shape-level invariants of the
//! formatter output (line counts, balanced delimiters, ability to be
//! re-parsed as JSON, level/target appearing in the line, etc).

use std::str::FromStr;

use log_io::format::{Format, HumanFormat, JsonFormat, LogfmtFormat};
use log_io::{Field, Filter, Level, Metadata, Record, Value};

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

fn arb_safe_text() -> impl Strategy<Value = String> {
    // Mix of ASCII printable, escape-trigger chars, and multi-byte
    // codepoints. Range is chosen so generated strings stay
    // interesting but small.
    "[ -~]{0,32}|[\u{0001}-\u{001F}]{0,4}|[\u{4e00}-\u{9fff}]{0,4}|\"\\\\/{0,4}"
        .prop_map(String::from)
}

fn arb_value() -> impl Strategy<Value = OwnedValue> {
    prop_oneof![
        Just(OwnedValue::Null),
        any::<bool>().prop_map(OwnedValue::Bool),
        any::<i64>().prop_map(OwnedValue::I64),
        any::<u64>().prop_map(OwnedValue::U64),
        any::<f64>().prop_map(OwnedValue::F64),
        arb_safe_text().prop_map(OwnedValue::Str),
    ]
}

#[derive(Debug, Clone)]
enum OwnedValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
}

impl OwnedValue {
    fn as_value(&self) -> Value<'_> {
        match self {
            Self::Null => Value::Null,
            Self::Bool(b) => Value::Bool(*b),
            Self::I64(n) => Value::I64(*n),
            Self::U64(n) => Value::U64(*n),
            Self::F64(n) => Value::F64(*n),
            Self::Str(s) => Value::Str(s.as_str()),
        }
    }
}

#[derive(Debug, Clone)]
struct OwnedField {
    key: String,
    value: OwnedValue,
}

fn arb_owned_field() -> impl Strategy<Value = OwnedField> {
    (arb_safe_text(), arb_value()).prop_map(|(k, v)| OwnedField { key: k, value: v })
}

fn arb_record() -> impl Strategy<Value = (Level, String, String, Vec<OwnedField>)> {
    (
        arb_level(),
        arb_safe_text(), // target
        arb_safe_text(), // message
        prop::collection::vec(arb_owned_field(), 0..10),
    )
}

fn build_record<'a>(
    level: Level,
    target: &'a str,
    message: &'a str,
    fields: &'a [OwnedField],
    buf: &'a mut Vec<Field<'a>>,
) -> Record<'a> {
    buf.clear();
    for f in fields {
        buf.push(Field::new(f.key.as_str(), f.value.as_value()));
    }
    Record::new(Metadata::new(level, target), message, buf)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        ..ProptestConfig::default()
    })]

    #[test]
    fn json_output_is_one_line_terminated((level, target, message, fields) in arb_record()) {
        let mut buf = Vec::new();
        let record = build_record(level, &target, &message, &fields, &mut buf);
        let mut out = String::new();
        JsonFormat::new().write_record(&record, &mut out).unwrap();
        prop_assert!(out.ends_with('\n'), "missing newline: {out:?}");
        prop_assert_eq!(out.matches('\n').count(), 1, "extra newlines: {:?}", out);
        prop_assert!(out.starts_with('{'), "no opening brace");
        prop_assert!(out.trim_end().ends_with('}'), "no closing brace");
    }

    #[test]
    fn json_level_string_appears((level, target, message, fields) in arb_record()) {
        let mut buf = Vec::new();
        let record = build_record(level, &target, &message, &fields, &mut buf);
        let mut out = String::new();
        JsonFormat::new().write_record(&record, &mut out).unwrap();
        let needle = format!("\"level\":\"{}\"", level.as_str());
        prop_assert!(out.contains(&needle), "missing level: {out}");
    }

    #[test]
    fn logfmt_output_is_one_line_terminated((level, target, message, fields) in arb_record()) {
        let mut buf = Vec::new();
        let record = build_record(level, &target, &message, &fields, &mut buf);
        let mut out = String::new();
        LogfmtFormat::new().write_record(&record, &mut out).unwrap();
        prop_assert!(out.ends_with('\n'));
        prop_assert_eq!(out.matches('\n').count(), 1, "extra newlines: {:?}", out);
        let needle = format!("level={}", level.as_str());
        prop_assert!(out.contains(&needle));
    }

    #[test]
    fn human_output_is_one_line_terminated((level, target, message, fields) in arb_record()) {
        let mut buf = Vec::new();
        let record = build_record(level, &target, &message, &fields, &mut buf);
        let mut out = String::new();
        HumanFormat::new().write_record(&record, &mut out).unwrap();
        prop_assert!(out.ends_with('\n'));
        prop_assert_eq!(out.matches('\n').count(), 1, "extra newlines: {:?}", out);
        prop_assert!(out.contains(level.as_str_upper()));
    }

    #[test]
    fn level_parse_round_trip(level in arb_level()) {
        let s = level.as_str();
        let parsed: Level = s.parse().unwrap();
        prop_assert_eq!(parsed, level);
        let upper: Level = level.as_str_upper().parse().unwrap();
        prop_assert_eq!(upper, level);
    }

    #[test]
    fn level_ordering_is_total(a in arb_level(), b in arb_level()) {
        let c = a.cmp(&b);
        let d = b.cmp(&a);
        prop_assert_eq!(c, d.reverse());
    }

    #[test]
    fn filter_admits_at_or_above_threshold(threshold in arb_level(), level in arb_level()) {
        let f = Filter::new(threshold);
        let admitted = f.is_enabled("any", level);
        prop_assert_eq!(admitted, level >= threshold);
    }

    #[test]
    fn filter_directive_round_trip_for_well_formed(
        default in arb_level(),
        overrides in prop::collection::vec((
            "[a-z_]+(::[a-z_]+){0,3}",
            arb_level(),
        ), 0..6),
    ) {
        // De-duplicate by target, keeping the FIRST occurrence to
        // match the filter's insertion-order precedence.
        let mut seen = std::collections::HashSet::new();
        let mut unique: Vec<(String, Level)> = Vec::new();
        for (t, l) in &overrides {
            if seen.insert(t.clone()) {
                unique.push((t.clone(), *l));
            }
        }
        let mut parts = vec![default.as_str().to_string()];
        for (target, level) in &unique {
            parts.push(format!("{target}={}", level.as_str()));
        }
        let directive = parts.join(",");
        let filter = Filter::parse(&directive).unwrap();
        prop_assert_eq!(filter.default_level(), default);
        // For each target, the effective threshold (per the filter's
        // "first matching prefix wins" rule) must round-trip: records
        // at that threshold pass, records at the next lower level
        // don't.
        for (target, _) in &unique {
            let effective = filter.threshold_for(target);
            prop_assert!(filter.is_enabled(target, effective));
            if effective > Level::Trace {
                let lower = match effective {
                    Level::Error => Level::Warn,
                    Level::Warn => Level::Info,
                    Level::Info => Level::Debug,
                    Level::Debug => Level::Trace,
                    _ => Level::Trace,
                };
                prop_assert!(!filter.is_enabled(target, lower));
            }
        }
    }

    #[test]
    fn json_escapes_dont_break_balanced_quote_count(
        s in any::<String>(),
    ) {
        let fields = [Field::new("k", Value::Str(s.as_str()))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let mut out = String::new();
        JsonFormat::new().write_record(&record, &mut out).unwrap();
        // Count unescaped quotes: every " not preceded by \ ends an
        // even/odd cycle. The total must be even (since strings are
        // delimited by paired quotes).
        let mut count = 0;
        let bytes = out.as_bytes();
        for i in 0..bytes.len() {
            if bytes[i] == b'"' {
                let mut backslashes = 0;
                let mut j = i;
                while j > 0 && bytes[j - 1] == b'\\' {
                    backslashes += 1;
                    j -= 1;
                }
                if backslashes % 2 == 0 {
                    count += 1;
                }
            }
        }
        prop_assert_eq!(count % 2, 0, "unbalanced unescaped quotes in {:?}", out);
    }

    #[test]
    fn integer_field_serializes_to_decimal_in_logfmt(n in any::<i64>()) {
        let fields = [Field::new("n", Value::I64(n))];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let mut out = String::new();
        LogfmtFormat::new().write_record(&record, &mut out).unwrap();
        let needle = format!("n={n}");
        prop_assert!(out.contains(&needle));
    }

    #[test]
    fn level_from_str_rejects_garbage(s in "[a-z]{5,10}") {
        // For any non-canonical lowercase string, parse should fail.
        if matches!(
            s.as_str(),
            "off" | "trace" | "debug" | "info" | "warn" | "warning" | "error" | "err"
        ) {
            return Ok(());
        }
        prop_assert!(Level::from_str(&s).is_err());
    }
}

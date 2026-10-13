#![no_main]
//! Fuzz target: `LogfmtFormat` on arbitrary records.
//!
//! Invariants:
//! * never panics;
//! * exactly one line with no raw control, C1, separator or bidi
//!   character;
//! * the line parses with the reference logfmt parser, and level,
//!   target, message and every string field value round-trip.

use libfuzzer_sys::fuzz_target;
use log_io::format::LogfmtFormat;
use log_io_fuzz::{FuzzRecord, FuzzValue, assert_single_safe_line, parse_logfmt, render};

fuzz_target!(|input: FuzzRecord| {
    input.with_record(|record| {
        for format in [LogfmtFormat::new(), LogfmtFormat::new().timestamp_rfc3339(true)] {
            let out = render(&format, record);
            assert_single_safe_line(&out);
            let pairs = parse_logfmt(&out[..out.len() - 1]).expect("parseable logfmt");
            let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
            assert_eq!(get("level"), Some(input.level().as_str()));
            assert_eq!(get("target"), Some(input.target.as_str()));
            assert_eq!(get("message"), Some(input.message.as_str()));
            // Fields are written after the metadata, in order.
            let tail = &pairs[pairs.len() - input.fields().count()..];
            for ((_, value), (_, got)) in input.fields().zip(tail) {
                if let FuzzValue::Str(s) | FuzzValue::Display(s) = value {
                    assert_eq!(got, s);
                }
            }
        }
    });
});

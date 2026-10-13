#![no_main]
//! Fuzz target: `JsonFormat` on arbitrary records.
//!
//! Invariants:
//! * never panics;
//! * compact output is exactly one line with no raw control, C1,
//!   separator or bidi character;
//! * compact and pretty output both parse with `serde_json`;
//! * level, target and message round-trip, and no user field can
//!   replace them.

use libfuzzer_sys::fuzz_target;
use log_io::format::JsonFormat;
use log_io_fuzz::{FuzzRecord, assert_single_safe_line, render};

fuzz_target!(|input: FuzzRecord| {
    input.with_record(|record| {
        let out = render(&JsonFormat::new(), record);
        assert_single_safe_line(&out);
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(v["level"].as_str(), Some(input.level().as_str()));
        assert_eq!(v["target"].as_str(), Some(input.target.as_str()));
        assert_eq!(v["message"].as_str(), Some(input.message.as_str()));

        let pretty = render(&JsonFormat::new().pretty(true).timestamp_rfc3339(true), record);
        let p: serde_json::Value = serde_json::from_str(&pretty).expect("valid pretty JSON");
        assert_eq!(p["message"], v["message"]);
    });
});

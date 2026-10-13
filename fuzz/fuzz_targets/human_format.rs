#![no_main]
//! Fuzz target: `HumanFormat` on arbitrary records.
//!
//! Invariants:
//! * never panics;
//! * exactly one line with no raw control, C1, separator or bidi
//!   character (colour off), and exactly one line with colour on.

use libfuzzer_sys::fuzz_target;
use log_io::format::HumanFormat;
use log_io_fuzz::{FuzzRecord, assert_single_safe_line, render};

fuzz_target!(|input: FuzzRecord| {
    input.with_record(|record| {
        let out = render(&HumanFormat::new().show_source_location(true), record);
        assert_single_safe_line(&out);
        assert!(out.contains(input.level().as_str_upper()));

        let colored = render(&HumanFormat::new().color(true), record);
        assert_eq!(colored.matches('\n').count(), 1, "{colored:?}");
    });
});

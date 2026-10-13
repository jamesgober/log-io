#![no_main]
//! Fuzz target: filter directive parser.
//!
//! Invariants:
//! * `Filter::parse` never panics on arbitrary UTF-8;
//! * a parsed filter is internally consistent: `is_enabled` agrees with
//!   `threshold_for`, nothing below `min_level` is admitted, and rules
//!   are ordered most specific first;
//! * an error always names a segment of the input.

use libfuzzer_sys::fuzz_target;
use log_io::{Filter, Level};

fuzz_target!(|data: (&str, &str)| {
    let (directive, target) = data;
    match Filter::parse(directive) {
        Ok(filter) => {
            let threshold = filter.threshold_for(target);
            for level in Level::ALL {
                assert_eq!(filter.is_enabled(target, level), level >= threshold);
                if level < filter.min_level() {
                    assert!(!filter.is_enabled(target, level));
                }
            }
            let lengths: Vec<usize> = filter.rules().iter().map(|r| r.target.len()).collect();
            assert!(lengths.windows(2).all(|w| w[0] >= w[1]));
        }
        Err(e) => {
            assert!(directive.contains(e.segment()) || e.segment() == directive);
            let _ = e.to_string();
        }
    }
});

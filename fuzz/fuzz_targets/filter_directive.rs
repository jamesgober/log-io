#![no_main]
//! Fuzz target: filter directive parser.
//!
//! Invariants:
//! * `Filter::parse` must never panic on arbitrary input.
//! * When it succeeds, the resulting filter must round-trip a
//!   non-empty target through `is_enabled` without panicking.

use libfuzzer_sys::fuzz_target;
use log_io::{Filter, Level};

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(filter) = Filter::parse(s) {
        let _ = filter.is_enabled("anything", Level::Info);
        let _ = filter.is_enabled("app::auth", Level::Trace);
        let _ = filter.default_level();
        let _ = filter.rules();
    }
});

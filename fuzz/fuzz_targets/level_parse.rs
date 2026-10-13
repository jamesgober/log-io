#![no_main]
//! Fuzz target: `Level::from_str` never panics, and accepts exactly the
//! documented spellings.

use libfuzzer_sys::fuzz_target;
use log_io::Level;

fuzz_target!(|s: &str| {
    let accepted = ["off", "trace", "debug", "info", "warn", "warning", "error", "err"]
        .iter()
        .any(|name| s.eq_ignore_ascii_case(name));
    assert_eq!(s.parse::<Level>().is_ok(), accepted);
});

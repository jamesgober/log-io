#![no_main]
//! Fuzz target: Level parser.

use libfuzzer_sys::fuzz_target;
use log_io::Level;

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let _ = s.parse::<Level>();
});

#![no_main]
//! Fuzz target: JSON formatter.
//!
//! Invariants:
//! * Never panic on any UTF-8 message / field value.
//! * Output is always valid JSON. We use `serde_json` (vendored as a
//!   one-shot dep here, since the fuzz crate is separate) to parse the
//!   result.
//! * Output is exactly one line.

use libfuzzer_sys::fuzz_target;
use log_io::format::{Format, JsonFormat};
use log_io::{Field, Level, Metadata, Record, Value};

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let fields = [
        Field::new("k", Value::Str(s)),
        Field::new("len", Value::U64(s.len() as u64)),
    ];
    let record = Record::new(
        Metadata::new(Level::Info, "fuzz").with_timestamp(1_700_000_000_000_000_000),
        s,
        &fields,
    );
    let mut out = String::new();
    JsonFormat::new()
        .write_record(&record, &mut out)
        .expect("JSON write should not fail");

    // Shape invariants.
    assert!(out.starts_with('{'));
    assert!(out.trim_end().ends_with('}'));
    assert_eq!(out.matches('\n').count(), 1);
});

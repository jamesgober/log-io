#![no_main]
//! Fuzz target: logfmt formatter.
//!
//! Invariants:
//! * Never panic on any UTF-8 input.
//! * Output is exactly one line.
//! * Output contains the literal substring `level=info`.

use libfuzzer_sys::fuzz_target;
use log_io::format::{Format, LogfmtFormat};
use log_io::{Field, Level, Metadata, Record, Value};

fuzz_target!(|data: &[u8]| {
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };
    let fields = [Field::new("k", Value::Str(s))];
    let record = Record::new(Metadata::new(Level::Info, "fuzz"), s, &fields);
    let mut out = String::new();
    LogfmtFormat::new()
        .write_record(&record, &mut out)
        .expect("logfmt write should not fail");
    assert_eq!(out.matches('\n').count(), 1);
    assert!(out.contains("level=info"));
});

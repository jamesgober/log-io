//! Micro-benchmarks for the three built-in formatters.
//!
//! Run with `cargo bench --bench format`. The bench harness is a hand-
//! rolled loop because this crate intentionally has zero runtime
//! dependencies; numbers report ns/iter for each format.

use std::hint::black_box;
use std::time::Instant;

use log_io::format::{Format, HumanFormat, JsonFormat, LogfmtFormat};
use log_io::{Field, Level, Metadata, Record, Value};

const ITERS: u32 = 200_000;

fn time<F: FnMut()>(name: &str, mut f: F) {
    // Warm up.
    for _ in 0..1000 {
        f();
    }
    let start = Instant::now();
    for _ in 0..ITERS {
        f();
    }
    let elapsed = start.elapsed();
    let per = elapsed.as_nanos() / u128::from(ITERS);
    println!("{name:>20}: {per:>5} ns/record");
}

fn main() {
    let fields = [
        Field::new("port", Value::U64(8080)),
        Field::new("host", Value::Str("0.0.0.0")),
        Field::new("path", Value::Str("/api/users")),
        Field::new("status", Value::U64(200)),
        Field::new("duration_ms", Value::F64(12.345)),
    ];
    let record = Record::new(
        Metadata::new(Level::Info, "app::http").with_timestamp(1_700_000_000_000_000_000),
        "request completed",
        &fields,
    );

    let json = JsonFormat::new();
    let logfmt = LogfmtFormat::new();
    let human = HumanFormat::new();

    let mut buf = String::with_capacity(256);

    time("json", || {
        buf.clear();
        json.write_record(black_box(&record), &mut buf).unwrap();
        black_box(&buf);
    });
    time("logfmt", || {
        buf.clear();
        logfmt.write_record(black_box(&record), &mut buf).unwrap();
        black_box(&buf);
    });
    time("human", || {
        buf.clear();
        human.write_record(black_box(&record), &mut buf).unwrap();
        black_box(&buf);
    });

    // Empty-record cost: lower bound for serialization overhead.
    let empty = Record::new(Metadata::new(Level::Info, "t"), "msg", &[]);
    time("json (no fields)", || {
        buf.clear();
        json.write_record(black_box(&empty), &mut buf).unwrap();
        black_box(&buf);
    });
}

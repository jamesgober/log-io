//! Formatter-only cost: serialize one record into a reused `String`.
//!
//! Run with `cargo bench --bench format`. These numbers isolate the
//! three built-in formatters from locking, IO, filtering, and clock
//! reads.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use log_io::format::{Format, HumanFormat, JsonFormat, LogfmtFormat};
use log_io::{Field, Level, Metadata, Record, Value};

fn format_benches(c: &mut Criterion) {
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
    let empty = Record::new(Metadata::new(Level::Info, "t"), "msg", &[]);

    let json = JsonFormat::new();
    let logfmt = LogfmtFormat::new();
    let human = HumanFormat::new();
    let mut buf = String::with_capacity(512);

    let mut group = c.benchmark_group("format");
    group.bench_function("json_5_fields", |b| {
        b.iter(|| {
            buf.clear();
            json.write_record_generic(black_box(&record), &mut buf)
                .unwrap();
            black_box(buf.len())
        });
    });
    group.bench_function("logfmt_5_fields", |b| {
        b.iter(|| {
            buf.clear();
            logfmt
                .write_record_generic(black_box(&record), &mut buf)
                .unwrap();
            black_box(buf.len())
        });
    });
    group.bench_function("human_5_fields", |b| {
        b.iter(|| {
            buf.clear();
            human
                .write_record_generic(black_box(&record), &mut buf)
                .unwrap();
            black_box(buf.len())
        });
    });
    group.bench_function("json_no_fields", |b| {
        b.iter(|| {
            buf.clear();
            json.write_record_generic(black_box(&empty), &mut buf)
                .unwrap();
            black_box(buf.len())
        });
    });
    group.bench_function("json_5_fields_dyn_writer", |b| {
        b.iter(|| {
            buf.clear();
            let w: &mut dyn core::fmt::Write = &mut buf;
            json.write_record(black_box(&record), w).unwrap();
            black_box(buf.len())
        });
    });
    group.finish();
}

criterion_group!(benches, format_benches);
criterion_main!(benches);

//! End-to-end pipeline cost: `Logger` -> filter -> format -> sink.
//!
//! Run with `cargo bench --bench pipeline`. Every writer-backed case
//! writes into a discarding `io::Write`, so the numbers measure the
//! pipeline itself rather than disk or terminal throughput.
//!
//! Cases:
//!
//! * `dispatch_only` - a `NullSink` that never formats. Filter check,
//!   record assembly and dispatch overhead.
//! * `below_threshold` - the record is rejected by the filter.
//! * `json_writer` / `logfmt_writer` - the full pipeline with three
//!   call-site fields, timestamps and context capture disabled.
//! * `json_writer_no_fields` - the same with no fields.
//! * `default_fields_2` - two logger-level default fields merged in.
//! * `context_empty` / `context_2` - thread-local context capture
//!   enabled with zero or two context fields installed.
//! * `default_config` - `Logger::builder()` defaults: wall-clock
//!   timestamp on every record plus context capture. This is the
//!   cost a typical user pays.
//! * `macro_info` - the `info!` macro with three fields on the
//!   default configuration.
//! * `non_blocking_error_drop_newest` - an `Error` record (the default
//!   flush level) through a `NonBlocking` sink with
//!   `Overflow::DropNewest`: format, enqueue and hand off the flush.

use std::hint::black_box;
use std::io::{self, Write};

use criterion::{Criterion, criterion_group, criterion_main};
use log_io::format::{JsonFormat, LogfmtFormat};
use log_io::sink::{NonBlocking, NullSink, Overflow};
use log_io::{Field, Level, Logger, Value, context};

struct DevNull;

impl Write for DevNull {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn quiet() -> log_io::LoggerBuilder {
    Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
}

fn pipeline_benches(c: &mut Criterion) {
    let null = quiet().with_sink(NullSink::new()).build();
    let filtered = Logger::builder()
        .level(Level::Error)
        .no_timestamps()
        .no_context()
        .with_sink(NullSink::new())
        .build();
    let json = quiet().writer(DevNull, JsonFormat::new()).build();
    let logfmt = quiet().writer(DevNull, LogfmtFormat::new()).build();
    let defaults = quiet()
        .with_default_field("service", "api")
        .with_default_field("version", "1.2.3")
        .writer(DevNull, JsonFormat::new())
        .build();
    let with_ctx = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .writer(DevNull, JsonFormat::new())
        .build();
    let default_cfg = Logger::builder()
        .level(Level::Trace)
        .writer(DevNull, JsonFormat::new())
        .build();

    let fields = [
        Field::new("port", Value::U64(8080)),
        Field::new("host", Value::Str("0.0.0.0")),
        Field::new("path", Value::Str("/api/users")),
    ];

    let background = quiet()
        .with_sink(
            NonBlocking::builder(DevNull, JsonFormat::new())
                .overflow(Overflow::DropNewest)
                .build()
                .expect("spawn the bench writer thread"),
        )
        .build();

    let mut group = c.benchmark_group("pipeline");
    group.bench_function("dispatch_only", |b| {
        b.iter(|| null.log(Level::Info, black_box("hello"), &fields));
    });
    group.bench_function("below_threshold", |b| {
        b.iter(|| filtered.log(Level::Info, black_box("hello"), &fields));
    });
    group.bench_function("json_writer", |b| {
        b.iter(|| json.log(Level::Info, black_box("hello"), &fields));
    });
    group.bench_function("logfmt_writer", |b| {
        b.iter(|| logfmt.log(Level::Info, black_box("hello"), &fields));
    });
    group.bench_function("json_writer_no_fields", |b| {
        b.iter(|| json.log(Level::Info, black_box("hello"), &[]));
    });
    group.bench_function("default_fields_2", |b| {
        b.iter(|| defaults.log(Level::Info, black_box("hello"), &fields));
    });
    context::clear();
    group.bench_function("context_empty", |b| {
        b.iter(|| with_ctx.log(Level::Info, black_box("hello"), &fields));
    });
    {
        let _t = context::with_trace_id("tx-7f3a");
        let _r = context::with_request_id("req-001");
        group.bench_function("context_2", |b| {
            b.iter(|| with_ctx.log(Level::Info, black_box("hello"), &fields));
        });
    }
    group.bench_function("default_config", |b| {
        b.iter(|| default_cfg.log(Level::Info, black_box("hello"), &fields));
    });
    group.bench_function("macro_info", |b| {
        b.iter(|| {
            log_io::info!(
                default_cfg,
                "hello",
                port = 8080_u64,
                host = "0.0.0.0",
                path = "/api/users"
            );
        });
    });
    group.bench_function("non_blocking_error_drop_newest", |b| {
        b.iter(|| background.log(Level::Error, black_box("hello"), &fields));
    });
    group.finish();
    background.shutdown().expect("drain the bench writer");
}

criterion_group!(benches, pipeline_benches);
criterion_main!(benches);

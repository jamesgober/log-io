//! End-to-end pipeline benchmark: Logger -> Filter -> Format -> Sink.
//!
//! Three paths are measured:
//!
//! 1. `dispatch_only` - sink discards the record before formatting.
//!    Measures filter + context snapshot + dispatch overhead.
//! 2. `below_threshold` - record filtered out. Measures the gate.
//! 3. `* + writer` - the full pipeline, format included, writing to
//!    a non-allocating discarding `Write`.

use std::hint::black_box;
use std::io::{self, Write};
use std::time::Instant;

use log_io::format::{JsonFormat, LogfmtFormat};
use log_io::sink::NullSink;
use log_io::{Field, Level, Logger, Value};

const ITERS: u32 = 200_000;

struct DevNull;

impl Write for DevNull {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn time<F: FnMut()>(name: &str, mut f: F) {
    for _ in 0..1000 {
        f();
    }
    let start = Instant::now();
    for _ in 0..ITERS {
        f();
    }
    let elapsed = start.elapsed();
    let per = elapsed.as_nanos() / u128::from(ITERS);
    println!("{name:>34}: {per:>5} ns/call");
}

fn main() {
    let logger_null = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(NullSink::new())
        .build();

    let logger_filtered = Logger::builder()
        .level(Level::Error)
        .no_timestamps()
        .no_context()
        .with_sink(NullSink::new())
        .build();

    let logger_json_writer = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(DevNull, JsonFormat::new())
        .build();

    let logger_logfmt_writer = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(DevNull, LogfmtFormat::new())
        .build();

    let logger_default_fields = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_default_field("service", "api")
        .with_default_field("version", "1.2.3")
        .writer(DevNull, JsonFormat::new())
        .build();

    let logger_with_ctx_capture = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .writer(DevNull, JsonFormat::new())
        .build();

    let fields = [
        Field::new("port", Value::U64(8080)),
        Field::new("host", Value::Str("0.0.0.0")),
        Field::new("path", Value::Str("/api/users")),
    ];

    time("dispatch_only (null sink)", || {
        logger_null.log(Level::Info, black_box("hello"), &fields);
    });
    time("below_threshold (filtered out)", || {
        logger_filtered.log(Level::Info, black_box("hello"), &fields);
    });
    time("json + writer (full pipeline)", || {
        logger_json_writer.log(Level::Info, black_box("hello"), &fields);
    });
    time("logfmt + writer (full pipeline)", || {
        logger_logfmt_writer.log(Level::Info, black_box("hello"), &fields);
    });
    time("json + writer (no fields)", || {
        logger_json_writer.log(Level::Info, black_box("hello"), &[]);
    });
    time("with 2 default fields", || {
        logger_default_fields.log(Level::Info, black_box("hello"), &fields);
    });
    time("with empty context snapshot", || {
        logger_with_ctx_capture.log(Level::Info, black_box("hello"), &fields);
    });
}

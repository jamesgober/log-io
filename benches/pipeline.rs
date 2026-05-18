//! End-to-end pipeline benchmark: Logger -> Filter -> Format -> Sink.
//!
//! Two paths are measured:
//!
//! 1. `*_null`: the sink discards records before formatting, so this is
//!    the cost of filter + context snapshot + dispatch.
//! 2. `*_writer`: a custom sink wraps a discarding writer so the
//!    format step actually runs.

use std::hint::black_box;
use std::io::{self, Write};
use std::time::Instant;

use log_io::format::JsonFormat;
use log_io::sink::WriterSink;
use log_io::{Field, Level, Logger, Value};

const ITERS: u32 = 200_000;

/// Writer that drops everything but exercises the formatter path.
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
    println!("{name:>30}: {per:>5} ns/call");
}

fn main() {
    let logger_null = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .null()
        .json()
        .build();

    let logger_filtered = Logger::builder()
        .level(Level::Error)
        .no_timestamps()
        .no_context()
        .null()
        .json()
        .build();

    let logger_json_writer = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .add_sink(WriterSink::new(DevNull, JsonFormat::new()))
        .null()
        .json()
        .build();

    let logger_logfmt_writer = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .add_sink(WriterSink::new(
            DevNull,
            log_io::format::LogfmtFormat::new(),
        ))
        .null()
        .json()
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
}

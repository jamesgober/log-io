//! Multi-thread throughput: N threads logging into one shared logger.
//!
//! Run with `cargo bench --bench concurrent`. Each sample spawns N
//! scoped threads that each emit a fixed number of records into a
//! single JSON `WriterSink` backed by a discarding writer, so the
//! measurement is dominated by formatting and the sink mutex. The
//! reported throughput is records per second across all threads.

use std::hint::black_box;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use log_io::format::JsonFormat;
use log_io::{Field, Level, Logger, Value};

const RECORDS_PER_THREAD: u64 = 20_000;

struct DevNull;

impl Write for DevNull {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn run(logger: &Logger, threads: usize) -> Duration {
    let start = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let fields = [
                    Field::new("port", Value::U64(8080)),
                    Field::new("host", Value::Str("0.0.0.0")),
                    Field::new("path", Value::Str("/api/users")),
                ];
                for _ in 0..RECORDS_PER_THREAD {
                    logger.log(Level::Info, black_box("tick"), &fields);
                }
            });
        }
    });
    start.elapsed()
}

fn concurrent_benches(c: &mut Criterion) {
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(DevNull, JsonFormat::new())
        .build();

    let mut group = c.benchmark_group("concurrent");
    group.sample_size(20);
    for threads in [1_usize, 2, 4, 8, 16] {
        group.throughput(Throughput::Elements(RECORDS_PER_THREAD * threads as u64));
        group.bench_function(format!("threads_{threads}"), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    total += run(&logger, threads);
                }
                total
            });
        });
    }
    group.finish();
}

criterion_group!(benches, concurrent_benches);
criterion_main!(benches);

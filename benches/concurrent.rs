//! Concurrent throughput benchmark.
//!
//! Spawns N worker threads each logging records to a shared logger
//! backed by a discarding writer. Reports total records/sec.

use std::hint::black_box;
use std::io::{self, Write};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use log_io::format::JsonFormat;
use log_io::{Field, Level, Logger, Value};

const RECORDS_PER_THREAD: u64 = 100_000;

struct DevNull;

impl Write for DevNull {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn run(threads: usize) {
    let logger = Arc::new(
        Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .no_context()
            .writer(DevNull, JsonFormat::new())
            .build(),
    );
    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..threads {
        let logger = Arc::clone(&logger);
        handles.push(thread::spawn(move || {
            let fields = [
                Field::new("port", Value::U64(8080)),
                Field::new("host", Value::Str("0.0.0.0")),
                Field::new("path", Value::Str("/api/users")),
            ];
            for _ in 0..RECORDS_PER_THREAD {
                logger.log(Level::Info, black_box("tick"), &fields);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let elapsed = start.elapsed();
    let total = u128::from(RECORDS_PER_THREAD * threads as u64);
    let per_sec = (total * 1_000_000_000) / elapsed.as_nanos().max(1);
    println!(
        "{threads:>2} threads: {total:>10} records in {:>6} ms => {per_sec:>10} rec/s",
        elapsed.as_millis()
    );
}

fn main() {
    for &n in &[1_usize, 2, 4, 8, 16] {
        run(n);
    }
}

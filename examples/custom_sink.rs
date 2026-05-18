//! Implement a custom [`log_io::Sink`].
//!
//! The sink batches records into a `Vec<String>` in memory so a test
//! or an in-process consumer can inspect them. Production sinks would
//! ship the bytes off-process; the trait is the same either way.

use std::sync::Mutex;

use log_io::format::{Format, JsonFormat};
use log_io::{Level, Logger, Record, Result, Sink};

struct MemorySink {
    format: JsonFormat,
    records: Mutex<Vec<String>>,
}

impl MemorySink {
    fn new() -> Self {
        Self {
            format: JsonFormat::new(),
            records: Mutex::new(Vec::new()),
        }
    }

    fn snapshot(&self) -> Vec<String> {
        self.records.lock().unwrap().clone()
    }
}

impl Sink for MemorySink {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        let mut s = String::with_capacity(128);
        self.format
            .write_record(record, &mut s)
            .map_err(log_io::Error::from)?;
        self.records.lock().unwrap().push(s);
        Ok(())
    }
    fn flush(&self) -> Result<()> {
        Ok(())
    }
}

fn main() {
    let sink = std::sync::Arc::new(MemorySink::new());
    let logger = Logger::builder()
        .level(Level::Info)
        .no_timestamps()
        .no_context()
        .with_sink(sink.clone())
        .build();

    log_io::info!(logger, "one", k = 1_u32);
    log_io::info!(logger, "two", k = 2_u32);

    for record in sink.snapshot() {
        println!("captured: {}", record.trim_end());
    }
}

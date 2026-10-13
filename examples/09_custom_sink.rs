//! A custom sink that keeps formatted records in memory, for tests or
//! in-process consumers.
//!
//! Run with `cargo run --example 09_custom_sink`.

use std::sync::{Arc, Mutex, PoisonError};

use log_io::format::{Format, JsonFormat};
use log_io::{Level, Logger, Record, Result, Sink};

#[derive(Default)]
struct MemorySink {
    records: Mutex<Vec<String>>,
}

impl MemorySink {
    fn snapshot(&self) -> Vec<String> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Sink for MemorySink {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        let mut line = String::with_capacity(128);
        JsonFormat::new().write_record_generic(record, &mut line)?;
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }
}

fn main() {
    let sink = Arc::new(MemorySink::default());
    let logger = Logger::builder()
        .level(Level::Info)
        .no_timestamps()
        .with_sink(Arc::clone(&sink))
        .build();

    log_io::info!(logger, "one", k = 1_u32);
    log_io::info!(logger, "two", k = 2_u32);

    for record in sink.snapshot() {
        println!("captured: {}", record.trim_end());
    }
}

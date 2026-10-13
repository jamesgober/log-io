//! One record, several destinations: human-readable on stderr for
//! people, JSON in a file for ingestion.
//!
//! Run with `cargo run --example 06_multi_sink`.

use log_io::format::JsonFormat;
use log_io::sink::FileSink;
use log_io::{Level, Logger};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join("log-io-multi.jsonl");
    let file_sink = FileSink::create(&path, JsonFormat::new())?;

    let logger = Logger::builder()
        .level(Level::Info)
        .stderr_human()
        .with_sink(file_sink)
        .build();

    log_io::info!(logger, "boot", region = "us-east-1");
    log_io::warn!(logger, "rate limited", remaining = 0_u32);
    logger.flush()?;

    println!("structured JSON also written to {}", path.display());
    Ok(())
}

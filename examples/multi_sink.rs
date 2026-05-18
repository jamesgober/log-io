//! Fan a record out to multiple sinks: human-readable on stderr for
//! developer eyes, JSON to a file for log ingest.

use log_io::format::JsonFormat;
use log_io::sink::FileSink;
use log_io::{Level, Logger};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join("log-io-multi.jsonl");
    let file_sink = FileSink::create(&path, JsonFormat::new())?;

    let logger = Logger::builder()
        .level(Level::Info)
        .stderr_human() // pretty for dev
        .with_sink(file_sink) // structured for ingest
        .build();

    log_io::info!(logger, "boot", region = "us-east-1");
    log_io::warn!(logger, "rate limited", remaining = 0_u32);
    logger.flush()?;

    println!("structured JSON also written to {}", path.display());
    Ok(())
}

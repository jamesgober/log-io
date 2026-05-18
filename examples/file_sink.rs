//! Writing records to a rotating-friendly file sink.

use log_io::format::JsonFormat;
use log_io::sink::FileSink;
use log_io::{Level, Logger};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join("log-io-example.jsonl");
    let sink = FileSink::append(&path, JsonFormat::new())?;

    let logger = Logger::builder()
        .level(Level::Info)
        .add_sink(sink)
        .null()
        .json()
        .build();

    for i in 0..5 {
        log_io::info!(logger, "tick", i = i as u32);
    }
    logger.flush()?;

    println!("wrote logs to {}", path.display());
    Ok(())
}

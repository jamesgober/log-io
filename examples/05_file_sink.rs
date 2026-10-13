//! Writing to a file without losing buffered records.
//!
//! `FileSink` buffers writes. Records at `Error` and above are flushed
//! before the call returns; everything else is flushed when the guard
//! returned by `init` is dropped at the end of `main`.
//!
//! Run with `cargo run --example 05_file_sink`.

use log_io::format::JsonFormat;
use log_io::sink::FileSink;
use log_io::{Level, Logger};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join("log-io-example.jsonl");
    let sink = FileSink::builder(&path, JsonFormat::new())
        .append(false)
        .sync_on_flush(true) // fsync on every flush, for crash safety
        .open()?;

    let logger = Logger::builder().level(Level::Info).with_sink(sink).build();
    let _guard = log_io::init(logger)?;

    for i in 0..5_u32 {
        log_io::info!("tick", i = i);
    }
    log_io::error!("flushed (and synced) before this call returns");

    println!("wrote logs to {}", path.display());
    Ok(())
}

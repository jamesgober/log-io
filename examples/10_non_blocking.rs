//! Move file I/O off the logging threads with a background writer, and
//! rotate the file by size.
//!
//! Run with `cargo run --example 10_non_blocking`.

use log_io::format::JsonFormat;
use log_io::sink::{NonBlocking, Overflow, RotatingFile};
use log_io::{Level, Logger};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join("log-io-non-blocking");
    std::fs::create_dir_all(&dir)?;

    let file = RotatingFile::builder(dir.join("app.jsonl"))
        .max_bytes(64 * 1024)
        .max_files(3)
        .open()?;
    let sink = NonBlocking::builder(file, JsonFormat::new())
        .capacity(4096)
        // Block never drops a record; DropNewest never waits on the
        // writer (error records then only queue their flush).
        .overflow(Overflow::Block)
        .build()?;

    let logger = Logger::builder().level(Level::Info).with_sink(sink).build();
    let _guard = logger.flush_guard(); // drains the queue at the end of main

    std::thread::scope(|s| {
        for worker in 0..4_u32 {
            let logger = &logger;
            s.spawn(move || {
                for i in 0..2_000_u32 {
                    log_io::info!(logger, "processed item", worker = worker, item = i);
                }
            });
        }
    });

    println!("logs (with rotated siblings) in {}", dir.display());
    Ok(())
}

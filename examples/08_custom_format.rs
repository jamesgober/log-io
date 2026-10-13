//! A custom format, two ways: implementing `Format`, or wrapping a
//! closure with `format::from_fn`. Formats are object-safe, so one can be
//! chosen at run time as a `Box<dyn Format>`.
//!
//! Run with `cargo run --example 08_custom_format`.

use core::fmt;

use log_io::format::{self, Format, JsonFormat};
use log_io::{Level, Logger, Record};

/// `level|target|message|k=v|k=v`. Keys and values are written raw:
/// a real format must escape them (see the built-in formats).
struct PipeFormat;

impl Format for PipeFormat {
    fn write_record(&self, record: &Record<'_>, w: &mut dyn fmt::Write) -> fmt::Result {
        write!(
            w,
            "{}|{}|{}",
            record.metadata.level, record.metadata.target, record.message
        )?;
        for field in record.all_fields() {
            write!(w, "|{}={}", field.key, field.value)?;
        }
        w.write_char('\n')
    }
}

fn main() {
    let closure = format::from_fn(|r: &Record<'_>, w: &mut dyn fmt::Write| {
        writeln!(w, "[{}] {}", r.metadata.level.as_str_upper(), r.message)
    });

    // Pick a format at run time.
    let chosen: Box<dyn Format> = match std::env::args().nth(1).as_deref() {
        Some("json") => Box::new(JsonFormat::new()),
        Some("closure") => Box::new(closure),
        _ => Box::new(PipeFormat),
    };

    let logger = Logger::builder()
        .level(Level::Info)
        .no_timestamps()
        .stdout(chosen)
        .build();

    log_io::info!(logger, "deployed", region = "eu-west-1", version = "2.0.0");
    log_io::warn!(logger, "queue depth", n = 200_u32);
}

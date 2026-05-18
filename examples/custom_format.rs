//! Implement a custom [`log_io::format::Format`].
//!
//! This produces CSV-ish output: `level,target,message,k=v,k=v`. Real
//! CSV would quote fields containing commas; this example keeps the
//! formatter short on purpose.

use core::fmt;

use log_io::format::Format;
use log_io::{Level, Logger, Record};

struct CsvFormat;

impl Format for CsvFormat {
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        write!(
            writer,
            "{},{},{}",
            record.metadata.level.as_str(),
            record.metadata.target,
            record.message
        )?;
        for field in record.all_fields() {
            write!(writer, ",{}={}", field.key, field.value)?;
        }
        writer.write_char('\n')
    }
}

fn main() {
    let logger = Logger::builder()
        .level(Level::Info)
        .no_timestamps()
        .no_context()
        .stdout(CsvFormat)
        .build();

    log_io::info!(logger, "deployed", region = "eu-west-1", version = "1.2.3");
    log_io::warn!(logger, "queue depth", n = 200_u32);
}

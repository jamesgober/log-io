//! Minimal usage: build a logger, log a few records.

use log_io::{Field, Level, Logger, Value};

fn main() {
    let logger = Logger::builder().level(Level::Info).stdout_human().build();

    logger.log(
        Level::Info,
        "server started",
        &[Field::new("port", 8080_u32)],
    );
    logger.log(
        Level::Warn,
        "slow request",
        &[Field::new("path", "/api/users"), Field::new("ms", 412_u64)],
    );
    logger.log(
        Level::Error,
        "request failed",
        &[
            Field::new("path", "/api/users"),
            Field::new("status", 500_u16),
            Field::new("error", Value::Str("database timeout")),
        ],
    );
}

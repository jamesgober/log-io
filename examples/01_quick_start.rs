//! The smallest useful setup: one logger, human-readable output on
//! stderr, a few records.
//!
//! Run with `cargo run --example 01_quick_start`.

use log_io::{Field, Level, Logger, Value};

fn main() {
    // `stderr_human` colours the level column when stderr is a terminal
    // (and honours NO_COLOR).
    let logger = Logger::builder().level(Level::Info).stderr_human().build();

    logger.log(
        Level::Info,
        "server started",
        &[Field::new("port", 8080_u16)],
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
    logger.log(Level::Debug, "filtered out by the Info threshold", &[]);
}

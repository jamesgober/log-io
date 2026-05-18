//! Attach service-level fields once at builder time and have them
//! appear on every record without the call sites repeating them.

use log_io::{Level, Logger};

fn main() {
    let logger = Logger::builder()
        .level(Level::Info)
        .with_default_field("service", "billing")
        .with_default_field("version", env!("CARGO_PKG_VERSION"))
        .with_default_field("env", "production")
        .stdout_json()
        .build();

    log_io::info!(logger, "boot");
    log_io::info!(logger, "ready", port = 8443_u32);
    log_io::warn!(logger, "slow query", query_ms = 1280_u64);
}

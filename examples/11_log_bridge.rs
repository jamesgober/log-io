//! Route records from the `log` facade (used by most of the ecosystem)
//! through a log-io pipeline, structured key-values included.
//!
//! Run with `cargo run --example 11_log_bridge --features log`.

use log_io::compat::LogBridge;
use log_io::{Level, Logger};

fn main() -> Result<(), log::SetLoggerError> {
    let logger = Logger::builder()
        .level(Level::Debug)
        .with_default_field("service", "api")
        .stdout_logfmt()
        .build();
    let _guard = logger.flush_guard();
    LogBridge::new(logger).install()?;

    log::info!(user = "alice", attempts = 3; "login succeeded");
    log::warn!(target: "db", "slow query: {} ms", 412);
    log::trace!("filtered out by the Debug threshold");
    Ok(())
}

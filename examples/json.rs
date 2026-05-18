//! JSON output to stdout, with macros.

use log_io::{Level, Logger};

fn main() {
    let logger = Logger::builder().level(Level::Info).stdout_json().build();

    log_io::info!(
        logger,
        "service ready",
        region = "us-east-1",
        port = 8080_u32
    );
    log_io::warn!(logger, "retry", attempt = 2_u32, max = 5_u32);
    log_io::error!(
        logger,
        "shutdown",
        signal = "SIGTERM",
        graceful = true,
        elapsed_ms = 1245_u64,
    );
}

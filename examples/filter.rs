//! Per-target filtering via a directive string.
//!
//! The directive form mirrors `RUST_LOG`: a default level plus
//! `target=level` overrides, separated by commas.

use log_io::{Level, Logger};

fn main() {
    // Default warn, but enable debug for `app::auth`, silence `hyper`.
    let logger = Logger::builder()
        .filter_directive("warn, app::auth=debug, hyper=off")
        .stdout()
        .human()
        .build();

    logger
        .try_log_with_target(Level::Info, "app", "info-app (filtered out)", &[])
        .unwrap();
    logger
        .try_log_with_target(Level::Warn, "app", "warn-app", &[])
        .unwrap();
    logger
        .try_log_with_target(Level::Debug, "app::auth", "auth debug visible", &[])
        .unwrap();
    logger
        .try_log_with_target(Level::Error, "hyper", "hyper (filtered out)", &[])
        .unwrap();
}

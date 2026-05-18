//! Per-target filtering via a directive string.
//!
//! The directive form mirrors `RUST_LOG`: a default level plus
//! `target=level` overrides, separated by commas.

use log_io::{Level, Logger};

fn main() {
    let logger = Logger::builder()
        .filter_directive("warn, app::auth=debug, hyper=off")
        .stdout_human()
        .build();

    logger
        .try_log(Level::Info, "app", "info-app (filtered out)", &[])
        .unwrap();
    logger.try_log(Level::Warn, "app", "warn-app", &[]).unwrap();
    logger
        .try_log(Level::Debug, "app::auth", "auth debug visible", &[])
        .unwrap();
    logger
        .try_log(Level::Error, "hyper", "hyper (filtered out)", &[])
        .unwrap();
}

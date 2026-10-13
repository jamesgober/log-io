//! Per-target filtering with `RUST_LOG`-style directives. The most
//! specific rule wins, regardless of order. The directive can also come
//! from an environment variable.
//!
//! Run with `cargo run --example 04_filter`, or try
//! `LOG_IO="debug,hyper=off" cargo run --example 04_filter`.

use log_io::Logger;

fn main() -> Result<(), log_io::BuildError> {
    let logger = Logger::builder()
        .filter_directive("warn, app=info, app::auth=debug, hyper=off")
        .env_filter("LOG_IO") // overrides the directive above when set
        .stdout_human()
        .try_build()?;

    log_io::info!(logger, target: "app", "visible: app is at info");
    log_io::debug!(logger, target: "app", "hidden: app is at info");
    log_io::debug!(logger, target: "app::auth::session", "visible: most specific rule wins");
    log_io::error!(logger, target: "hyper::client", "hidden: hyper is off");
    log_io::warn!(logger, target: "other", "visible: default is warn");
    Ok(())
}

//! The logging macros: `format!`-style messages, typed fields, dotted
//! and quoted keys, and lazy `Display` (`%`) / `Debug` (`?`) values.
//!
//! Run with `cargo run --example 02_macros`.

use log_io::{Level, Logger, Value};

fn main() {
    let logger = Logger::builder().level(Level::Debug).stdout_json().build();

    let user = "alice";
    let addr = std::net::Ipv4Addr::new(10, 0, 0, 7);
    let roles = ["admin", "ops"];

    // Inline captures and positional arguments, then fields.
    log_io::info!(
        logger,
        "user {user} logged in from {}",
        addr,
        session = 42_u64
    );

    // Dotted and string-literal keys; `%` and `?` render lazily.
    log_io::debug!(
        logger,
        "request",
        http.method = "GET",
        http.status = 200_u16,
        "content-type" = "application/json",
        peer = %addr,
        roles = ?roles,
    );

    // An explicit target, used by per-target filters.
    log_io::warn!(logger, target: "billing::invoices", "retrying", attempt = 2_u8);

    // Errors keep their source chain.
    let err = std::io::Error::other("connection reset by peer");
    log_io::error!(logger, "upstream failed", error = Value::Error(&err));

    // Nothing here is evaluated: the record is filtered out first.
    log_io::trace!(logger, "expensive {}", expensive());
}

/// Only called when trace records are enabled.
fn expensive() -> String {
    String::from("an expensive summary")
}

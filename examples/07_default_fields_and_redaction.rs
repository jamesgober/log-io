//! Service-level default fields attached to every record, and redaction
//! of secret-looking keys at the logging boundary.
//!
//! Run with `cargo run --example 07_default_fields_and_redaction`.

use log_io::{Level, Logger};

fn main() {
    let logger = Logger::builder()
        .level(Level::Info)
        .with_default_field("service", "billing")
        .with_default_field("version", env!("CARGO_PKG_VERSION"))
        .redact_common_secrets() // password, token, api_key, authorization, ...
        .redact_key("card_number")
        .stdout_json()
        .build();

    log_io::info!(logger, "boot");
    log_io::info!(
        logger,
        "login",
        user = "alice",
        password = "hunter2",        // written as "[REDACTED]"
        db.password = "also hidden", // last dotted segment matches too
    );
    log_io::warn!(logger, "payment declined", card_number = "4111111111111111");
}

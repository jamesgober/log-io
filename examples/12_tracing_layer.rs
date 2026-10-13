//! Use log-io as the output stage of a `tracing` subscriber. Span fields
//! are attached to every event inside the span.
//!
//! Run with `cargo run --example 12_tracing_layer --features tracing`.

use log_io::compat::TracingLayer;
use log_io::{Level, Logger};
use tracing_subscriber::layer::SubscriberExt;

fn main() -> Result<(), tracing::subscriber::SetGlobalDefaultError> {
    let logger = Logger::builder().level(Level::Info).stdout_json().build();
    let _guard = logger.flush_guard();
    let subscriber = tracing_subscriber::registry().with(TracingLayer::new(logger));
    tracing::subscriber::set_global_default(subscriber)?;

    let span = tracing::info_span!("request", id = 7_u64, user = "alice");
    let _entered = span.enter();
    tracing::info!(
        status = 200_u16,
        elapsed_ms = 12.5,
        "handled {}",
        "/api/items"
    );
    tracing::warn!(retries = 2_u8, "slow upstream");
    Ok(())
}

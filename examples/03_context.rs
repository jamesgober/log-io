//! Thread-local context: attach `trace_id` / `request_id` once and every
//! record below carries them. Snapshots carry context to other threads
//! (or async tasks, via `ContextSnapshot::wrap`).
//!
//! Run with `cargo run --example 03_context`.

use log_io::{Level, Logger, context};

fn main() {
    let logger = Logger::builder().level(Level::Info).stdout_logfmt().build();

    handle_request(&logger, "tx-7f3a", "req-001");
    handle_request(&logger, "tx-9c4b", "req-002");
}

fn handle_request(logger: &Logger, trace_id: &str, request_id: &str) {
    let _trace = context::with_trace_id(trace_id);
    let _request = context::with_request_id(request_id);

    log_io::info!(logger, "request received", path = "/api/items");

    // Hand the work to another thread without losing the context.
    let snapshot = context::capture();
    let worker_logger = logger.clone();
    std::thread::spawn(move || {
        snapshot.scope(|| {
            log_io::info!(worker_logger, "fetched items", count = 42_u32);
        });
    })
    .join()
    .unwrap_or_else(|_| eprintln!("worker panicked"));

    log_io::info!(logger, "request completed", status = 200_u16);
}

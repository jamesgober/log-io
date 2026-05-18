//! Demonstrates thread-local context propagation.
//!
//! All records emitted within the request handler scope automatically
//! carry the trace and request IDs without the call sites needing to
//! pass them explicitly.

use log_io::{context, Field, Level, Logger};

fn main() {
    let logger = Logger::builder().level(Level::Info).stdout_logfmt().build();

    handle_request(&logger, "tx-7f3a", "req-001");
    handle_request(&logger, "tx-9c4b", "req-002");
}

fn handle_request(logger: &Logger, trace_id: &str, request_id: &str) {
    let _t = context::with_trace_id(trace_id);
    let _r = context::with_request_id(request_id);

    log_io::info!(logger, "request received", path = "/api/items");
    do_work(logger);
    log_io::info!(logger, "request completed", status = 200_u16);
}

fn do_work(logger: &Logger) {
    logger.log(Level::Info, "fetched items", &[Field::new("count", 42_u32)]);
}

//! Zero-allocation regression guard for the steady-state hot path.
//!
//! A counting `#[global_allocator]` wraps the system allocator and
//! counts allocations made by the *current thread* only, so tests
//! running in parallel in this binary cannot pollute each other's
//! numbers. Each case warms up (thread-local buffers grow to their
//! working size), then asserts that a further batch of calls performs
//! no allocation at all.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::io::{self, Write};

use log_io::format::{HumanFormat, JsonFormat, LogfmtFormat};
use log_io::{Field, Level, Logger, context};

struct CountingAlloc;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

fn bump() {
    // `try_with` because the allocator can run during thread-local
    // teardown, after `ALLOCS` itself is gone.
    let _ignored = ALLOCS.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: every method forwards to `System` with the caller's layout
// and pointer unchanged, so all `GlobalAlloc` contracts are inherited
// from the system allocator. The only extra work is a const-initialized
// thread-local `Cell` update, which never allocates.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: forwarded verbatim; the caller upholds `alloc`'s contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        // SAFETY: forwarded verbatim; the caller upholds the contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was produced by `System` via one of the methods above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: `ptr` was produced by `System`; the caller upholds the contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn allocations_during(mut f: impl FnMut()) -> u64 {
    for _ in 0..64 {
        f();
    }
    let before = ALLOCS.with(Cell::get);
    for _ in 0..1_000 {
        f();
    }
    ALLOCS.with(Cell::get) - before
}

struct DevNull;

impl Write for DevNull {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn fields() -> [Field<'static>; 3] {
    [
        Field::new("port", 8080_u32),
        Field::new("host", "0.0.0.0"),
        Field::new("ratio", 1.5_f64),
    ]
}

#[test]
fn counter_detects_allocations() {
    // Guards the guard: if the counting allocator were not installed
    // or not counting, every other test here would pass vacuously.
    let n = allocations_during(|| {
        let v: Vec<u8> = Vec::with_capacity(black_box(16));
        black_box(v);
    });
    assert_eq!(n, 1_000);
}

#[test]
fn default_config_json_is_allocation_free() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .writer(DevNull, JsonFormat::new())
        .build();
    let fields = fields();
    let n = allocations_during(|| logger.log(Level::Info, "hello", &fields));
    assert_eq!(n, 0, "default JSON pipeline allocated {n} times");
}

#[test]
fn logfmt_and_human_are_allocation_free() {
    let fields = fields();
    let logfmt = Logger::builder()
        .level(Level::Trace)
        .writer(DevNull, LogfmtFormat::new())
        .build();
    let n = allocations_during(|| logfmt.log(Level::Info, "hello world", &fields));
    assert_eq!(n, 0, "logfmt pipeline allocated {n} times");

    let human = Logger::builder()
        .level(Level::Trace)
        .writer(DevNull, HumanFormat::new())
        .build();
    let n = allocations_during(|| human.log(Level::Info, "hello world", &fields));
    assert_eq!(n, 0, "human pipeline allocated {n} times");
}

#[test]
fn context_and_default_fields_are_allocation_free() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .with_default_field("service", "api")
        .with_default_field("version", "1.2.3")
        .writer(DevNull, JsonFormat::new().timestamp_rfc3339(true))
        .build();
    let _t = context::with_trace_id("tx-1");
    let _r = context::with_request_id("rq-1");
    let fields = fields();
    let n = allocations_during(|| logger.log(Level::Info, "hello", &fields));
    assert_eq!(n, 0, "context + default fields allocated {n} times");
}

#[test]
fn macros_are_allocation_free() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .writer(DevNull, JsonFormat::new())
        .build();
    let n = allocations_during(|| {
        log_io::info!(logger, "hello", port = 8080_u32, host = "x");
    });
    assert_eq!(n, 0, "info! allocated {n} times");
}

#[test]
fn filtered_out_records_are_allocation_free() {
    let logger = Logger::builder()
        .level(Level::Error)
        .writer(DevNull, JsonFormat::new())
        .build();
    let fields = fields();
    let n = allocations_during(|| logger.log(Level::Info, "hello", &fields));
    assert_eq!(n, 0, "filtered-out record allocated {n} times");
}

#[test]
fn format_args_messages_are_allocation_free() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .writer(DevNull, JsonFormat::new())
        .build();
    let user = "alice";
    let mut n = 0_u64;
    let allocs = allocations_during(|| {
        n += 1;
        log_io::info!(
            logger,
            "user {user} request {n} took {} ms",
            12.5,
            http.status = 200_u16
        );
    });
    assert_eq!(allocs, 0, "format_args message allocated {allocs} times");
}

#[test]
fn key_collisions_and_redaction_are_allocation_free() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .with_default_field("service", "api")
        .with_default_field("tenant", "default")
        .redact_common_secrets()
        .writer(DevNull, LogfmtFormat::new())
        .build();
    let _t = context::with_field("tenant", "ctx");
    let fields = [
        Field::new("tenant", "call"),
        Field::new("password", "hunter2"),
        Field::new("user", "alice"),
    ];
    let n = allocations_during(|| logger.log(Level::Info, "login", &fields));
    assert_eq!(n, 0, "dedup + redaction allocated {n} times");
}

#[test]
fn human_format_with_timestamps_is_allocation_free() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .writer(
            DevNull,
            HumanFormat::new().color(true).show_source_location(true),
        )
        .build();
    let n = allocations_during(|| {
        log_io::warn!(logger, "slow", ms = 412_u64, path = "/api users");
    });
    assert_eq!(n, 0, "human format allocated {n} times");
}

#[cfg(feature = "log")]
#[test]
fn log_bridge_is_allocation_free() {
    use log::Log;
    let logger = Logger::builder()
        .level(Level::Trace)
        .writer(DevNull, JsonFormat::new())
        .build();
    let bridge = log_io::compat::LogBridge::new(logger);
    let kvs: &[(&str, log::kv::Value<'_>)] = &[
        ("user", log::kv::Value::from("alice")),
        ("n", log::kv::Value::from(3_u64)),
    ];
    let mut i = 0_u32;
    let n = allocations_during(|| {
        i += 1;
        bridge.log(
            &log::Record::builder()
                .level(log::Level::Info)
                .target("app")
                .key_values(&kvs)
                .args(format_args!("request {i}"))
                .build(),
        );
    });
    assert_eq!(n, 0, "log bridge allocated {n} times");
}

#[cfg(feature = "tracing")]
#[test]
fn tracing_layer_events_are_allocation_free() {
    use tracing_subscriber::layer::SubscriberExt;
    let logger = Logger::builder()
        .level(Level::Trace)
        .writer(DevNull, JsonFormat::new())
        .build();
    let subscriber = tracing_subscriber::registry().with(log_io::compat::TracingLayer::new(logger));
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("request", id = 7_u64, user = "alice");
        let _entered = span.enter();
        let n = allocations_during(|| {
            tracing::info!(status = 200_u16, path = "/api", "handled {}", 42);
        });
        assert_eq!(n, 0, "tracing event allocated {n} times");
    });
}

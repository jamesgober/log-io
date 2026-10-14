<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>log-io</b>
    <br>
    <sub>
        <sup>STRUCTURED LOGGING IO PIPELINE FOR RUST</sup>
    </sub>
</h1>

<p align="center">
    <a href="https://crates.io/crates/log-io"><img src="https://img.shields.io/crates/v/log-io.svg" alt="Crates.io"></a>
    <a href="https://crates.io/crates/log-io" alt="Download"><img alt="Crates.io Downloads" src="https://img.shields.io/crates/d/log-io?color=%230099ff"></a>
    <a href="https://docs.rs/log-io"><img src="https://docs.rs/log-io/badge.svg" alt="Documentation"></a>
    <a href="https://github.com/jamesgober/log-io/actions"><img alt="GitHub CI" src="https://github.com/jamesgober/log-io/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md" title="MSRV"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue"></a>
    <a href="#license"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
</p>

<p align="center">
    <b>Filter, format and deliver structured log records.</b>
    <br>
    <i>No allocation in steady state, output that is safe for any input, and nothing lost at shutdown.</i>
</p>

<br>

<p>
    <strong>log-io</strong> is a structured logging pipeline: a record passes a per-target filter, picks up ambient fields (service defaults, request context), is serialized as JSON, logfmt or a human-readable line, and is delivered to one or more sinks such as standard output, a buffered file, a rotating file or a background writer. It works on its own through its macros and global logger, or as the output stage for code that already logs through the <code>log</code> facade or <code>tracing</code>.
</p>

<p>
    It focuses on the properties that are easy to get wrong in a logger. The steady-state hot path performs no heap allocation, which a counting-allocator test enforces. Every format escapes control characters, line separators and bidi overrides, so a hostile request path cannot drive your terminal or forge log lines, and a field named <code>level</code> cannot override the real level. Error records are flushed before the call returns, and a guard flushes everything else at the end of <code>main</code>, so the last lines before a crash are on disk.
</p>

<p>
    The default build has zero dependencies and no <code>unsafe</code> code. The data model and the three formatters also work under <code>no_std</code> without an allocator. The <code>log</code> and <code>tracing</code> bridges are opt-in features.
</p>

---

## Features

### **Pipeline**
- **Three formats** - JSON Lines, logfmt, and an aligned human-readable line with optional terminal colour.
- **Per-target filtering** - `RUST_LOG`-style directives where the most specific rule wins; read them from an environment variable with `env_filter`.
- **Ambient fields** - service-level default fields and thread-local context (`trace_id`, `request_id`), de-duplicated with clear precedence.
- **Several sinks per logger** - stdout/stderr, buffered files, size- and time-based rotation, any `io::Write`, and a background writer with a bounded queue.

### **Ergonomics**
- **Macros with `format!` messages** - `info!("user {id} logged in", http.status = 200, peer = %addr)`, with or without an explicit logger.
- **Global logger** - `log_io::init(logger)` returns a guard that flushes at the end of `main`.
- **Async-friendly context** - capture context once and re-install it around every poll of a future, on any executor.
- **`log` and `tracing` bridges** - route existing ecosystem logging into log-io sinks (features `log` and `tracing`).

### **Safety and reliability**
- **Injection-safe output** - C0/C1 controls, DEL, U+2028/2029 and bidi controls are escaped in every format; metadata keys cannot be spoofed by fields.
- **Redaction** - replace `password`, `token`, `authorization` and other configured keys with `[REDACTED]` at the logging boundary.
- **Durable shutdown** - flush-on-level (errors by default), flush guards, `fsync` on flush when you need it, and an opt-in panic hook.
- **Zero unsafe code** - `#![forbid(unsafe_code)]`; sinks recover from mutex poisoning; library code never unwraps.

---

## Installation

```toml
[dependencies]
log-io = "2"

# Optional integrations:
log-io = { version = "2", features = [
    "log",      # route `log` macros into log-io (LogBridge)
    "tracing",  # use log-io as a tracing-subscriber layer (TracingLayer)
] }
```

**Feature recommendations**
- **Applications:** the defaults (`std`, `json`, `logfmt`, `human`).
- **Libraries already using `log` or `tracing`:** add `log` or `tracing` and keep your existing macros.
- **Embedded (`no_std`):** `default-features = false, features = ["json"]` (or `logfmt` / `human`) for the data model and formatters only.

---

## Quick Start

```rust
use log_io::{Level, Logger};

let logger = Logger::builder()
    .level(Level::Info)
    .env_filter("LOG_IO") // e.g. LOG_IO="info,my_app::db=debug"
    .with_default_field("service", "billing")
    .stdout_json()
    .build();
let _guard = log_io::init(logger).expect("first and only global logger");

let port = 8080_u16;
log_io::info!("listening on port {port}", tls = true);
log_io::warn!(target: "my_app::db", "slow query", ms = 412_u64, table = "users");
```

### **Context across threads and async tasks**

```rust
use log_io::{context, Logger};

let logger = Logger::builder().stdout_logfmt().build();
let _request = context::with_request_id("req-7");
log_io::info!(logger, "accepted"); // carries request_id=req-7

let snapshot = context::capture();
let worker = logger.clone();
std::thread::spawn(move || {
    snapshot.scope(|| log_io::info!(worker, "processing")); // carries it too
})
.join()
.unwrap();
// For futures: tokio::spawn(snapshot.wrap(async move { ... }))
```

### **Durable file output off the hot path**

```rust,no_run
use log_io::format::JsonFormat;
use log_io::sink::{NonBlocking, RotatingFile, Rotation};
use log_io::{Level, Logger};

let file = RotatingFile::builder("logs/app.jsonl")
    .max_bytes(50 * 1024 * 1024)
    .rotation(Rotation::Daily)
    .max_files(14)
    .open()?;
let sink = NonBlocking::new(file, JsonFormat::new())?;
let logger = Logger::builder()
    .level(Level::Info)
    .redact_common_secrets()
    .with_sink(sink)
    .build();
let _guard = log_io::init(logger)?; // drains the queue when main returns
# Ok::<(), Box<dyn std::error::Error>>(())
```

### **Plugging into `log` and `tracing`**

```rust
use log_io::compat::{LogBridge, TracingLayer};
use log_io::Logger;
use tracing_subscriber::layer::SubscriberExt;

let logger = Logger::builder().stdout_logfmt().build();
LogBridge::new(logger.clone()).install().ok();
log::info!(user = "alice"; "from the log facade");

let subscriber = tracing_subscriber::registry().with(TracingLayer::new(logger));
tracing::subscriber::with_default(subscriber, || {
    let _span = tracing::info_span!("request", id = 7_u64).entered();
    tracing::info!(status = 200_u16, "from tracing"); // carries id=7
});
```

---

## Performance

Measured on an AMD Ryzen 9 9950X3D, Windows 11, rustc 1.95; see
[`BENCH.md`](BENCH.md) for the method and the full tables.

| Workload | 2.0.0 | vs 1.0.0 |
|---|---:|---:|
| JSON format, 5 fields | ~123 ns | 8% faster |
| logfmt format, 5 fields | ~115 ns | 26% faster |
| Pipeline, JSON writer, 3 fields | ~64 ns | same |
| Pipeline, default config (timestamp + context) | ~114 ns | same |
| Record rejected by the filter | ~1 ns | 60% faster |
| 4 threads, one shared JSON sink | ~24 ns/record | 25% faster |
| 8 threads, one shared JSON sink | ~27 ns/record | 2.7x faster |

No heap allocation per record in steady state (enforced by
`tests/alloc.rs`).

---

## Status: Stable

**Current version:** `2.0.0`. The API follows Semantic Versioning;
`Value`, `Error`, `Metadata`, `Record` and the configuration enums are
`#[non_exhaustive]` so they can grow in minor releases. Upgrading from
1.x: see [`docs/MIGRATION.md`](docs/MIGRATION.md).

**Planned:** sampling and rate limiting per call site, OpenTelemetry-shaped
JSON layouts, compression of rotated files.

---

## Documentation & Resources

### **Documentation**
- **[Documentation Index](docs/README.md)** - where everything is
- **[API Reference](docs/API.md)** - every component with runnable examples
- **[Migration Guide](docs/MIGRATION.md)** - upgrading from 1.x
- **[Benchmarks](BENCH.md)** - method, machine and numbers
- **[Specification](REPS.md)** - design contract and policies
- **[docs.rs/log-io](https://docs.rs/log-io)** - generated rustdoc

### **Examples**

| Example | Shows |
|---|---|
| [`01_quick_start`](examples/01_quick_start.rs) | the smallest useful setup |
| [`02_macros`](examples/02_macros.rs) | format strings, dotted keys, `%` / `?` values, targets |
| [`03_context`](examples/03_context.rs) | request context and snapshots across threads |
| [`04_filter`](examples/04_filter.rs) | directives, most-specific matching, `LOG_IO` env var |
| [`05_file_sink`](examples/05_file_sink.rs) | durable file output with a flush guard |
| [`06_multi_sink`](examples/06_multi_sink.rs) | human on stderr and JSON in a file |
| [`07_default_fields_and_redaction`](examples/07_default_fields_and_redaction.rs) | service fields and secret redaction |
| [`08_custom_format`](examples/08_custom_format.rs) | custom and runtime-selected formats |
| [`09_custom_sink`](examples/09_custom_sink.rs) | implementing `Sink` |
| [`10_non_blocking`](examples/10_non_blocking.rs) | background writer with rotation |
| [`11_log_bridge`](examples/11_log_bridge.rs) | the `log` facade (feature `log`) |
| [`12_tracing_layer`](examples/12_tracing_layer.rs) | `tracing` with span fields (feature `tracing`) |

Run one with `cargo run --example 02_macros` (add `--features log` or
`--features tracing` for the last two).

---

### **Troubleshooting**

<details>
  <summary><b>My last log lines are missing after the program exits.</b></summary>
    <p>A logger stored in a <code>static</code> is never dropped. Keep the guard from <code>log_io::init</code> alive until the end of <code>main</code>, and call <code>log_io::global::shutdown()</code> before <code>std::process::exit</code>. Records at <code>Level::Error</code> and above are flushed immediately (a <code>NonBlocking</code> sink with <code>Overflow::DropNewest</code> only queues that flush, so it never waits).</p>
</details>

<details>
  <summary><b>A per-target rule is ignored for records from <code>Logger::log</code>.</b></summary>
    <p><code>Logger::log</code> has no target argument and uses the builder's <code>default_target</code> (empty by default). Use the macros (target = module path), <code>try_log</code>, or set <code>default_target</code>.</p>
</details>

<details>
  <summary><b>Context set in an async handler is missing or appears in other requests.</b></summary>
    <p>Context is per thread, and a guard cannot be held across <code>.await</code> in a multi-threaded runtime (it is <code>!Send</code>). Capture a snapshot with <code>context::capture()</code> and wrap the future with <code>snapshot.wrap(..)</code>. On executors that do not require <code>Send</code> (<code>join!</code>, <code>LocalSet</code>) the compiler cannot help: wrap every future that holds a guard across <code>.await</code>, or its fields show up in sibling futures.</p>
</details>

<details>
  <summary><b>Colours show up as garbage in a Windows console.</b></summary>
    <p>The classic console needs virtual-terminal mode switched on, which this crate cannot do without <code>unsafe</code>. Automatic colour is only enabled in terminals known to handle ANSI (Windows Terminal, ConEmu, Git Bash); install <code>HumanFormat::new()</code> explicitly to disable colour, or use <code>ColorChoice::Always</code> if your program enables VT mode.</p>
</details>

---

### **Version Compatibility**

- **Rust:** 1.85+ (edition 2024). Raising the MSRV is a major-version change.
- **API stability:** semantic versioning; see `REPS.md` section 9.
- **Feature flags:** additive; `log` and `tracing` are independent of each other.

### **Development Setup**

```bash
git clone https://github.com/jamesgober/log-io.git
cd log-io
cargo test --all-features    # unit, integration, property, doc tests
cargo bench --all-features   # criterion benchmarks
cargo clippy --all-targets --all-features -- -D warnings
cargo +nightly fuzz run json_format   # see fuzz/README.md
```

<hr>

<!-- LICENSE
############################################# -->
<div id="license">
    <h2>License</h2>
    <p><b>log-io</b> is licensed under the <b>Apache License, Version 2.0</b>. See <a href="./LICENSE" title="Apache-2.0 License">LICENSE</a> or <a href="http://www.apache.org/licenses/LICENSE-2.0" title="Apache-2.0 License" target="_blank">apache.org/licenses/LICENSE-2.0</a>.</p>
    <p>Unless required by applicable law or agreed to in writing, software distributed under the License is distributed on an <b>"AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND</b>, either express or implied.</p>
</div>

<!-- FOOT COPYRIGHT
################################################# -->
<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>JAMES GOBER.</strong></sup>
</div>

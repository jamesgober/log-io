<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <strong>log-io</strong>
    <br>
    <sup><sub>STRUCTURED LOGGING IO PIPELINE FOR RUST</sub></sup>
</h1>

<p align="center">
    <a href="https://crates.io/crates/log-io"><img alt="crates.io" src="https://img.shields.io/crates/v/log-io.svg"></a>
    <a href="https://crates.io/crates/log-io"><img alt="downloads" src="https://img.shields.io/crates/d/log-io.svg"></a>
    <a href="https://docs.rs/log-io"><img alt="docs.rs" src="https://docs.rs/log-io/badge.svg"></a>
    <a href="https://github.com/jamesgober/log-io/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/jamesgober/log-io/actions/workflows/ci.yml/badge.svg"></a>
</p>

<p align="center">
    Structured log records flow through one pipeline: zero-allocation fast path, multiple output formats, context propagation.
</p>

---

## Overview

`log-io` is a structured logging pipeline for Rust. It is not a wrapper
around `log` or `tracing`. A record flows through three composable
stages: filter, format, sink. The fast path is allocation-free; the
data model is `no_std`-compatible; the crate has zero runtime
dependencies.

## Install

```toml
[dependencies]
log-io = "1"
```

## Quick start

```rust
use log_io::{Level, Logger};

let logger = Logger::builder()
    .level(Level::Info)
    .stdout_json()
    .build();

log_io::info!(logger, "server started", port = 8080_u32);
log_io::warn!(logger, "slow request", path = "/api/users", ms = 412_u64);
```

## Features

- **Three output formats**: JSON (line-delimited), logfmt
  (`key=value`), human-readable (timestamp + aligned level column).
- **Per-target filtering**: `info, app::auth=debug, hyper=off` style
  directives, prefix-matched on `::` and `.` boundaries.
- **Thread-local context**: stash `trace_id` / `request_id` once at
  the request boundary; every downstream record carries them.
- **Per-logger default fields**: attach service / version / region
  once at builder time; every record carries them.
- **Multi-sink fan-out**: human format on stderr for developer eyes
  and JSON to a file for ingest, served from one logger.
- **Source location capture**: macros attach `file!()`, `line!()`,
  and `module_path!()` to every record.
- **Zero runtime dependencies**: depends only on `core` and `std`.
- **`no_std`-compatible data model**: the `Record` / `Format` layer
  compiles without `std` and can be wired to embedded
  `core::fmt::Write` destinations.
- **`#![forbid(unsafe_code)]`** crate-wide.

## Performance

| Workload                            | ns / record |
|-------------------------------------|------------:|
| JSON, 5 fields, formatter only      | ~131        |
| logfmt, 5 fields, formatter only    | ~159        |
| human, 5 fields, formatter only     | ~210        |
| Pipeline, 3 fields, JSON + writer   | ~59         |
| Pipeline, filtered out below thresh | ~1          |

Single-thread throughput is ~15 M records/sec to a discarding writer;
scales to ~28 M at four threads. See [`BENCH.md`](BENCH.md) for the
full methodology.

## Documentation

- [`docs/API.md`](docs/API.md) - complete API reference with examples.
- [`REPS.md`](REPS.md) - formal project specification.
- [`BENCH.md`](BENCH.md) - benchmark methodology and numbers.
- [docs.rs/log-io](https://docs.rs/log-io) - generated rustdoc.

## Examples

The [`examples/`](examples) directory contains runnable demos:

| Example          | Topic                                                 |
|------------------|-------------------------------------------------------|
| `basic`          | Minimal usage with the human format.                  |
| `json`           | JSON output to stdout using macros.                   |
| `context`        | Thread-local trace / request ID propagation.          |
| `filter`         | Directive-style per-target filtering.                 |
| `file_sink`      | Writing records to a file.                            |
| `multi_sink`     | Fanning out one record to several destinations.       |
| `custom_format`  | Implementing the `Format` trait yourself.             |
| `custom_sink`    | Implementing the `Sink` trait yourself.               |
| `default_fields` | Service-level fields attached once at builder time.   |

Run any of them with `cargo run --example <name>`.

## License

Licensed under the Apache License, Version 2.0. See [`LICENSE`](LICENSE)
for the full text.


<!--
:: COPYRIGHT
=============================================== -->
<div align="center">
  <br>
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2025 <strong>JAMES GOBER.</strong></sup>
</div>

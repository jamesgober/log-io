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

## Status

`0.9.x` is the pre-1.0 stabilization line. The public API is feature-
complete and tested; `1.0.0` will follow a soak period.

## What it does

Structured logging pipeline for Rust. Zero-allocation fast path, JSON / logfmt / human-readable outputs, context propagation (request-id, trace-id), per-module filtering, async-safe sinks. An IO pipeline for log records, not a wrapper around log+tracing.

## Quick start

```rust
use log_io::{Field, Level, Logger};

let logger = Logger::builder()
    .level(Level::Info)
    .stdout()
    .json()
    .build();

log_io::info!(logger, "server started", port = 8080_u32);
log_io::warn_!(logger, "slow request", path = "/api/users", ms = 412_u64);
```

## Features

- **Three output formats**: JSON (line-delimited), logfmt
  (`key=value`), human-readable (timestamp + aligned level column).
- **Per-target filtering**: `info, app::auth=debug, hyper=off` style
  directives, prefix-matched on `::` and `.` boundaries.
- **Thread-local context**: stash `trace_id` / `request_id` once at
  the request boundary; every downstream record carries them.
- **Zero runtime dependencies**: depends only on `core` and `std`.
- **`no_std`-compatible data model**: the `Record` / `Format` layer
  compiles without `std` and can be wired to embedded `core::fmt::Write`
  destinations.
- **`#![forbid(unsafe_code)]`** crate-wide.

## Status

See [`.dev/ROADMAP.md`](.dev/ROADMAP.md) for the path to 1.0.
[`docs/API.md`](docs/API.md) is the prose API reference;
[`REPS.md`](REPS.md) is the formal specification.

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
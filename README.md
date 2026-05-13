<h1 align="center">
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

Status: ACTIVE - planning + initial scaffold. Public API not stable. See `.dev/ROADMAP.md` for the path to 1.0.

This repository is published primarily to reserve the crate name and
to establish the project scaffolding. Implementation work proceeds on
the schedule documented in `.dev/ROADMAP.md`.

## What it does

Structured logging pipeline for Rust. Zero-allocation fast path, JSON / logfmt / human-readable outputs, context propagation (request-id, trace-id), per-module filtering, async-safe sinks. An IO pipeline for log records, not a wrapper around log+tracing.

## License

Licensed under the Apache License, Version 2.0. See [`LICENSE`](LICENSE)
for the full text.

Copyright (C) 2026 James Gober.

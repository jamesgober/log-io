# Changelog

All notable changes to this project are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-05-18

First stable release. The public API is frozen; subsequent 1.x
releases will preserve backwards compatibility.

### Public API

The pipeline is built around five small layers:

- **Data model** (`no_std`): `Level`, `Value`, `Field`, `Metadata`,
  `Record`. All fields are borrowed; the fast path performs no
  allocation.
- **Filter**: `Filter`, `FilterRule`, `ParseFilterError`. Per-target
  severity overrides with directive parsing (`info, app::auth=debug,
  hyper=off` form).
- **Format** (`no_std`): the `Format` trait plus three built-in
  implementations behind feature flags: `JsonFormat`,
  `LogfmtFormat`, `HumanFormat`.
- **Sink**: the `Sink` trait plus five built-ins: `StdoutSink`,
  `StderrSink`, `FileSink`, `WriterSink<W, F>`, `NullSink`.
- **Logger**: `Logger`, `LoggerBuilder`, `ErrorHandler` type alias,
  thread-local `context` module, and convenience macros (`trace!`,
  `debug!`, `info!`, `warn!`, `error!`, `log_at!`).

### Features

- Three output formats: JSON (line-delimited, with optional pretty
  mode and RFC 3339 timestamps), logfmt (`key=value`), and
  human-readable (timestamp + aligned level column).
- Per-target filtering with `RUST_LOG`-style directives.
- Thread-local context (`with_trace_id`, `with_request_id`,
  `with_field`) with RAII guards; reentrancy-safe.
- Per-logger default fields (`with_default_field`) for service /
  version / region identifiers.
- Multi-sink fan-out: install multiple sinks per logger; the first
  error is surfaced but every sink still receives the record.
- Source location capture: macros attach `file!()`, `line!()`,
  `module_path!()` to every record.
- Error handler callback (`on_error`) for surfacing sink failures
  without taking down the calling thread.
- Hand-rolled RFC 3339 timestamp formatter (no dependencies).
- `no_std` build mode for the data model and all three formatters.

### Hardening

- Mutex poisoning on sink locks is recovered silently. A panic that
  briefly held a sink lock does not disable logging.
- `context::with_snapshot` is reentrancy-safe. A sink that emits a
  log record from inside its own `write_record` sees an empty
  context snapshot rather than panicking.
- `HumanFormat` escapes newlines / tabs / CRs in keys, values,
  message, and target so the one-record-per-line contract holds for
  any UTF-8 input.

### Performance

- Per-record formatting uses a thread-local `String` buffer; the
  steady-state hot path is allocation-free.
- JSON escaper writes clean ASCII runs as chunks instead of
  character-by-character.
- Format-only JSON: ~131 ns/record; full pipeline JSON: ~59 ns/call.
- Single-thread throughput: ~15 M records/sec to a discarding
  writer; ~28 M at four threads.

### Testing

- 112 tests: 61 unit + 18 integration + 16 stress + 11 property +
  6 doctest.
- Property tests via `proptest` cover formatter shape invariants
  and parser round-trips.
- Fuzz harnesses under `fuzz/` (`cargo-fuzz`, nightly-only) cover
  the filter directive parser, JSON / logfmt escapers, and
  `Level::from_str`.

### Quality gates

- `#![forbid(unsafe_code)]` crate-wide.
- Clippy strict (`-D warnings`) across `--all-features` and
  `--no-default-features` matrices.
- Build matrix green on stable and on MSRV 1.75.
- `cargo-semver-checks` wired into CI.

## [0.10.0] - 2026-05-18

Pre-release hardening line, superseded by 1.0.0 the same day. See
git tag `v0.10.0`.

## [0.9.0] - 2026-05-18

First pre-release implementation, superseded by 0.10.0 the same day.
See git tag `v0.9.0`.

## [0.1.0] - 2026-05-12

Initial repository scaffold: Apache-2.0 license, README, REPS
specification stub, CI workflow, and crate name reservation on
crates.io.

[Unreleased]: https://github.com/jamesgober/log-io/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/jamesgober/log-io/releases/tag/v1.0.0
[0.10.0]: https://github.com/jamesgober/log-io/releases/tag/v0.10.0
[0.9.0]: https://github.com/jamesgober/log-io/releases/tag/v0.9.0
[0.1.0]: https://github.com/jamesgober/log-io/releases/tag/v0.1.0

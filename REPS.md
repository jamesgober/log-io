# log-io - Project Specification (REPS)

> Authoritative specification for the public API surface and design
> contract of `log-io`. The portfolio-wide engineering standard (REPS:
> Rust Efficiency & Performance Standards) applies in full; this file
> records how log-io meets it and where it is specialised.

## 1. Identity

- **Crate name:** `log-io`
- **Author:** James Gober <me@jamesgober.com>
- **Repository:** https://github.com/jamesgober/log-io
- **License:** Apache-2.0
- **Edition:** 2024
- **MSRV:** 1.85

## 2. Mission

Structured logging pipeline for Rust: filter, format, sink. A hot path
that does not allocate in steady state, JSON / logfmt / human-readable
output that is safe for any input, context propagation across threads
and async tasks, per-target filtering, and output that is not lost at
shutdown. Usable on its own or as the output stage for the `log` and
`tracing` ecosystems.

## 3. Scope

1. A logger admits a record subject to a target / severity filter.
2. The logger assembles ambient fields (defaults, thread context,
   integration-supplied context), removes duplicate keys, applies
   redaction, and dispatches the record to each sink in order.
3. Each sink serializes the record with its format and delivers it.
4. Records at or above the flush level are flushed before the call
   returns (a `NonBlocking` sink under `Overflow::DropNewest` queues the
   flush instead of waiting); shutdown drains and flushes every sink.

The built-in synchronous sinks write on the caller's thread.
`sink::NonBlocking` moves I/O to a background thread with a bounded
queue. Context is thread-local; `context::ContextSnapshot` carries it
into other threads and async tasks without depending on a runtime.

## 4. Public API

The complete reference with examples is [`docs/API.md`](docs/API.md);
rustdoc on docs.rs is authoritative for signatures. Summary:

### Data model (`no_std`, no `alloc`)

- `Level` (exhaustive: Trace, Debug, Info, Warn, Error, Off),
  `ParseLevelError`
- `Value<'a>` (`#[non_exhaustive]`): Null, Bool, I64, U64, I128, U128,
  F64, Str, Char, Bytes, Display, Debug, Error
- `Field<'a>`, `Metadata<'a>` (`#[non_exhaustive]`), `Record<'a>`
  (`#[non_exhaustive]`)

### Formats (`no_std`)

- `format::Format` (object-safe `write_record(&self, &Record, &mut dyn
  fmt::Write)`, provided `write_record_generic` fast path)
- `format::{JsonFormat, LogfmtFormat, HumanFormat}` behind `json`,
  `logfmt`, `human`; `format::{from_fn, FnFormat}`
- `format::ColorChoice` (`std`)

### Pipeline (`std`)

- `Logger`, `LoggerBuilder`, `BuildError`, `ErrorHandler`, `REDACTED`
- `Filter`, `FilterRule`, `ParseFilterError`, `ParseFilterErrorKind`
- `Sink` (`write_record`, `flush`, provided `request_flush` and
  `shutdown`)
- `sink::{StdoutSink, StderrSink, FileSink, FileSinkBuilder,
  WriterSink, NullSink, NonBlocking, NonBlockingBuilder, Overflow,
  RotatingFile, RotatingFileBuilder, Rotation, RotatingFileSink}`
- `context::{with_field, with_trace_id, with_request_id, with_snapshot,
  capture, clear, len, ContextGuard, ContextSnapshot, WithContext,
  MAX_CONTEXT_SLOTS}`
- `init`, `set_global`, `FlushGuard`, `SetGlobalError`,
  `global::{get, set, init, flush, shutdown, install_panic_hook}`
- `Error` (`#[non_exhaustive]`), `Result`
- Macros: `log_at!`, `trace!`, `debug!`, `info!`, `warn!`, `error!`

### Integrations

- `compat::LogBridge`, `compat::to_level_filter` (feature `log`)
- `compat::TracingLayer` (feature `tracing`)

## 5. Safety contract

No `unsafe` code: the crate root carries `#![forbid(unsafe_code)]`.
Library code does not call `unwrap` / `expect` (denied by lint). The
only documented panic is `LoggerBuilder::build` without a sink, a
programming error; `try_build` reports it as a value instead.

## 6. Lints

The crate root denies `warnings`, `missing_docs`, `unused_must_use`,
`unused_results`, `clippy::unwrap_used`, `clippy::expect_used`,
`clippy::todo`, `clippy::unimplemented`, `clippy::print_stdout`,
`clippy::print_stderr`, `clippy::dbg_macro`, `clippy::unreachable` and
`clippy::undocumented_unsafe_blocks`. Test modules may allow `unwrap`
and `expect`.

## 7. MSRV and toolchain policy

MSRV is 1.85 (first release with edition 2024). Raising it is a major
version change and is recorded in the CHANGELOG. `rust-toolchain.toml`
selects the `stable` channel so local and CI stable jobs build with
current stable; the CI `msrv` job builds and tests on 1.85 exactly.
Every dependency, including dev-dependencies, must build on the MSRV.

## 8. Performance contract

[`BENCH.md`](BENCH.md) records the measured numbers, the machine, the
toolchain and the method (criterion medians). They are indicative, not
guarantees. The contract is behavioural:

- Emitting a record through the built-in synchronous sinks performs no
  heap allocation in steady state, including timestamps, thread
  context, default fields, redaction, `format!`-style messages, and the
  `log` and `tracing` bridges, as long as at most 16 fields per group
  (defaults + context, call fields, span fields) are attached.
  `tests/alloc.rs` enforces this with a counting allocator.
- A record rejected by the filter costs one comparison when it is below
  the filter's minimum level.
- Formatting happens outside sink locks.
- A change that regresses a tracked benchmark by more than 5% needs a
  recorded justification (REPS performance gate).

## 9. Stability guarantees

Semantic versioning. The public surface in section 4 is stable across
`2.x.y`. `Value`, `Error`, `Metadata`, `Record`, `ParseFilterErrorKind`,
`BuildError`, `Overflow`, `Rotation` and `ColorChoice` are
`#[non_exhaustive]` and may grow in minor releases. Output formats may
gain fields in minor releases; existing keys and escaping rules change
only in a major release.

Not covered: benchmark numbers, items marked `#[doc(hidden)]` (the
`__private` macro support module), and crate-internal items.

## 10. Dependency policy

Zero runtime dependencies in the default build. Optional integrations
must be behind a non-default feature, mature, maintained, and
justified in the CHANGELOG:

| Feature | Dependencies | Why |
|---|---|---|
| `log` | `log` (std, kv) | The ecosystem's logging facade |
| `tracing` | `tracing-core`, `tracing-subscriber` (std) | Output stage for `tracing` users |

Dev-dependencies: `proptest`, `criterion`, `serde_json`, `tracing`,
`tracing-subscriber`. `cargo deny` (see `deny.toml`) and `cargo audit`
run in CI.

## 11. Testing requirements

- Unit tests next to the code (`#[cfg(test)]` modules).
- `tests/end_to_end.rs`: every output path, sink, macro form and the
  async context helpers.
- `tests/regressions.rs`: one test per audit finding.
- `tests/durability.rs`: child-process tests of exit behaviour.
- `tests/global.rs`: the global logger lifecycle.
- `tests/stress.rs`: large inputs, real mutex poisoning, concurrency.
- `tests/property.rs`: format validity (serde_json, a reference logfmt
  parser), filter and context models, RFC 3339 against a naive calendar.
- `tests/alloc.rs`: the zero-allocation contract.
- `fuzz/`: six cargo-fuzz targets, smoke-run in CI.

## 12. Documentation requirements

Every public item carries rustdoc; non-trivial items have runnable
examples. `docs/API.md` is the narrative reference, `docs/MIGRATION.md`
the upgrade guide, `BENCH.md` the performance record, and
`docs/release/` the release notes.

## 13. Out of scope

- OpenTelemetry export, and ECS / GELF / Bunyan key layouts. A custom
  `Format` can produce them.
- Sampling and rate limiting. Planned; not shipped because doing it
  well needs per-callsite state in the macros.
- Compression of rotated files.
- Async (`Future`-returning) sinks. `NonBlocking` keeps I/O off async
  executor threads without tying the crate to a runtime.

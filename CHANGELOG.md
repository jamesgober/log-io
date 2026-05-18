# Changelog

All notable changes to this project are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.10.0] - 2026-05-18

Pre-1.0 hardening release. Breaks the builder API surface; tighter
defaults, faster hot path, deeper test coverage.

### Added

- `LoggerBuilder::with_default_field` - attach a field to every
  record (typical use: `service`, `version`, `env`).
- `LoggerBuilder::on_error` - install a callback invoked when a sink
  returns an error, so failures are not silently dropped.
- `Logger::try_emit` - lowest-level entry point that accepts a
  pre-built `Metadata`. The macros forward to it so source location
  (`file!()`, `line!()`) is now captured automatically.
- `Logger::sink_count` - diagnostic accessor.
- Format-shortcut methods on the builder: `.stdout_json()`,
  `.stdout_logfmt()`, `.stdout_human()`, and the `stderr_*` mirrors
  (all feature-gated).
- New examples: `multi_sink`, `custom_format`, `custom_sink`,
  `default_fields`.
- Stress test suite (`tests/stress.rs`): huge messages, panics in
  sinks, mutex poison recovery, context overflow, multi-logger
  isolation, etc.
- Property tests (`tests/property.rs`) via `proptest` dev-dep.
  Caught a real bug in `HumanFormat` (newlines in field keys broke
  the one-record-per-line invariant); fixed.
- Fuzz targets in `fuzz/`: `filter_directive`, `json_escape`,
  `logfmt_format`, `level_parse` (nightly-only).
- `BENCH.md` documenting performance numbers and methodology.
- `benches/concurrent.rs` measuring multi-thread throughput.

### Changed

- **Breaking:** builder no longer carries a format generic.
  `Logger::builder().stdout().json().build()` is now
  `Logger::builder().stdout_json().build()` or
  `.stdout(JsonFormat::new()).build()`. Cleaner type signature; no
  type-state weirdness.
- **Breaking:** removed `.file()`, `.file_append()`, `.null()`, and
  the factory-style `.writer(closure)`. Use `.with_sink(FileSink::…?)`
  for files (opening is fallible and shouldn't hide inside `build()`),
  `.with_sink(NullSink::new())` for null, and `.writer(W, F)` passing
  the writer directly.
- **Breaking:** renamed `Logger::try_log_with_target` to
  `Logger::try_log`.
- **Breaking:** renamed macro `warn_!` to `warn!`. Rust allows
  macros to be named `warn`; there is no actual conflict with the
  `warn` attribute in expression position.
- **Breaking:** dispatch now continues across sink failures. The
  first error is surfaced, but every sink still receives the record
  so a transient failure on one destination does not starve others.
- **Breaking:** `HumanFormat` no longer requires the `std` feature.
  The crate's RFC 3339 formatter was already `no_std`-compatible.
- Performance: per-record formatting uses a thread-local `String`
  buffer; the steady-state hot path is allocation-free.
- Performance: JSON escaper writes clean ASCII runs as single chunks
  instead of character-by-character.
- Performance: format-only JSON cost dropped from ~160 to ~131 ns
  per record; full-pipeline JSON cost dropped from ~93 to ~59 ns.
- Hardening: mutex poisoning on sink locks is now recovered
  silently. A panic that briefly held a sink lock no longer disables
  logging forever.
- Hardening: `context::with_snapshot` is reentrancy-safe; a sink
  that emits a log record from inside its own `write_record` sees an
  empty context snapshot instead of panicking.
- Hardening: `HumanFormat` now escapes newlines / tabs / CRs in
  keys, values, message, and target so the line-per-record contract
  holds for any UTF-8 input.

### Removed

- `NoFormat` is no longer a usable `Format`. Building a logger
  without explicitly choosing a sink / format combination now panics
  in `build()`.

## [0.9.0] - 2026-05-18

Initial pre-1.0 stabilization line. See git tag `v0.9.0` for the
full feature list. Superseded by 0.10.0 within the same release day.

## [0.1.0] - 2026-05-12

### Added

- Initial repository scaffold.
- Apache-2.0 license, README, REPS specification stub, CI workflow,
  `.dev/` planning structure (DIRECTIVES, ROADMAP, PROMPTS).
- Crate name reserved on crates.io.

[Unreleased]: https://github.com/jamesgober/log-io/compare/v0.10.0...HEAD
[0.10.0]: https://github.com/jamesgober/log-io/releases/tag/v0.10.0
[0.9.0]: https://github.com/jamesgober/log-io/releases/tag/v0.9.0
[0.1.0]: https://github.com/jamesgober/log-io/releases/tag/v0.1.0

# Changelog

All notable changes to this project are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.1.0] - 2026-10-23

Additive release: nothing that worked with 2.0 changes behaviour.

### Added

- `sink::RecentLines<F>`: a ring of the last N formatted lines for crash
  reports and status pages. Slots are allocated up front (`capacity`
  lines of at most `line_limit` bytes, `DEFAULT_LINE_LIMIT` = 1 KiB), so
  memory is fixed and writing does not allocate; longer lines are cut on
  a character boundary. `snapshot` and `write_to` read it oldest first;
  `try_snapshot` and `try_write_to` never block, so a panic hook can read
  the ring even if the panic happened while it was being written.
  Redacted values stay redacted.
- `FileSinkBuilder::mode(bits)` and `RotatingFileBuilder::mode(bits)`:
  Unix permission bits for every file the sink creates (for a rotating
  file, also each new file after a rotation). The umask still applies and
  existing files keep their mode. Without the call nothing changes. No
  effect on Windows, where files inherit the directory's access control
  list.

## [2.0.0] - 2026-10-13

Major release. Fixes every finding of the 2026-10-13 audit, several of
which required breaking changes, and adds the features the 1.x line was
missing (global logger, durable shutdown, `log` / `tracing` bridges,
background and rotating sinks). Upgrade guide:
[`docs/MIGRATION.md`](docs/MIGRATION.md).

### Breaking

- MSRV raised from 1.75 to **1.85**; the crate moved to edition 2024.
- `Format::write_record` takes `&mut dyn fmt::Write`, making `Format`
  object-safe (`Box<dyn Format>`, `&dyn Format`). The statically
  dispatched fast path is the new provided method
  `write_record_generic`, which the built-in formats override.
- `Value`, `Error`, `Metadata` and `Record` are `#[non_exhaustive]`.
  `Value` gained variants holding trait-object references, so `Value`,
  `Field` and `Record` are no longer `Send`, `Sync`, `UnwindSafe` or
  `RefUnwindSafe`; `Value`'s `PartialEq` compares the lazy variants by
  identity.
- `ContextGuard` is `!Send` and `!Sync`, so a guard can no longer be
  moved to (and dropped on) another thread or held across an `.await`
  in a `Send` future.
- Macro messages are format strings: literal braces must be escaped.
  The logger argument must dereference to `&Logger`.
- Filters pick the most specific matching rule instead of the first one;
  `Filter::parse` rejects empty input (including a directive with only
  empty segments, such as `","`), empty targets, targets ending in a
  separator, and targets containing whitespace; `ParseFilterError` is a
  struct with `kind()` and `segment()`. Records at `Level::Off` are never
  admitted.
- Records at `Level::Error` and above are flushed before the logging call
  returns (`LoggerBuilder::flush_on(Level::Off)` restores 1.x behaviour).
  A `NonBlocking` sink with `Overflow::DropNewest` is the exception: it
  never waits, and only queues the flush for its worker.
  Dropping the last `Logger` clone shuts its sinks down.
- `on_error` is called once per failing sink and for flush errors.
- Output changes: reserved keys are written as `fields.<key>`, duplicate
  keys are removed (including repeated call-site keys, where the last one
  wins, and `level` / `fields.level` pairs), floats use the round-trippable `{:?}` form, logfmt
  escapes are JSON-compatible, more characters are escaped in every
  format, `stdout_human` / `stderr_human` colour on terminals. Full table
  in the migration guide.
- `Error`'s `Display` text names the failing stage.

### Added

- Global logger: `log_io::init` (returns a `FlushGuard`),
  `log_io::set_global`, and `log_io::global::{get, flush, shutdown,
  install_panic_hook}`. Macros without a logger argument use it.
- `Logger::shutdown`, `Logger::flush_guard`, `Logger::filter`,
  `Logger::default_target`.
- `LoggerBuilder::try_build` with `BuildError`, `try_filter_directive`,
  `env_filter` (`RUST_LOG`-style directives from an environment
  variable), `flush_on`, `default_target`, `redact_key`,
  `redact_common_secrets` (and `COMMON_SECRET_KEYS`).
- Macros: `format!`-style messages with inline captures, positional
  arguments, dotted keys (`http.status = 200`), string-literal keys,
  `%` (Display) and `?` (Debug) value sigils, and `target:`.
- `Value::{I128, U128, Bytes, Display, Debug, Error}`, plus `From`
  conversions for `i128`, `u128`, `&[u8]`, `&String`, `&Cow<str>`,
  `&Vec<u8>`.
- `context::capture` and `ContextSnapshot` (`attach`, `scope`, `wrap`
  for futures) to carry context across threads and async tasks.
- `sink::NonBlocking`: background writer with a bounded queue, an
  `Overflow` policy (`Block` or `DropNewest`), drop and error counters,
  and synchronous fallback after shutdown. Its writer and `on_error`
  handler may log into the same sink without deadlocking.
- `sink::RotatingFile` (size and UTC time rotation, retention, safe on
  Windows, all-or-nothing rotation that never loses rotated history)
  and the `RotatingFileSink` alias.
- `FileSink::builder` with `buffer_capacity` and `sync_on_flush`;
  `WriterSink::{with_writer, into_inner}`.
- `Sink::shutdown` (provided, defaults to `flush`) and
  `Sink::request_flush` (provided, defaults to `flush`; called for
  records at the flush level, so a background sink can avoid waiting).
- `format::from_fn` closure adapter; `HumanFormat::color` and
  `format::ColorChoice` (TTY detection, `NO_COLOR`, `CLICOLOR_FORCE`);
  `LogfmtFormat::timestamp_rfc3339`.
- `Filter::{min_level, with_default_level}`, `FromStr for Filter`,
  `ParseFilterErrorKind`.
- `Error::{Closed, Custom}` and `Error::custom`.
- `log_io::REDACTED`, `log_io::ErrorHandler` and `log_io::BuildError`
  at the crate root.
- Feature `log`: `compat::LogBridge`, a `log::Log` implementation that
  keeps key-values. Adds the `log` crate (MIT OR Apache-2.0; no further
  dependencies with the `std` and `kv` features used). `log` is the
  de-facto logging facade, so this lets every crate that logs through it
  write to log-io sinks.
- Feature `tracing`: `compat::TracingLayer`, a `tracing-subscriber`
  layer that attaches span fields as context. Adds `tracing-core` and
  `tracing-subscriber` (MIT), with only their `std` features. This is the
  integration most async applications need.

### Changed

- Thread-local format and message buffers shrink back to 64 KiB after
  an oversized record instead of keeping the peak size forever.
- `Logger::enabled` rejects records below the filter's minimum level
  without scanning rules; the macros check the filter once (1.x checked
  it twice).
- `StdoutSink` / `StderrSink` drop their redundant mutex and write under
  the standard stream's lock.
- `HumanFormat` omits the `[target]` column for an empty target.
- Benchmarks use criterion 0.7 and include the default configuration
  (timestamps and context on).
- Documentation: corrected the "async-safe sinks" claim (sinks are
  thread-safe and synchronous; `NonBlocking` moves I/O off the caller),
  the `&dyn Format` / closure claims, the performance tables
  (re-measured), the rotation advice, and broken links.

### Fixed

Found by the 2026-10-14 re-audit of this release before publication:

- A `RotatingFile` rotation whose final rename failed (on Windows,
  another process holding the active file without delete sharing) had
  already deleted the oldest rotated file and shifted the rest, so every
  retry deleted one more file of history. Rotation now moves the active
  file aside first, prunes only after every rename succeeded, and undoes
  completed renames on failure (LOG-R01).
- With `global::install_panic_hook`, a panic on a `NonBlocking` worker
  thread hung the process forever and swallowed the panic message: the
  hook logged into the same sink and waited for the panicking thread.
  The hook now skips the logger on background worker threads, and a
  `NonBlocking` sink never waits for itself (LOG-R02).
- Under `Overflow::DropNewest`, one `error!` waited for the whole
  `NonBlocking` queue to be written (357 ms measured), contradicting
  "never waits". Flush-level records now only queue a flush request
  there (LOG-R03).
- A `ContextGuard` held across an `.await` inside a future wrapped with
  `ContextSnapshot::wrap` leaked into sibling futures polled on the same
  thread (`join!`, `LocalSet`); the wrapper now keeps such context with
  its future between polls (LOG-R04).
- `redact_common_secrets` missed hyphenated header names (`x-api-key`,
  `set-cookie`, `proxy-authorization`) and some common keys; matching
  now treats `-` and `_` alike and the list grew (LOG-R05).
- Repeated call-site keys (`info!(lg, "m", a = 1, a = 2)`) were written
  twice (LOG-R06).
- `NonBlockingBuilder::build` panicked for a thread name containing NUL
  and for huge capacities instead of returning an error (LOG-R07).
- The directive `","` parsed to an all-`Off` filter, silencing all
  logging (LOG-R08).
- `RotatingFile` read the clock on every write even without a time
  schedule (LOG-R09).

From the 2026-10-13 audit of 1.0.0:

- Buffered `FileSink` records were lost when the logger lived in a
  `static` or the process exited without dropping it (LOG-001).
- A `ContextGuard` dropped on another thread leaked its field into
  unrelated records on the original thread (LOG-002).
- `context::with_field` called from a sink panicked; a guard dropped in
  a sink leaked; a guard dropped during thread-local teardown aborted
  the process (LOG-003, LOG-008).
- First-match filter precedence ignored more specific rules (LOG-004).
- Human-format terminal and newline injection, including through
  `Value::Char` (LOG-005); logfmt keys passed control characters and
  empty keys produced ` =v` (LOG-016).
- Duplicate keys and user fields named like metadata keys could spoof
  `level` / `message` in last-wins JSON parsers (LOG-006).
- The per-thread format buffer retained the largest record ever logged
  (LOG-007).
- `ErrorHandler` was not reachable from the crate root (LOG-009).
- Floats were written without `.` or exponent (`1`), as 300+ digit
  strings, or in a form serde_json rejects (`f64::MAX`) (LOG-012).
- `on_error` saw only the first sink error of a record (LOG-014), and a
  handler that logged through a failing logger recursed until the stack
  overflowed (LOG-015).
- Degenerate filter directives were accepted silently and
  `filter_directive` swallowed errors (LOG-017).
- `Logger::log`'s empty target could not be matched by target rules; it
  is now configurable (LOG-018).
- RFC 3339 timestamps wrapped for out-of-range instants (LOG-019).
- `cargo test --no-default-features` failed on an ungated doctest
  (LOG-022).

### Security

- All three formats escape C0 and C1 control characters, DEL, U+2028,
  U+2029 and the bidirectional override and isolate characters, so log
  output cannot drive a terminal or forge extra lines.
- Field names cannot shadow record metadata, and secrets can be
  redacted at the logging boundary.

### Testing

- Unit and integration tests, a regression test for every audit
  finding, child-process durability tests, property tests validated
  with `serde_json` and a reference logfmt parser, concurrency stress
  tests, and a counting-allocator test that keeps the hot path
  allocation-free (counts in the release notes).
- Six fuzz targets (JSON, logfmt and human formats on arbitrary records,
  the full logger pipeline, the filter parser, level parsing),
  smoke-run in CI.
- CI: Linux, macOS and Windows across default, all, no-default and
  std-only feature sets; each feature on its own; MSRV build and test;
  `no_std` target build; `cargo audit`; `cargo deny`; docs with
  `-D warnings`; semver checks; release-mode tests; fuzz smoke runs.

### Dependencies (dev only)

- `criterion` 0.7 for benchmarks (REPS requires criterion).
- `serde_json` (with `float_roundtrip`) to validate JSON output.
- `tracing` and `tracing-subscriber` (with `registry`) to test the
  `tracing` bridge.

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
specification stub and CI workflow. This version was not published to
crates.io.

[Unreleased]: https://github.com/jamesgober/log-io/compare/v2.1.0...HEAD
[2.1.0]: https://github.com/jamesgober/log-io/compare/v2.0.0...v2.1.0
[2.0.0]: https://github.com/jamesgober/log-io/compare/v1.0.0...v2.0.0
[1.0.0]: https://github.com/jamesgober/log-io/compare/v0.10.0...v1.0.0
[0.10.0]: https://github.com/jamesgober/log-io/releases/tag/v0.10.0
[0.9.0]: https://github.com/jamesgober/log-io/releases/tag/v0.9.0
[0.1.0]: https://github.com/jamesgober/log-io/releases/tag/v0.1.0

# log-io - Project Specification (REPS)

> Authoritative specification for the public API surface and design
> contract of `log-io`.

## 1. Identity

- **Crate name:** `log-io`
- **Author:** James Gober <me@jamesgober.com>
- **Repository:** https://github.com/jamesgober/log-io
- **License:** Apache-2.0
- **MSRV:** 1.75

## 2. Mission

Structured logging pipeline for Rust. Zero-allocation steady-state
hot path, JSON / logfmt / human-readable outputs, context propagation
(request-id, trace-id), per-module filtering, async-safe sinks. An IO
pipeline for log records, not a wrapper around log+tracing.

## 3. Scope

`log-io` provides a small, composable pipeline:

1. A logger admits a record subject to a target/severity filter.
2. The record is dispatched to one or more sinks in installation order.
3. Each sink serializes the record using a configured format.

The pipeline is synchronous. Async runtime integration is the
caller's responsibility: wrap a sink in a channel or batch emissions
at the call site.

## 4. Public API

### Core data model (no_std)

```rust
pub enum Level { Trace, Debug, Info, Warn, Error, Off }
impl Level {
    pub const fn as_str(self) -> &'static str;
    pub const fn as_str_upper(self) -> &'static str;
    pub const fn is_enabled_at(self, threshold: Self) -> bool;
    pub const ALL: [Self; 5];
}
impl FromStr for Level { type Err = ParseLevelError; }

pub enum Value<'a> {
    Null, Bool(bool), I64(i64), U64(u64), F64(f64),
    Str(&'a str), Char(char),
}

pub struct Field<'a> { pub key: &'a str, pub value: Value<'a> }
impl<'a> Field<'a> {
    pub fn new<V: Into<Value<'a>>>(key: &'a str, value: V) -> Self;
}

pub struct Metadata<'a> {
    pub level: Level,
    pub target: &'a str,
    pub file: Option<&'a str>,
    pub line: Option<u32>,
    pub timestamp_unix_nanos: Option<u128>,
}
impl<'a> Metadata<'a> {
    pub const fn new(level: Level, target: &'a str) -> Self;
    pub const fn with_location(self, file: &'a str, line: u32) -> Self;
    pub const fn with_timestamp(self, ts_unix_nanos: u128) -> Self;
}

pub struct Record<'a> {
    pub metadata: Metadata<'a>,
    pub message: &'a str,
    pub fields: &'a [Field<'a>],
    pub context: &'a [Field<'a>],
}
impl<'a> Record<'a> {
    pub const fn new(meta: Metadata<'a>, msg: &'a str, fields: &'a [Field<'a>]) -> Self;
    pub const fn with_context(
        meta: Metadata<'a>, msg: &'a str,
        fields: &'a [Field<'a>], context: &'a [Field<'a>],
    ) -> Self;
    pub fn all_fields(&self) -> impl Iterator<Item = &Field<'a>>;
}
```

### Filter

```rust
pub struct Filter { /* ... */ }
pub struct FilterRule { pub target: String, pub level: Level }
pub struct ParseFilterError;

impl Filter {
    pub fn new(default_level: Level) -> Self;
    pub fn with_rule(self, target: impl Into<String>, level: Level) -> Self;
    pub fn default_level(&self) -> Level;
    pub fn rules(&self) -> &[FilterRule];
    pub fn is_enabled(&self, target: &str, level: Level) -> bool;
    pub fn threshold_for(&self, target: &str) -> Level;
    pub fn parse(directive: &str) -> Result<Self, ParseFilterError>;
}
```

### Format

```rust
pub mod format {
    pub trait Format: Send + Sync {
        fn write_record<W: core::fmt::Write + ?Sized>(
            &self, record: &Record<'_>, writer: &mut W,
        ) -> core::fmt::Result;
    }
    pub struct JsonFormat;   // feature = "json"
    pub struct LogfmtFormat; // feature = "logfmt"
    pub struct HumanFormat;  // feature = "human" (no_std-compatible)
}
```

### Sinks (std only)

```rust
pub mod sink {
    pub trait Sink: Send + Sync {
        fn write_record(&self, record: &Record<'_>) -> Result<()>;
        fn flush(&self) -> Result<()>;
    }
    pub struct StdoutSink<F: Format>;
    pub struct StderrSink<F: Format>;
    pub struct FileSink<F: Format>;     // append / truncating-create
    pub struct WriterSink<W, F>;        // wrap any Send + Write
    pub struct NullSink;
}
```

### Context (std only)

```rust
pub mod context {
    pub const MAX_CONTEXT_SLOTS: usize = 16;
    pub struct ContextGuard;
    pub fn with_field<'a, V: Into<Value<'a>>>(key: &str, value: V) -> ContextGuard;
    pub fn with_trace_id(id: &str) -> ContextGuard;
    pub fn with_request_id(id: &str) -> ContextGuard;
    pub fn with_snapshot<R>(f: impl FnOnce(&[Field<'_>]) -> R) -> R;
    pub fn clear();
    pub fn len() -> usize;
}
```

### Logger (std only)

```rust
pub struct Logger { /* clonable Arc-handle */ }
pub struct LoggerBuilder { /* no generic */ }
pub type ErrorHandler = Arc<dyn Fn(&Error) + Send + Sync>;

impl Logger {
    pub fn builder() -> LoggerBuilder;
    pub fn threshold_for(&self, target: &str) -> Level;
    pub fn enabled(&self, target: &str, level: Level) -> bool;
    pub fn min_level(&self) -> Level;
    pub fn sink_count(&self) -> usize;
    pub fn log(&self, level: Level, message: &str, fields: &[Field<'_>]);
    pub fn try_log(&self, level: Level, target: &str, message: &str,
                   fields: &[Field<'_>]) -> Result<()>;
    pub fn try_emit(&self, metadata: Metadata<'_>, message: &str,
                    fields: &[Field<'_>]) -> Result<()>;
    pub fn flush(&self) -> Result<()>;
}

impl LoggerBuilder {
    pub fn level(self, level: Level) -> Self;
    pub fn filter(self, filter: Filter) -> Self;
    pub fn target_level(self, target: impl Into<String>, level: Level) -> Self;
    pub fn filter_directive(self, directive: &str) -> Self;
    pub fn no_timestamps(self) -> Self;
    pub fn no_context(self) -> Self;
    pub fn with_default_field<'a, V: Into<Value<'a>>>(
        self, key: impl Into<String>, value: V,
    ) -> Self;
    pub fn on_error(self, handler: ErrorHandler) -> Self;

    pub fn with_sink(self, sink: impl Sink + 'static) -> Self;
    pub fn stdout<F: Format + Send + Sync + 'static>(self, format: F) -> Self;
    pub fn stderr<F: Format + Send + Sync + 'static>(self, format: F) -> Self;
    pub fn writer<W, F>(self, writer: W, format: F) -> Self
        where W: Write + Send + 'static, F: Format + Send + Sync + 'static;

    // Feature-gated shortcuts:
    #[cfg(feature = "json")]   pub fn stdout_json(self)   -> Self;
    #[cfg(feature = "json")]   pub fn stderr_json(self)   -> Self;
    #[cfg(feature = "logfmt")] pub fn stdout_logfmt(self) -> Self;
    #[cfg(feature = "logfmt")] pub fn stderr_logfmt(self) -> Self;
    #[cfg(feature = "human")]  pub fn stdout_human(self)  -> Self;
    #[cfg(feature = "human")]  pub fn stderr_human(self)  -> Self;

    pub fn build(self) -> Logger; // panics if no sink installed
}
```

### Macros (std only)

```rust
log_io::trace!(logger, "msg" [, key = expr]*);
log_io::debug!(logger, "msg" [, key = expr]*);
log_io::info!(logger,  "msg" [, key = expr]*);
log_io::warn!(logger,  "msg" [, key = expr]*);
log_io::error!(logger, "msg" [, key = expr]*);
log_io::log_at!(logger, level, "msg" [, key = expr]*);
```

Every macro captures `file!()`, `line!()`, and `module_path!()` and
attaches them to the record's metadata. The target seen by filters
is the calling module's path.

## 5. Safety contract

No `unsafe` code. The crate root carries `#![forbid(unsafe_code)]`.

## 6. MSRV policy

Pinned at 1.75. Bumps require a minor version increment and a
CHANGELOG entry under `### Changed` with rationale.

## 7. Performance contract

See `BENCH.md` for measured numbers. Indicative:

| Workload                            | ns / record |
|-------------------------------------|------------:|
| JSON, 5 fields, formatter only      | ~131        |
| logfmt, 5 fields, formatter only    | ~159        |
| human, 5 fields, formatter only     | ~210        |
| Pipeline, 3 fields, JSON + writer   | ~59         |
| Pipeline, no fields, JSON + writer  | ~25         |
| Pipeline, filtered out below thresh | ~1          |

Throughput on a single thread is ~15 M records/sec to a discarding
writer; scales to ~28 M at four threads before mutex contention.

## 8. Stability guarantees

The crate is at `1.0.0`. The public API surface listed in section 4
is stable: subsequent `1.x.y` releases preserve backwards
compatibility. Backwards-incompatible changes require a major
version bump and a CHANGELOG entry under `### Changed` flagged
**Breaking**.

The following are NOT covered by the stability guarantee:

- Performance numbers in section 7. They are indicative; small
  regressions or improvements are not breaking.
- Crate-internal items (anything `pub(crate)` or unexported).
- Banned-word policy and other developer-facing items in
  `.dev/DIRECTIVES.md`.

## 9. Dependency policy

Zero runtime dependencies. `dev-dependencies` are allowed (currently
only `proptest`, used in tests).

## 10. Testing requirements

- Unit tests live next to their code.
- `tests/end_to_end.rs` exercises every public output path.
- `tests/stress.rs` covers edge cases: huge messages, panics in
  sinks, mutex poison recovery, context overflow, multi-logger
  isolation, etc.
- `tests/property.rs` covers format invariants and parser
  round-trips via `proptest`.
- `fuzz/` contains nightly-only fuzz targets for the filter parser,
  JSON / logfmt formatters, and `Level::from_str`.

## 11. Documentation requirements

Every public item carries a rustdoc block. `docs/API.md` is a
narrative companion. `BENCH.md` documents the performance contract.

## 12. Out of scope

- Async sinks. Compose by wrapping a channel-backed writer in
  [`crate::sink::WriterSink`].
- Log rotation. Use external tooling (`logrotate`, `cronolog`) or
  open a fresh `FileSink` on rollover.
- Compatibility shim for `log` or `tracing`. Users who need
  drop-in compatibility should keep using those crates.

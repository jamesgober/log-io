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

Structured logging pipeline for Rust. Zero-allocation fast path, JSON / logfmt / human-readable outputs, context propagation (request-id, trace-id), per-module filtering, async-safe sinks. An IO pipeline for log records, not a wrapper around log+tracing.

## 3. Scope

`log-io` provides a small, composable pipeline:

1. A logger admits a record subject to a target/severity filter.
2. The record is dispatched to one or more sinks.
3. Each sink serializes the record using a configured format.

The pipeline is intentionally synchronous. Async runtime integration is
the caller's responsibility: wrap a sink in a channel or batch the
emissions at the call site.

## 4. Public API

The shape below lists the public surface area. Generic and lifetime
parameters are elided where they would distract; full signatures live
in the rustdoc.

### Core data model

```rust
pub enum Level {
    Trace, Debug, Info, Warn, Error, Off,
}
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
impl<'a> Value<'a> {
    pub const fn is_null(&self) -> bool;
    pub const fn variant_name(&self) -> &'static str;
}
// From<&str>, From<bool>, From<{integers}>, From<{floats}>,
// From<Option<T>> where T: Into<Value<'_>>.

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

### Filter (std only)

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
    pub struct HumanFormat;  // feature = "human" (requires std)
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
pub struct LoggerBuilder<F> { /* ... */ }

impl Logger {
    pub fn builder() -> LoggerBuilder<NoFormat>;
    pub fn threshold_for(&self, target: &str) -> Level;
    pub fn enabled(&self, target: &str, level: Level) -> bool;
    pub fn min_level(&self) -> Level;
    pub fn log(&self, level: Level, message: &str, fields: &[Field<'_>]);
    pub fn try_log_with_target(
        &self, level: Level, target: &str,
        message: &str, fields: &[Field<'_>],
    ) -> Result<()>;
    pub fn flush(&self) -> Result<()>;
}

impl<F> LoggerBuilder<F> {
    // Configuration
    pub fn level(self, level: Level) -> Self;
    pub fn filter(self, filter: Filter) -> Self;
    pub fn target_level(self, target: impl Into<String>, level: Level) -> Self;
    pub fn filter_directive(self, directive: &str) -> Self;
    pub fn no_timestamps(self) -> Self;
    pub fn no_context(self) -> Self;
    pub fn add_sink(self, sink: impl Sink + 'static) -> Self;

    // Output target selection
    pub fn stdout(self) -> Self;
    pub fn stderr(self) -> Self;
    pub fn file(self, path: impl Into<PathBuf>) -> Self;
    pub fn file_append(self, path: impl Into<PathBuf>) -> Self;
    pub fn writer<W, MakeW>(self, factory: MakeW) -> Self
        where W: Write + Send + 'static, MakeW: FnOnce() -> W + Send + 'static;
    pub fn null(self) -> Self;

    // Format selection (each consumes the builder and changes F)
    pub fn json(self) -> LoggerBuilder<JsonFormat>;            // feature
    pub fn json_with(self, fmt: JsonFormat) -> LoggerBuilder<JsonFormat>;
    pub fn logfmt(self) -> LoggerBuilder<LogfmtFormat>;        // feature
    pub fn human(self) -> LoggerBuilder<HumanFormat>;          // feature
    pub fn human_with(self, fmt: HumanFormat) -> LoggerBuilder<HumanFormat>;
}

impl<F: Format + Clone + 'static> LoggerBuilder<F> {
    pub fn build(self) -> Logger;
}
```

### Macros (std only)

```rust
log_io::trace!(logger, "msg" [, key = expr]*);
log_io::debug!(logger, "msg" [, key = expr]*);
log_io::info!(logger,  "msg" [, key = expr]*);
log_io::warn_!(logger, "msg" [, key = expr]*); // `warn` collides with attribute
log_io::error!(logger, "msg" [, key = expr]*);
log_io::log_at!(logger, level, "msg" [, key = expr]*);
```

## 5. Safety contract

No `unsafe` code is permitted. The crate root carries
`#![forbid(unsafe_code)]`.

## 6. MSRV policy

Pinned at 1.75. Bumps require a minor version increment and a
CHANGELOG entry under `### Changed` with rationale.

## 7. Performance contract

Measured on a developer laptop (release, LTO thin):

| Workload                            | ns / record |
|-------------------------------------|------------:|
| JSON, 5 fields, formatter only      | ~160        |
| logfmt, 5 fields, formatter only    | ~150        |
| human, 5 fields, formatter only     | ~180        |
| Pipeline, 3 fields, JSON + writer   | ~93         |
| Pipeline, no fields, JSON + writer  | ~44         |
| Pipeline, filtered out below thresh | ~4          |

Numbers are indicative, not contractual; rerun `cargo bench` on the
target machine to verify.

## 8. Stability guarantees

`0.x.y` releases are not API-stable. Stability begins at `1.0.0`.

## 9. Dependency policy

Zero runtime dependencies. The crate depends on `core` and (for
non-trivial features) `std`. Any future runtime dependency requires a
documented justification in `.dev/DESIGN.md` or its successor and a
minor-version bump.

## 10. Testing requirements

- Unit tests next to their code (`#[cfg(all(test, feature = "std"))]`).
- Integration tests in `tests/end_to_end.rs` cover every public output
  path: JSON, logfmt, human, filter directive, context, custom sink,
  macros, unicode, and concurrent writers.
- Doctests on every public API surface.

## 11. Documentation requirements

Every public item carries a rustdoc block. `docs/API.md` is a
narrative companion intended for offline reading.

## 12. Out of scope

- Async sinks. A user can build one by wrapping a channel-backed
  writer in [`crate::sink::WriterSink`].
- Log rotation. Use a rotating file utility (e.g. `logrotate`,
  `cronolog`) externally, or open a fresh [`crate::sink::FileSink`]
  on rollover.
- Compatibility shim for the `log` or `tracing` crates. Users who need
  drop-in compatibility should keep using those crates.

# log-io - API Reference

> Complete reference for the public API of `log-io`. The rustdoc on
> [docs.rs/log-io](https://docs.rs/log-io) is generated from the same
> source comments; this document is the offline-friendly mirror.

## Table of contents

1. [Overview](#overview)
2. [Feature flags](#feature-flags)
3. [Crate-level items](#crate-level-items)
4. [Data model](#data-model)
   - [`Level`](#level)
   - [`Value`](#value)
   - [`Field`](#field)
   - [`Metadata`](#metadata)
   - [`Record`](#record)
5. [Filtering](#filtering)
   - [`Filter`](#filter)
   - [`FilterRule`](#filterrule)
   - [`ParseFilterError`](#parsefiltererror)
6. [Formats](#formats)
   - [`Format` trait](#format-trait)
   - [`JsonFormat`](#jsonformat)
   - [`LogfmtFormat`](#logfmtformat)
   - [`HumanFormat`](#humanformat)
7. [Sinks](#sinks)
   - [`Sink` trait](#sink-trait)
   - [`StdoutSink`](#stdoutsink)
   - [`StderrSink`](#stderrsink)
   - [`FileSink`](#filesink)
   - [`WriterSink`](#writersink)
   - [`NullSink`](#nullsink)
8. [Errors](#errors)
   - [`Error`](#error)
   - [`Result`](#result)
9. [Logger](#logger)
   - [`Logger`](#logger-1)
   - [`LoggerBuilder`](#loggerbuilder)
   - [`ErrorHandler`](#errorhandler)
10. [Context propagation](#context-propagation)
11. [Macros](#macros)
12. [`no_std` support](#no_std-support)

## Overview

A record flows through three stages:

```
caller -> Logger -> Filter -> [Sink #0 ... Sink #N]
                                  |
                                  v
                              Format -> Writer
```

- The **logger** captures wall-clock time (optional), splices in
  thread-local context (optional) and any default fields, and
  constructs the [`Record`].
- The **filter** decides whether the record is admitted.
- Each **sink** receives the same `&Record`. Sinks run sequentially
  in insertion order. The first error is surfaced, but every sink
  still receives the record so a single failure does not starve the
  others.

## Feature flags

| Flag     | Default | Effect                                                       |
|----------|---------|--------------------------------------------------------------|
| `std`    | yes     | Enables `Logger`, sinks, context, timestamps.                |
| `json`   | yes     | Enables [`JsonFormat`].                                      |
| `logfmt` | yes     | Enables [`LogfmtFormat`].                                    |
| `human`  | yes     | Enables [`HumanFormat`].                                     |

With `default-features = false`, the crate compiles in `no_std` mode
exposing only the data model, the `Format` trait, and any formatters
whose flags are still enabled. See [`no_std` support](#no_std-support).

## Crate-level items

### `VERSION`

```rust
pub const VERSION: &str;
```

The crate version as set by Cargo at build time.

```rust
assert_eq!(log_io::VERSION, env!("CARGO_PKG_VERSION"));
```

## Data model

### `Level`

```rust
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Off,
}
```

Severity of a log record. Ordered from least to most severe; `Off`
is a sentinel above `Error` used to mean "admit nothing" in filter
thresholds.

#### `Level::as_str`

```rust
pub const fn as_str(self) -> &'static str;
```

Canonical lowercase name (`"trace"`, `"debug"`, ..., `"off"`).

```rust
use log_io::Level;
assert_eq!(Level::Info.as_str(), "info");
```

#### `Level::as_str_upper`

```rust
pub const fn as_str_upper(self) -> &'static str;
```

Uppercase form used by [`HumanFormat`].

```rust
use log_io::Level;
assert_eq!(Level::Warn.as_str_upper(), "WARN");
```

#### `Level::is_enabled_at`

```rust
pub const fn is_enabled_at(self, threshold: Self) -> bool;
```

Returns `true` if `self` is at or above `threshold`.

```rust
use log_io::Level;
assert!(Level::Error.is_enabled_at(Level::Info));
assert!(!Level::Debug.is_enabled_at(Level::Info));
```

#### `Level::ALL`

```rust
pub const ALL: [Self; 5];
```

Every real severity in ascending order. Excludes `Off`.

```rust
use log_io::Level;
for level in Level::ALL {
    println!("{}", level.as_str());
}
```

#### `FromStr` and `Display`

```rust
impl FromStr for Level { type Err = ParseLevelError; }
impl Display for Level;
```

Parsing accepts case-insensitive canonical forms (`off`, `trace`,
`debug`, `info`, `warn`, `error`) and the aliases `warning` /
`err`. Display writes the lowercase form.

```rust
use log_io::Level;
let lvl: Level = "INFO".parse().unwrap();
assert_eq!(lvl, Level::Info);
let lvl: Level = "warning".parse().unwrap();
assert_eq!(lvl, Level::Warn);
```

### `ParseLevelError`

```rust
pub struct ParseLevelError;
```

Returned by `Level::from_str` when the input doesn't match any
canonical form or alias.

```rust
use log_io::Level;
assert!("verbose".parse::<Level>().is_err());
```

### `Value`

```rust
pub enum Value<'a> {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(&'a str),
    Char(char),
}
```

A borrowed field value. The `'a` lifetime ties string values to the
caller's stack so records carry no heap allocation.

#### Constructors via `From`

```rust
use log_io::Value;

let s: Value = "hello".into();              // Value::Str
let n: Value = 42_u64.into();               // Value::U64
let b: Value = true.into();                 // Value::Bool
let f: Value = 1.5_f64.into();              // Value::F64
let null: Value = Option::<u32>::None.into(); // Value::Null
```

`From` is implemented for `&str`, `bool`, `char`, every primitive
integer (signed and unsigned), `f32`, `f64`, and `Option<T>` where
`T: Into<Value<'a>>`.

#### `Value::is_null`

```rust
pub const fn is_null(&self) -> bool;
```

Returns `true` if and only if the variant is `Null`.

#### `Value::variant_name`

```rust
pub const fn variant_name(&self) -> &'static str;
```

Diagnostic-friendly variant name (e.g. `"u64"`, `"str"`).

#### `Default` and `Display`

```rust
impl Default for Value<'_>;   // returns Value::Null
impl Display for Value<'_>;
```

`Display` formats each variant's payload in the obvious way; `Null`
renders as the string `"null"`.

```rust
use log_io::Value;
assert_eq!(Value::U64(5).to_string(), "5");
assert_eq!(Value::Null.to_string(), "null");
```

### `Field`

```rust
pub struct Field<'a> {
    pub key: &'a str,
    pub value: Value<'a>,
}
```

A key-value pair attached to a [`Record`]. Both key and value are
borrowed; the field cannot outlive their backing data.

#### `Field::new`

```rust
pub fn new<V: Into<Value<'a>>>(key: &'a str, value: V) -> Self;
```

**Parameters**

| Name    | Type                 | Description                            |
|---------|----------------------|----------------------------------------|
| `key`   | `&'a str`            | Field name. Should be unique per record. |
| `value` | `impl Into<Value>`   | Any value convertible to a `Value`.    |

```rust
use log_io::{Field, Value};

let f = Field::new("port", 8080_u32);
assert_eq!(f.key, "port");
assert_eq!(f.value, Value::U64(8080));

let s = String::from("alice");
let f = Field::new("user", s.as_str()); // borrow at the call site
```

### `Metadata`

```rust
pub struct Metadata<'a> {
    pub level: Level,
    pub target: &'a str,
    pub file: Option<&'a str>,
    pub line: Option<u32>,
    pub timestamp_unix_nanos: Option<u128>,
}
```

Origin and severity information for a record.

| Field                  | Required | Description                                  |
|------------------------|----------|----------------------------------------------|
| `level`                | yes      | Severity.                                    |
| `target`               | yes      | Logical target (usually `module_path!()`).   |
| `file`                 | no       | Source file path (set by macros).            |
| `line`                 | no       | Source line number (set by macros).          |
| `timestamp_unix_nanos` | no       | Nanoseconds since Unix epoch.                |

#### `Metadata::new`

```rust
pub const fn new(level: Level, target: &'a str) -> Self;
```

Minimal constructor; all optional fields are `None`.

```rust
use log_io::{Level, Metadata};
let m = Metadata::new(Level::Info, "app");
```

#### `Metadata::with_location`

```rust
pub const fn with_location(self, file: &'a str, line: u32) -> Self;
```

Attach source location.

```rust
use log_io::{Level, Metadata};
let m = Metadata::new(Level::Info, "app").with_location("src/lib.rs", 42);
```

#### `Metadata::with_timestamp`

```rust
pub const fn with_timestamp(self, ts_unix_nanos: u128) -> Self;
```

Attach an explicit timestamp. The logger sets this automatically
unless `LoggerBuilder::no_timestamps` was called.

```rust
use log_io::{Level, Metadata};
let m = Metadata::new(Level::Info, "app").with_timestamp(1_700_000_000_000_000_000);
```

### `Record`

```rust
pub struct Record<'a> {
    pub metadata: Metadata<'a>,
    pub message: &'a str,
    pub fields: &'a [Field<'a>],
    pub context: &'a [Field<'a>],
}
```

The immutable unit dispatched through the pipeline. `context` holds
default fields and thread-local context; `fields` holds per-call
fields.

#### `Record::new`

```rust
pub const fn new(metadata: Metadata<'a>, message: &'a str, fields: &'a [Field<'a>]) -> Self;
```

Build a record with no context fields.

```rust
use log_io::{Field, Level, Metadata, Record};

let fields = [Field::new("port", 80_u32)];
let r = Record::new(Metadata::new(Level::Info, "app"), "started", &fields);
```

#### `Record::with_context`

```rust
pub const fn with_context(
    metadata: Metadata<'a>,
    message: &'a str,
    fields: &'a [Field<'a>],
    context: &'a [Field<'a>],
) -> Self;
```

Build a record with explicit context fields. The [`Logger`] uses
this form internally to splice in thread-local context.

#### `Record::all_fields`

```rust
pub fn all_fields(&self) -> impl Iterator<Item = &Field<'a>>;
```

Iterate over context fields followed by per-call fields. Most
formatters render in this order so context appears before the
call-site data.

```rust
use log_io::{Field, Level, Metadata, Record};

let ctx = [Field::new("trace_id", "abc")];
let fields = [Field::new("port", 80_u32)];
let r = Record::with_context(
    Metadata::new(Level::Info, "app"),
    "served",
    &fields,
    &ctx,
);
for f in r.all_fields() {
    println!("{} = {:?}", f.key, f.value);
}
```

## Filtering

### `Filter`

```rust
pub struct Filter { /* opaque */ }
```

A target-aware severity gate. Pairs a default level with zero or
more per-target overrides.

#### `Filter::new`

```rust
pub fn new(default_level: Level) -> Self;
```

Filter with the given default and no overrides.

```rust
use log_io::{Filter, Level};
let f = Filter::new(Level::Info);
```

#### `Filter::with_rule`

```rust
pub fn with_rule(self, target: impl Into<String>, level: Level) -> Self;
```

Add a per-target override. Rules are checked in insertion order;
the first matching prefix wins, so add the most specific prefix
first.

```rust
use log_io::{Filter, Level};
let f = Filter::new(Level::Warn)
    .with_rule("app::auth", Level::Debug)
    .with_rule("hyper", Level::Off);
```

#### `Filter::default_level`

```rust
pub fn default_level(&self) -> Level;
```

The configured default.

#### `Filter::rules`

```rust
pub fn rules(&self) -> &[FilterRule];
```

The configured rules in insertion order.

#### `Filter::is_enabled`

```rust
pub fn is_enabled(&self, target: &str, level: Level) -> bool;
```

`true` if a record at `level` targeted at `target` would be
admitted.

```rust
use log_io::{Filter, Level};
let f = Filter::new(Level::Warn).with_rule("app", Level::Debug);
assert!(f.is_enabled("app", Level::Debug));
assert!(!f.is_enabled("other", Level::Info));
```

#### `Filter::threshold_for`

```rust
pub fn threshold_for(&self, target: &str) -> Level;
```

The effective threshold the filter applies to `target`. Useful for
implementing call-site short-circuits.

#### `Filter::parse`

```rust
pub fn parse(directive: &str) -> Result<Self, ParseFilterError>;
```

Parse a directive of the form `<default>[, <target>=<level>]*`.
Matching is prefix-based on `::` / `.` module boundaries.

```rust
use log_io::{Filter, Level};
let f = Filter::parse("warn, app::auth=debug, hyper=off").unwrap();
assert_eq!(f.default_level(), Level::Warn);
assert!(f.is_enabled("app::auth::login", Level::Debug));
assert!(!f.is_enabled("hyper", Level::Error));
```

### `FilterRule`

```rust
pub struct FilterRule {
    pub target: String,
    pub level: Level,
}
```

One rule in a [`Filter`]. Constructed via [`FilterRule::new`] or
implicitly by [`Filter::with_rule`].

```rust
pub fn new(target: impl Into<String>, level: Level) -> Self;
```

### `ParseFilterError`

```rust
pub struct ParseFilterError;
```

Returned by [`Filter::parse`] when any segment is malformed.

## Formats

### `Format` trait

```rust
pub trait Format: Send + Sync {
    fn write_record<W: core::fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> core::fmt::Result;
}
```

The serialization stage of the pipeline. Implementors write to any
[`core::fmt::Write`] sink so the trait is usable in `no_std`.

The trait is implemented for `&T`, `Box<T>`, and `Arc<T>` where `T:
Format`.

**Implementing a custom format**

```rust
use core::fmt;
use log_io::{format::Format, Record};

struct CompactFormat;

impl Format for CompactFormat {
    fn write_record<W: fmt::Write + ?Sized>(
        &self,
        record: &Record<'_>,
        writer: &mut W,
    ) -> fmt::Result {
        write!(
            writer,
            "{}|{}|{}\n",
            record.metadata.level.as_str(),
            record.metadata.target,
            record.message,
        )
    }
}
```

### `JsonFormat`

```rust
pub struct JsonFormat { /* opaque */ }
```

Line-delimited JSON. Each record emits one JSON object terminated
by `\n`. Available behind `feature = "json"`.

Field order: `timestamp` (if set), `level`, `target`, `message`,
`file`, `line`, then context fields, then per-call fields.

#### `JsonFormat::new` / `JsonFormat::default`

```rust
pub const fn new() -> Self;
impl Default for JsonFormat;
```

Compact JSON, unix-nanosecond timestamps.

```rust
use log_io::format::JsonFormat;
let fmt = JsonFormat::new();
```

#### `JsonFormat::pretty`

```rust
pub const fn pretty(self, enabled: bool) -> Self;
```

Toggle indented multi-line output. Intended for hand inspection,
not for ingest pipelines.

```rust
use log_io::format::JsonFormat;
let fmt = JsonFormat::new().pretty(true);
```

#### `JsonFormat::timestamp_rfc3339`

```rust
pub const fn timestamp_rfc3339(self, enabled: bool) -> Self;
```

Emit timestamps as RFC 3339 strings (`"2026-05-18T13:24:01.123456789Z"`)
instead of Unix nanoseconds.

```rust
use log_io::format::JsonFormat;
let fmt = JsonFormat::new().timestamp_rfc3339(true);
```

### `LogfmtFormat`

```rust
pub struct LogfmtFormat { /* opaque */ }
```

`key=value key=value ...\n` lines. Values containing whitespace,
`=`, or `"` are double-quoted with escapes; bare alphanumerics are
emitted unquoted. Available behind `feature = "logfmt"`.

```rust
use log_io::format::LogfmtFormat;
let fmt = LogfmtFormat::new();
```

### `HumanFormat`

```rust
pub struct HumanFormat { /* opaque */ }
```

Single-line terminal-friendly output:

```
TIMESTAMP LEVEL [target] message key=value key=value
```

Levels are upper-case and column-padded. Timestamps are RFC 3339
when present. Available behind `feature = "human"`; works under
`no_std`.

#### `HumanFormat::new` / `HumanFormat::default`

```rust
pub const fn new() -> Self;
impl Default for HumanFormat;
```

Defaults: target shown, source location hidden.

#### `HumanFormat::show_target`

```rust
pub const fn show_target(self, enabled: bool) -> Self;
```

Toggle the `[target]` column.

```rust
use log_io::format::HumanFormat;
let fmt = HumanFormat::new().show_target(false);
```

#### `HumanFormat::show_source_location`

```rust
pub const fn show_source_location(self, enabled: bool) -> Self;
```

When enabled and the record carries source location, the line is
suffixed with ` (file:line)`.

```rust
use log_io::format::HumanFormat;
let fmt = HumanFormat::new().show_source_location(true);
```

## Sinks

### `Sink` trait

```rust
pub trait Sink: Send + Sync {
    fn write_record(&self, record: &Record<'_>) -> Result<()>;
    fn flush(&self) -> Result<()>;
}
```

Output stage of the pipeline. Must be safe to call concurrently.
The trait is implemented for `Arc<S>` and `Box<S>` where `S: Sink`.

**Implementing a custom sink**

```rust
use std::sync::Mutex;
use log_io::{Record, Result, Sink};

struct InMemorySink {
    lines: Mutex<Vec<String>>,
}

impl Sink for InMemorySink {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        self.lines.lock().unwrap().push(record.message.to_owned());
        Ok(())
    }
    fn flush(&self) -> Result<()> {
        Ok(())
    }
}
```

### `StdoutSink`

```rust
pub struct StdoutSink<F: Format> { /* opaque */ }

impl<F: Format> StdoutSink<F> {
    pub fn new(format: F) -> Self;
}
```

Writes serialized records to `io::stdout()` under a sink-local
mutex.

```rust
use log_io::format::JsonFormat;
use log_io::sink::StdoutSink;
let sink = StdoutSink::new(JsonFormat::new());
```

### `StderrSink`

```rust
pub struct StderrSink<F: Format> { /* opaque */ }

impl<F: Format> StderrSink<F> {
    pub fn new(format: F) -> Self;
}
```

Identical to [`StdoutSink`] but writes to standard error.

### `FileSink`

```rust
pub struct FileSink<F: Format> { /* opaque */ }

impl<F: Format> FileSink<F> {
    pub fn append<P: AsRef<Path>>(path: P, format: F) -> Result<Self>;
    pub fn create<P: AsRef<Path>>(path: P, format: F) -> Result<Self>;
}
```

`BufWriter<File>` wrapped in a mutex. `Sink::flush` flushes the
buffer to the OS but does not call `fsync`; use the file directly
if you need stricter durability.

`append` opens the file for append (creating if needed); `create`
truncates an existing file.

```rust
use log_io::format::JsonFormat;
use log_io::sink::FileSink;

let sink = FileSink::append("app.log", JsonFormat::new())?;
# Ok::<(), log_io::Error>(())
```

### `WriterSink`

```rust
pub struct WriterSink<W: Write + Send, F: Format> { /* opaque */ }

impl<W: Write + Send, F: Format> WriterSink<W, F> {
    pub fn new(writer: W, format: F) -> Self;
}
```

Adapter for any `Send + Write` value: in-memory buffers, sockets,
channel-backed writers, etc. Useful in tests for capturing output.

```rust
use log_io::format::JsonFormat;
use log_io::sink::WriterSink;
let buf: Vec<u8> = Vec::new();
let sink = WriterSink::new(buf, JsonFormat::new());
```

### `NullSink`

```rust
pub struct NullSink;

impl NullSink {
    pub const fn new() -> Self;
}
```

Discards every record. Used in benchmarks and tests.

## Errors

### `Error`

```rust
pub enum Error {
    Io(io::Error),
    Format(fmt::Error),
    Configuration(&'static str),
}
```

Errors produced by the logging pipeline.

`From<io::Error>` and `From<fmt::Error>` are implemented. `Display`
and `std::error::Error` are implemented; `Error::source` returns
the underlying error for `Io` and `Format`.

### `Result`

```rust
pub type Result<T> = core::result::Result<T, Error>;
```

Crate-wide result alias.

## Logger

### `Logger`

```rust
pub struct Logger { /* opaque, cheap to clone */ }
```

The configured pipeline. Cloning a `Logger` shares the underlying
filter and sinks via `Arc`.

#### `Logger::builder`

```rust
pub fn builder() -> LoggerBuilder;
```

Start configuring a logger. See [`LoggerBuilder`](#loggerbuilder).

#### `Logger::log`

```rust
pub fn log(&self, level: Level, message: &str, fields: &[Field<'_>]);
```

Emit a record using the empty string as the target. Errors are
silently swallowed; use [`Logger::try_log`] or
[`LoggerBuilder::on_error`] to surface them.

**Parameters**

| Name      | Type            | Description                                  |
|-----------|-----------------|----------------------------------------------|
| `level`   | `Level`         | Record severity.                             |
| `message` | `&str`          | Free-form message.                           |
| `fields`  | `&[Field<'_>]`  | Structured fields. May be empty.             |

```rust
use log_io::{Field, Level, Logger};
let logger = Logger::builder().level(Level::Info).stdout_human().build();
logger.log(Level::Info, "ok", &[Field::new("port", 80_u32)]);
```

#### `Logger::try_log`

```rust
pub fn try_log(
    &self,
    level: Level,
    target: &str,
    message: &str,
    fields: &[Field<'_>],
) -> Result<()>;
```

Emit a record with explicit `target`. The first sink error is
returned; every sink still receives the record.

```rust
use log_io::{Field, Level, Logger};
let logger = Logger::builder().level(Level::Info).stdout_json().build();
logger.try_log(Level::Info, "app::auth", "login ok", &[Field::new("user", 42_u64)])?;
# Ok::<(), log_io::Error>(())
```

#### `Logger::try_emit`

```rust
pub fn try_emit(
    &self,
    metadata: Metadata<'_>,
    message: &str,
    fields: &[Field<'_>],
) -> Result<()>;
```

Lowest-level entry point. Accepts a pre-built `Metadata` so callers
(typically the macros) can attach source location.

```rust
use log_io::{Level, Logger, Metadata};
let logger = Logger::builder().level(Level::Info).stdout_json().build();
let m = Metadata::new(Level::Info, "app").with_location(file!(), line!());
logger.try_emit(m, "manual", &[])?;
# Ok::<(), log_io::Error>(())
```

#### `Logger::enabled`

```rust
pub fn enabled(&self, target: &str, level: Level) -> bool;
```

Returns `true` if a record at `level`/`target` would pass the
filter. Use in hot paths to skip field construction.

```rust
use log_io::{Field, Level, Logger};
let logger = Logger::builder().level(Level::Info).stdout_json().build();
if logger.enabled("app", Level::Debug) {
    let computed = expensive_diagnostic();
    logger.log(Level::Debug, "report", &[Field::new("data", computed.as_str())]);
}
# fn expensive_diagnostic() -> String { String::new() }
```

#### `Logger::threshold_for`

```rust
pub fn threshold_for(&self, target: &str) -> Level;
```

The effective filter threshold for `target`.

#### `Logger::min_level`

```rust
pub fn min_level(&self) -> Level;
```

The lowest severity admitted by the filter anywhere. Useful for
macros to short-circuit at compile time or call site.

#### `Logger::sink_count`

```rust
pub fn sink_count(&self) -> usize;
```

Number of installed sinks. Diagnostic accessor.

#### `Logger::flush`

```rust
pub fn flush(&self) -> Result<()>;
```

Flush every installed sink. The first error is surfaced; every
sink is still attempted.

```rust
use log_io::{Level, Logger};
let logger = Logger::builder().level(Level::Info).stdout_json().build();
// ... log records ...
logger.flush()?;
# Ok::<(), log_io::Error>(())
```

### `LoggerBuilder`

```rust
pub struct LoggerBuilder { /* opaque */ }
```

Fluent builder. Construct with [`Logger::builder`]. Every method
returns `Self` so calls chain.

#### `LoggerBuilder::level`

```rust
pub fn level(self, level: Level) -> Self;
```

Set the default minimum severity. Existing rules are preserved.

```rust
use log_io::{Level, Logger};
let logger = Logger::builder().level(Level::Warn).stdout_json().build();
```

#### `LoggerBuilder::filter`

```rust
pub fn filter(self, filter: Filter) -> Self;
```

Replace the filter wholesale.

```rust
use log_io::{Filter, Level, Logger};
let f = Filter::new(Level::Info).with_rule("app::auth", Level::Debug);
let logger = Logger::builder().filter(f).stdout_json().build();
```

#### `LoggerBuilder::target_level`

```rust
pub fn target_level(self, target: impl Into<String>, level: Level) -> Self;
```

Add a per-target filter override.

```rust
use log_io::{Level, Logger};
let logger = Logger::builder()
    .level(Level::Warn)
    .target_level("app::auth", Level::Debug)
    .target_level("hyper", Level::Off)
    .stdout_json()
    .build();
```

#### `LoggerBuilder::filter_directive`

```rust
pub fn filter_directive(self, directive: &str) -> Self;
```

Parse `directive` and replace the filter. Silently retains the
previous filter on parse error; use [`Filter::parse`] up front for
explicit handling.

```rust
use log_io::Logger;
let logger = Logger::builder()
    .filter_directive("warn, app::auth=debug, hyper=off")
    .stdout_json()
    .build();
```

#### `LoggerBuilder::no_timestamps`

```rust
pub fn no_timestamps(self) -> Self;
```

Disable automatic timestamp capture. Records still carry whatever
timestamp the caller sets on `Metadata`.

#### `LoggerBuilder::no_context`

```rust
pub fn no_context(self) -> Self;
```

Disable thread-local context capture.

#### `LoggerBuilder::with_default_field`

```rust
pub fn with_default_field<'a, V: Into<Value<'a>>>(
    self,
    key: impl Into<String>,
    value: V,
) -> Self;
```

Attach a field that the logger emits on every record. Typical use:
service / version / region identifiers.

```rust
use log_io::Logger;
let logger = Logger::builder()
    .with_default_field("service", "billing")
    .with_default_field("version", env!("CARGO_PKG_VERSION"))
    .with_default_field("env", "production")
    .stdout_json()
    .build();
```

#### `LoggerBuilder::on_error`

```rust
pub fn on_error(self, handler: ErrorHandler) -> Self;
```

Install a callback invoked when any sink returns an error. See
[`ErrorHandler`](#errorhandler).

```rust
use std::sync::Arc;
use log_io::Logger;
let logger = Logger::builder()
    .on_error(Arc::new(|e| eprintln!("log_io sink error: {e}")))
    .stdout_json()
    .build();
```

#### `LoggerBuilder::with_sink`

```rust
pub fn with_sink(self, sink: impl Sink + 'static) -> Self;
```

Install a pre-built sink. Use this when the sink is fallible to
construct (typical for [`FileSink`]) or when implementing a custom
[`Sink`].

```rust
use log_io::format::JsonFormat;
use log_io::sink::FileSink;
use log_io::Logger;

let sink = FileSink::append("app.log", JsonFormat::new())?;
let logger = Logger::builder().with_sink(sink).build();
# Ok::<(), log_io::Error>(())
```

#### `LoggerBuilder::stdout` / `stderr` / `writer`

```rust
pub fn stdout<F: Format + Send + Sync + 'static>(self, format: F) -> Self;
pub fn stderr<F: Format + Send + Sync + 'static>(self, format: F) -> Self;
pub fn writer<W, F>(self, writer: W, format: F) -> Self
where
    W: Write + Send + 'static,
    F: Format + Send + Sync + 'static;
```

Convenience constructors that wrap the destination + format and
install the resulting sink. Each adds one sink to the fanout.

```rust
use log_io::format::JsonFormat;
use log_io::Logger;
let logger = Logger::builder().stdout(JsonFormat::new().pretty(true)).build();
```

```rust
use log_io::format::JsonFormat;
use log_io::Logger;
let buf: Vec<u8> = Vec::new();
let logger = Logger::builder().writer(buf, JsonFormat::new()).build();
```

#### Format shortcuts

```rust
// feature = "json"
pub fn stdout_json(self) -> Self;
pub fn stderr_json(self) -> Self;

// feature = "logfmt"
pub fn stdout_logfmt(self) -> Self;
pub fn stderr_logfmt(self) -> Self;

// feature = "human"
pub fn stdout_human(self) -> Self;
pub fn stderr_human(self) -> Self;
```

Shorter forms for the common case. Each installs a sink wired to
the default configuration of the named format.

```rust
use log_io::Logger;
let logger = Logger::builder().stdout_json().build();
```

#### `LoggerBuilder::build`

```rust
pub fn build(self) -> Logger;
```

Finalize and return a usable [`Logger`].

**Panics** if no sink has been installed; a logger with zero sinks
is almost certainly a programmer error.

```rust
use log_io::Logger;
let logger = Logger::builder()
    .stdout_json()
    .build();
```

### `ErrorHandler`

```rust
pub type ErrorHandler = Arc<dyn Fn(&Error) + Send + Sync>;
```

Callback installed by [`LoggerBuilder::on_error`]. Invoked once per
sink error.

```rust
use std::sync::Arc;
use log_io::{ErrorHandler, Logger};

let handler: ErrorHandler = Arc::new(|err| {
    eprintln!("log sink failure: {err}");
});
let logger = Logger::builder().on_error(handler).stdout_json().build();
```

## Context propagation

The `context` module provides per-thread ambient fields that are
automatically attached to every record emitted on the thread.

### Constants

```rust
pub const MAX_CONTEXT_SLOTS: usize = 16;
```

Upper bound on the number of fields stored per thread. When
exceeded, the oldest slot is evicted on insertion.

### `with_field`

```rust
pub fn with_field<'a, V: Into<Value<'a>>>(key: &str, value: V) -> ContextGuard;
```

Push a key/value pair onto the current thread's context. Returns a
guard; the value is removed when the guard drops.

```rust
use log_io::{context, Level, Logger, Value};

let logger = Logger::builder().level(Level::Info).stdout_json().build();
{
    let _g = context::with_field("user_id", Value::U64(42));
    log_io::info!(logger, "served"); // record carries user_id=42
}
```

### `with_trace_id` / `with_request_id`

```rust
pub fn with_trace_id(id: &str) -> ContextGuard;
pub fn with_request_id(id: &str) -> ContextGuard;
```

Convenience wrappers for the most common keys.

```rust
use log_io::{context, Level, Logger};

let logger = Logger::builder().level(Level::Info).stdout_json().build();
let _t = context::with_trace_id("tx-7f3a");
let _r = context::with_request_id("req-001");
log_io::info!(logger, "served"); // record carries both
```

### `with_snapshot`

```rust
pub fn with_snapshot<R>(f: impl FnOnce(&[Field<'_>]) -> R) -> R;
```

Run `f` with the current thread's context exposed as a borrowed
slice of [`Field`]. Zero-allocation; reentrancy-safe (a re-entered
call sees an empty slice rather than panicking).

```rust
use log_io::context;
context::with_snapshot(|fields| {
    for f in fields {
        println!("{} = {:?}", f.key, f.value);
    }
});
```

### `clear`

```rust
pub fn clear();
```

Remove every context entry on the current thread. Intended for
tests; production code should rely on `ContextGuard` drop.

### `len`

```rust
pub fn len() -> usize;
```

Number of context entries on the current thread.

### `ContextGuard`

```rust
pub struct ContextGuard { /* opaque */ }
```

RAII guard returned by `with_field` and friends. Removes the
associated slot on drop. Guards may be dropped in any order.

## Macros

Each macro takes a `Logger` expression and produces a log call.
Fields use a `key = expr` syntax; the expression must produce a
value convertible to [`Value`]. Source location (`file!()`,
`line!()`) and module path (`module_path!()`) are captured
automatically.

### `log_at!`

```rust
log_io::log_at!(logger, Level::Info, "msg" [, key = value]*);
```

General form. The other level macros forward to this one.

```rust
use log_io::{Level, Logger};
let logger = Logger::builder().level(Level::Trace).stdout_json().build();
log_io::log_at!(logger, Level::Info, "boot", port = 8080_u32);
```

### `trace!` / `debug!` / `info!` / `warn!` / `error!`

```rust
log_io::trace!(logger, "msg" [, key = value]*);
log_io::debug!(logger, "msg" [, key = value]*);
log_io::info!(logger,  "msg" [, key = value]*);
log_io::warn!(logger,  "msg" [, key = value]*);
log_io::error!(logger, "msg" [, key = value]*);
```

Level-specific shortcuts.

```rust
use log_io::{Level, Logger};
let logger = Logger::builder().level(Level::Trace).stdout_json().build();
log_io::info!(logger, "request handled", path = "/api", duration_ms = 12_u64);
log_io::warn!(logger, "slow query", query_ms = 1280_u64);
log_io::error!(logger, "shutdown", signal = "SIGTERM", graceful = true);
```

## `no_std` support

With `default-features = false`, `log-io` compiles in `no_std`
mode. The available surface is:

- `Level`, `ParseLevelError`
- `Value`, `Field`, `Metadata`, `Record`
- `Format` trait
- `JsonFormat`, `LogfmtFormat`, `HumanFormat` (each behind its
  feature flag)

Not available: `Logger`, sinks, `Filter`, context, std-backed
timestamps. Embedded callers typically wire a custom formatter into
a UART-backed `core::fmt::Write` destination and produce
timestamps themselves via `Metadata::with_timestamp`.

```toml
[dependencies]
log-io = { version = "1", default-features = false, features = ["json"] }
```

```rust,no_run
#![no_std]

use log_io::format::{Format, JsonFormat};
use log_io::{Field, Level, Metadata, Record};

# struct Uart;
# impl core::fmt::Write for Uart { fn write_str(&mut self, _: &str) -> core::fmt::Result { Ok(()) } }
# let mut uart = Uart;
# let now_ns = 0_u128;
let fmt = JsonFormat::new();
let fields = [Field::new("temp_c", 23.5_f64)];
let meta = Metadata::new(Level::Info, "sensor").with_timestamp(now_ns);
let record = Record::new(meta, "tick", &fields);
fmt.write_record(&record, &mut uart).unwrap();
```

## Copyright

Copyright (C) 2026 James Gober. Licensed under Apache-2.0.

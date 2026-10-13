# log-io API Reference

> Reference for log-io 2.0. Rustdoc on [docs.rs/log-io](https://docs.rs/log-io)
> is authoritative for signatures; this document explains how the parts
> fit together. Every `rust` code block here is compiled and run as a
> doctest (`cargo test --doc --all-features`).

## Table of contents

1. [Overview](#overview)
2. [Feature flags](#feature-flags)
3. [Data model](#data-model)
4. [Formats](#formats)
5. [Filtering](#filtering)
6. [Logger](#logger)
7. [Global logger](#global-logger)
8. [Durability](#durability)
9. [Sinks](#sinks)
10. [Context](#context)
11. [Macros](#macros)
12. [Integrations: `log` and `tracing`](#integrations-log-and-tracing)
13. [Errors](#errors)
14. [Threading, async and allocation](#threading-async-and-allocation)
15. [`no_std`](#no_std)

## Overview

```text
caller --> Logger --> Filter --> assemble --> Sink #0 --> Format --> destination
           (macros,    (target,    (timestamp,   Sink #1 ...
            log,        level)      defaults,
            tracing)                context,
                                    dedup, redact)
```

- The **logger** checks the filter, stamps the time, gathers ambient
  fields (default fields, thread-local context, and context supplied by
  an integration), removes duplicate keys, redacts configured keys, and
  passes one `Record` to each sink in turn.
- The **filter** decides by target and level. The most specific rule
  wins.
- Each **sink** formats the record and writes it. A failing sink does
  not stop the others.
- Records at or above the **flush level** (default `Error`) are flushed
  to every sink before the call returns.

## Feature flags

| Feature | Default | Enables |
|---|---|---|
| `std` | yes | `Logger`, sinks, context, timestamps, global logger, macros |
| `json` | yes | `format::JsonFormat` |
| `logfmt` | yes | `format::LogfmtFormat` |
| `human` | yes | `format::HumanFormat` (works without `std`) |
| `log` | no | `compat::LogBridge` (adds the `log` crate) |
| `tracing` | no | `compat::TracingLayer` (adds `tracing-core`, `tracing-subscriber`) |

Without `std` the crate is `no_std` and needs no allocator.

## Data model

All data-model types borrow; building a record never allocates.

### `Level`

`Trace < Debug < Info < Warn < Error < Off`. `Off` is a filter
threshold meaning "nothing"; records never carry it (a record at `Off`
is never admitted). The enum is exhaustive.

```rust
use log_io::Level;

assert!(Level::Warn > Level::Info);
assert!(Level::Error.is_enabled_at(Level::Warn));
assert_eq!(Level::Warn.as_str(), "warn");
assert_eq!(Level::Warn.as_str_upper(), "WARN");
assert_eq!("WARNING".parse::<Level>(), Ok(Level::Warn)); // also "err"
assert_eq!(Level::ALL.len(), 5); // Off excluded
```

### `Value`

```rust
use log_io::Value;

let n: Value<'_> = 42_u8.into();                      // U64(42)
let big: Value<'_> = u128::MAX.into();                // U128
let s = String::from("alice");
let v: Value<'_> = (&s).into();                       // Str("alice")
let bytes: Value<'_> = (&[0xde_u8, 0xad][..]).into(); // Bytes, written as "dead"
let addr = std::net::Ipv4Addr::LOCALHOST;
let lazy = Value::Display(&addr);                     // rendered only if written
let err = std::io::Error::other("disk full");
let chain = Value::Error(&err);                       // "disk full: <source>: ..."
assert_eq!(lazy.to_string(), "127.0.0.1");
assert_eq!(bytes.to_string(), "dead");
assert!(Value::Null.is_null());
assert_eq!(n.variant_name(), "u64");
# let _ = (big, v, chain);
```

`Value` is `#[non_exhaustive]`; match it with a wildcard arm. The lazy
variants (`Display`, `Debug`, `Error`) hold trait-object references, so
`Value` is not `Send`/`Sync`, and `PartialEq` compares them by identity.

How each format writes values:

| Variant | JSON | logfmt | human |
|---|---|---|---|
| `Null` | `null` | empty | `<null>` |
| `Bool`, integers | literal | literal | literal |
| `F64` finite | `{:?}` text, e.g. `1.0`, `1e300` | same | same |
| `F64` NaN / inf | `null` | `NaN` / `inf` / `-inf` | same as logfmt |
| `Str`, `Char` | escaped string | quoted when needed | quoted when needed |
| `Bytes` | lowercase hex string | lowercase hex | lowercase hex |
| `Display`, `Debug`, `Error` | escaped string | quoted when needed | quoted when needed |

### `Field`, `Metadata`, `Record`

```rust
use log_io::{Field, Level, Metadata, Record};

let meta = Metadata::new(Level::Info, "app::http")
    .with_location("src/http.rs", 42)
    .with_timestamp(1_700_000_000_000_000_000);
let fields = [Field::new("status", 200_u16)];
let ctx = [Field::new("trace_id", "abc")];
let record = Record::with_context(meta, "handled", &fields, &ctx);

assert_eq!(record.metadata.line, Some(42));
let keys: Vec<&str> = record.all_fields().map(|f| f.key).collect();
assert_eq!(keys, ["trace_id", "status"]); // context first
```

`Metadata` and `Record` are `#[non_exhaustive]`: construct them with
these functions; all fields stay readable.

## Formats

### The `Format` trait

```text
pub trait Format: Send + Sync {
    fn write_record(&self, record: &Record<'_>, writer: &mut dyn fmt::Write) -> fmt::Result;
    fn write_record_generic<W: fmt::Write>(&self, record: &Record<'_>, writer: &mut W)
        -> fmt::Result where Self: Sized { /* calls write_record */ }
}
```

`write_record` is object-safe, so `Box<dyn Format>`, `&dyn Format` and
`Arc<dyn Format>` work and a format can be chosen at run time. The
built-in sinks call `write_record_generic`, which the built-in formats
override with a monomorphized implementation (dynamic dispatch through
`&mut dyn fmt::Write` costs 40-60% more per record for them).

```rust
use core::fmt;
use log_io::format::{self, Format, JsonFormat};
use log_io::{Level, Logger, Metadata, Record};

struct Tsv;
impl Format for Tsv {
    fn write_record(&self, r: &Record<'_>, w: &mut dyn fmt::Write) -> fmt::Result {
        writeln!(w, "{}\t{}\t{}", r.metadata.level, r.metadata.target, r.message)
    }
}

let closure = format::from_fn(|r: &Record<'_>, w: &mut dyn fmt::Write| {
    writeln!(w, "{}", r.message)
});

let pick_json = true;
let chosen: Box<dyn Format> = if pick_json { Box::new(JsonFormat::new()) } else { Box::new(Tsv) };
let logger = Logger::builder().stdout(chosen).stderr(closure).build();
# let _ = logger;

let mut out = String::new();
Tsv.write_record(&Record::new(Metadata::new(Level::Info, "t"), "m", &[]), &mut out)?;
assert_eq!(out, "info\tt\tm\n");
# Ok::<(), fmt::Error>(())
```

### Output safety (all built-in formats)

- C0 controls, DEL, C1 controls, U+2028, U+2029 and the bidirectional
  embedding / override / isolate characters are always escaped. A
  record is exactly one line (except pretty JSON) and cannot carry
  terminal escape sequences.
- JSON and logfmt reserve `timestamp`, `level`, `target`, `message`,
  `file`, `line`. A user field with one of those names is written as
  `fields.<name>`.

### `JsonFormat`

One object per line: `timestamp`, `level`, `target`, `message`, `file`,
`line`, then context fields, then call fields.

```rust
use log_io::format::{Format, JsonFormat};
use log_io::{Field, Level, Metadata, Record};

let fields = [Field::new("ratio", 1.0), Field::new("message", "spoof")];
let rec = Record::new(Metadata::new(Level::Info, "app").with_timestamp(0), "up", &fields);

let mut out = String::new();
JsonFormat::new().write_record(&rec, &mut out)?;
assert_eq!(
    out,
    "{\"timestamp\":0,\"level\":\"info\",\"target\":\"app\",\"message\":\"up\",\
      \"ratio\":1.0,\"fields.message\":\"spoof\"}\n"
);

out.clear();
JsonFormat::new().timestamp_rfc3339(true).write_record(&rec, &mut out)?;
assert!(out.starts_with("{\"timestamp\":\"1970-01-01T00:00:00.000000000Z\""));
# Ok::<(), core::fmt::Error>(())
```

`pretty(true)` indents for reading by eye. Nanosecond integer timestamps
exceed 2^53; consumers that parse numbers as doubles should use
`timestamp_rfc3339(true)`. RFC 3339 output is clamped at
`9999-12-31T23:59:59.999999999Z`.

### `LogfmtFormat`

`key=value` pairs. Values that are empty or contain whitespace, `=`,
`"`, `\` or a special character are quoted with JSON-compatible escapes
(`\"`, `\\`, `\n`, `\u001b`). Keys are never quoted: whitespace, `=`,
`"` and special characters become `_`; an empty key becomes `_`.

```rust
use log_io::format::{Format, LogfmtFormat};
use log_io::{Field, Level, Metadata, Record};

let fields = [Field::new("path", "/a b"), Field::new("bad key", 1_u8)];
let rec = Record::new(Metadata::new(Level::Warn, "db"), "slow", &fields);
let mut out = String::new();
LogfmtFormat::new().write_record(&rec, &mut out)?;
assert_eq!(out, "level=warn target=db message=slow path=\"/a b\" bad_key=1\n");
# Ok::<(), core::fmt::Error>(())
```

`timestamp_rfc3339(true)` switches the timestamp from nanoseconds to
RFC 3339.

### `HumanFormat` and `ColorChoice`

```text
2026-10-13T19:03:23.894235900Z WARN  [app::db] slow query table=users ms=412
```

Options: `show_target(bool)` (the column is also omitted for an empty
target), `show_source_location(bool)` (appends ` (file:line)`),
`color(bool)`. Message and target are escaped but not quoted;
backslashes are left alone, so the format is for people, not parsers.

```rust
use log_io::format::{ColorChoice, Format, HumanFormat};
use log_io::{Level, Metadata, Record};

let rec = Record::new(Metadata::new(Level::Error, "app"), "boom\x1b[2J", &[]);
let mut out = String::new();
HumanFormat::new().write_record(&rec, &mut out)?;
assert_eq!(out, "ERROR [app] boom\\x1b[2J\n");

// Colour decisions: Auto honours NO_COLOR, CLICOLOR_FORCE and TERM=dumb.
let colored = HumanFormat::new().color(ColorChoice::Auto.enabled_for_stderr());
assert!(ColorChoice::Always.resolve(false));
assert!(!ColorChoice::Never.resolve(true));
# let _ = colored;
# Ok::<(), core::fmt::Error>(())
```

`LoggerBuilder::stdout_human` / `stderr_human` use `ColorChoice::Auto`
for their stream. On Windows, `Auto` colours only inside terminals known
to interpret ANSI sequences (Windows Terminal, ConEmu, or when `TERM` is
set, as in Git Bash), because enabling virtual-terminal mode in the
classic console requires `unsafe` FFI.

## Filtering

### Rules and matching

A `Filter` has a default level and rules `target -> level`. A rule for
`app::db` matches `app::db`, `app::db::pool` and `app::db.pool`, not
`app::dbx`. The **longest** matching rule wins, so order never matters.

```rust
use log_io::{Filter, Level};

let f = Filter::new(Level::Info)
    .with_rule("app::db", Level::Trace)
    .with_rule("app", Level::Warn);
assert!(f.is_enabled("app::db::pool", Level::Trace));
assert!(!f.is_enabled("app::http", Level::Info));
assert_eq!(f.threshold_for("other"), Level::Info);
assert_eq!(f.min_level(), Level::Trace);
assert_eq!(f.rules()[0].target, "app::db"); // most specific first
```

### Directives

`Filter::parse` (also `str::parse::<Filter>()`) reads the `RUST_LOG`
style: comma-separated segments, each a bare level (the default) or
`target=level`. Whitespace and empty segments are ignored, level names
are case-insensitive, and the last occurrence of a target wins. With no
bare level the default is `off`.

```rust
use log_io::{Filter, Level, ParseFilterErrorKind};

let f: Filter = "info, app::db=trace ,hyper=off".parse()?;
assert_eq!(f.threshold_for("hyper::client"), Level::Off);

for (bad, kind) in [
    ("", ParseFilterErrorKind::Empty),
    ("verbose", ParseFilterErrorKind::InvalidLevel),
    ("=debug", ParseFilterErrorKind::EmptyTarget),
    ("app::=debug", ParseFilterErrorKind::TrailingSeparator),
    ("my app=debug", ParseFilterErrorKind::InvalidTarget),
] {
    assert_eq!(Filter::parse(bad).unwrap_err().kind(), kind);
}
let err = Filter::parse("info,db=loud").unwrap_err();
assert_eq!(err.segment(), "db=loud");
# Ok::<(), log_io::ParseFilterError>(())
```

Unlike `env_logger`, a bare word must be a level (a misspelt level is
an error rather than a target), and `/regex` message filters are not
supported.

## Logger

### Building

| Builder method | Default | Effect |
|---|---|---|
| `level(Level)` | `Info` | default threshold (rules kept) |
| `filter(Filter)` / `target_level(t, l)` | | replace the filter / add a rule |
| `filter_directive(&str)` | | parse a directive; invalid ones are reported by `try_build` |
| `try_filter_directive(&str)` | | same, failing immediately |
| `env_filter(var)` | | directive from an environment variable; unset or blank keeps the current filter |
| `default_target(t)` | `""` | target used by `Logger::log` |
| `no_timestamps()` / `no_context()` | on / on | skip the clock / thread context |
| `with_default_field(k, v)` | | field on every record (same key replaces) |
| `flush_on(Level)` | `Error` | flush every sink after records at or above it; `Off` disables |
| `redact_key(k)` / `redact_common_secrets()` | none | replace values with `[REDACTED]` |
| `on_error(handler)` | none | callback for sink, flush and format errors |
| `with_sink`, `writer`, `stdout`, `stderr` | | install sinks |
| `stdout_json`, `stderr_logfmt`, `stdout_human`, ... | | shortcuts per built-in format |
| `build()` / `try_build()` | | finish (`build` panics without a sink) |

```rust
use std::sync::Arc;
use log_io::{BuildError, Level, Logger};

let logger = Logger::builder()
    .level(Level::Info)
    .env_filter("LOG_IO_EXAMPLE_UNSET_VAR")
    .with_default_field("service", "billing")
    .redact_common_secrets()
    .flush_on(Level::Warn)
    .on_error(Arc::new(|e| eprintln!("logging failed: {e}")))
    .stderr_logfmt()
    .try_build()?;
assert_eq!(logger.min_level(), Level::Info);

assert!(matches!(Logger::builder().try_build(), Err(BuildError::NoSink)));
# Ok::<(), BuildError>(())
```

### Emitting

```rust
use log_io::{Field, Level, Logger, Metadata};

let logger = Logger::builder().default_target("app").stdout_json().build();

logger.log(Level::Info, "uses the default target", &[]);
logger.try_log(Level::Warn, "app::db", "explicit target", &[Field::new("ms", 412_u64)])?;
let meta = Metadata::new(Level::Error, "app").with_location(file!(), line!());
logger.try_emit(meta, "with source location", &[])?;

if logger.enabled("app::db", Level::Debug) {
    // build expensive fields only when they will be used
}
logger.flush()?;
# Ok::<(), log_io::Error>(())
```

`log` never returns errors (they go to the handler). `try_log` and
`try_emit` return the first sink or flush error; every sink still
receives the record.

### Record assembly

Ambient fields are, in increasing precedence: default fields, thread
context, integration context (for example `tracing` span fields), and
finally the call-site fields. Each key appears once; the highest
precedence wins. Redaction applies to all of them and never evaluates a
lazy value it replaces.

```rust
use std::io::Write;
use std::sync::{Arc, Mutex};
use log_io::format::JsonFormat;
use log_io::{context, Field, Level, Logger};

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);
impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

let buf = Buf::default();
let logger = Logger::builder()
    .no_timestamps()
    .with_default_field("user", "default")
    .redact_key("password")
    .writer(buf.clone(), JsonFormat::new())
    .build();
let _ctx = context::with_field("user", "from-context");
logger.log(Level::Info, "login", &[Field::new("password", "hunter2")]);

let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
assert_eq!(
    text,
    "{\"level\":\"info\",\"target\":\"\",\"message\":\"login\",\
      \"user\":\"from-context\",\"password\":\"[REDACTED]\"}\n"
);
```

### Error handling

`ErrorHandler` is `Arc<dyn Fn(&Error) + Send + Sync>`. It runs once per
failing sink per record, and for flush and message-formatting errors.
If the handler logs through a logger whose sink keeps failing, nested
failures are not re-reported (no unbounded recursion).

## Global logger

```rust,no_run
use log_io::{Level, Logger};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let logger = Logger::builder().level(Level::Info).stdout_json().build();
    let _guard = log_io::init(logger)?;      // keep until the end of main
    log_io::global::install_panic_hook();    // optional

    log_io::info!("service started", port = 8080_u16);
    if let Some(logger) = log_io::global::get() {
        logger.log(Level::Debug, "direct access", &[]);
    }
    log_io::global::shutdown()?;             // e.g. before process::exit
    std::process::exit(0);
}
```

- `init(logger)` installs the logger and returns a `FlushGuard`;
  `set_global(logger)` installs without a guard. Both fail with
  `SetGlobalError` (which returns the logger) if one is installed.
- `global::get()`, `global::flush()`, `global::shutdown()`.
- `global::install_panic_hook()` logs each panic (target `panic`, the
  panic message, location, thread name) and flushes, then runs the
  previous hook. It does nothing when the panic happened inside a sink
  on the same thread, where touching the logger could deadlock.

## Durability

| Situation | Buffered records |
|---|---|
| record at or above the flush level (default `Error`) | flushed before the call returns, with everything buffered before it |
| `Logger::flush` / `shutdown`, `global::flush` / `shutdown` | flushed |
| `FlushGuard` dropped (end of `main`) | flushed, background sinks drained |
| last `Logger` clone dropped | flushed, background sinks drained |
| return from `main` with the logger in a `static` and no guard | **lost** |
| `std::process::exit` without `shutdown` | **lost** |
| `panic = "abort"` without the panic hook, `abort()`, fatal signal | **lost** |
| `panic = "abort"` with the panic hook | the panic record and everything before it are flushed |

Flushing hands bytes to the operating system; that survives a crash of
the process but not of the machine. `FileSink::builder(..).sync_on_flush(true)`
adds `fsync` (`sync_data`) to every flush, and `buffer_capacity(0)`
writes every record through immediately.

## Sinks

### `Sink` trait

```text
pub trait Sink: Send + Sync {
    fn write_record(&self, record: &Record<'_>) -> Result<()>;
    fn flush(&self) -> Result<()>;
    fn shutdown(&self) -> Result<()> { self.flush() }
}
```

`shutdown` is called by `Logger::shutdown`, guards, and the last logger
drop. A sink must keep accepting records afterwards. `Arc<S>` and
`Box<S>` forward all three methods.

### Built-in sinks

| Sink | Writes to | Notes |
|---|---|---|
| `StdoutSink<F>` / `StderrSink<F>` | standard streams | one `write_all` per record under the stream's own lock |
| `FileSink<F>` | a file | 8 KiB buffer by default; `builder` sets `append`, `buffer_capacity`, `sync_on_flush` |
| `WriterSink<W, F>` | any `io::Write + Send` | `with_writer` gives locked access; `into_inner` returns the writer |
| `RotatingFileSink<F>` | `WriterSink<RotatingFile, F>` | see below |
| `NonBlocking<F>` | any `io::Write + Send`, on a thread | see below |
| `NullSink` | nowhere | does not format |

Built-in sinks recover from mutex poisoning and format outside their
lock.

### `RotatingFile`

```rust,no_run
use log_io::format::JsonFormat;
use log_io::sink::{RotatingFile, Rotation};
use log_io::Logger;

let file = RotatingFile::builder("logs/app.jsonl")
    .max_bytes(50 * 1024 * 1024) // rotate before exceeding 50 MiB
    .rotation(Rotation::Daily)   // and at 00:00 UTC
    .max_files(14)               // keep app.jsonl.1 ..= app.jsonl.14
    .open()?;
let logger = Logger::builder().writer(file, JsonFormat::new()).build();
# let _ = logger;
# Ok::<(), std::io::Error>(())
```

Rotation renames `app.jsonl` to `app.jsonl.1` (shifting older files up)
between records, never inside one. The handle is closed before renaming,
which makes it work on Windows. If a rename fails (another process
holding the file open without delete sharing), the writer keeps
appending to the current file, retries later, and reports the error from
the next `flush`. Records are never lost to a failed rotation.
`Rotation::{Never, Minutely, Hourly, Daily}`; periods are UTC.

### `NonBlocking`

```rust
use log_io::format::JsonFormat;
use log_io::sink::{NonBlocking, Overflow};
use log_io::Logger;

let sink = NonBlocking::builder(std::io::sink(), JsonFormat::new())
    .capacity(8192)                 // queued records
    .overflow(Overflow::DropNewest) // or Overflow::Block (default, lossless)
    .buffer_capacity(64 * 1024)     // worker's write buffer
    .build()?;
let logger = Logger::builder().with_sink(sink).build();
log_io::info!(logger, "formatted here, written on the worker thread");
logger.shutdown()?; // drains the queue and joins the worker
# Ok::<(), log_io::Error>(())
```

- Records are formatted on the calling thread and written by a
  background thread, which flushes whenever its queue drains.
- `flush` waits until everything queued before it is written.
- After `shutdown`, records are written synchronously, so late records
  from other threads are kept.
- Worker write errors are counted (`write_errors()`), passed to the
  builder's `on_error` handler on the worker thread, and returned by the
  next `flush`. Drops under `DropNewest` are counted (`dropped()`).
- Queued strings are recycled, so a single producer does not allocate
  in steady state; under heavy contention some records allocate.

### Writing a sink

```rust
use std::sync::Mutex;
use log_io::format::{Format, LogfmtFormat};
use log_io::{Logger, Record, Result, Sink};

#[derive(Default)]
struct Lines(Mutex<Vec<String>>);

impl Sink for Lines {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        let mut line = String::new();
        LogfmtFormat::new().write_record_generic(record, &mut line)?;
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(line);
        Ok(())
    }
    fn flush(&self) -> Result<()> { Ok(()) }
}

let sink = std::sync::Arc::new(Lines::default());
let logger = Logger::builder().no_timestamps().with_sink(sink.clone()).build();
log_io::info!(logger, "captured");
assert_eq!(sink.0.lock().unwrap().len(), 1);
```

## Context

Thread-local key-value pairs attached to every record from the thread.

```rust
use log_io::{context, Value};

{
    let _trace = context::with_trace_id("tx-1");
    let _user = context::with_field("user", 42_u64);
    let _inner = context::with_field("user", 43_u64); // shadows the outer user
    assert_eq!(context::len(), 2);
    context::with_snapshot(|fields| {
        assert_eq!(fields[1].value, Value::U64(43));
    });
}
assert_eq!(context::len(), 0);
```

- `with_field`, `with_trace_id`, `with_request_id` return a
  `ContextGuard`; dropping it removes the entry. Guards may be dropped
  in any order. Pushing a value allocates; attaching context to a record
  does not.
- At most `MAX_CONTEXT_SLOTS` (16) entries per thread and fields per
  record; the oldest entries are evicted.
- Changes made from inside a sink or format apply when the current
  dispatch ends. Guards dropped during thread teardown do nothing.
- `ContextGuard` is `!Send` / `!Sync`, which turns "guard held across
  `.await` in a multi-threaded executor" into a compile error:

```compile_fail
let guard = log_io::context::with_field("tenant", "a");
std::thread::spawn(move || drop(guard));
```

### Snapshots for threads and tasks

```rust
use log_io::context::{self, ContextSnapshot};
use log_io::Field;

let snapshot = {
    let _req = context::with_request_id("req-7");
    context::capture() // owned, Send + Sync, cheap to clone
};

std::thread::spawn({
    let snapshot = snapshot.clone();
    move || snapshot.scope(|| assert_eq!(context::len(), 1))
})
.join()
.unwrap();

let _attached = snapshot.attach(); // until the guard drops
let explicit = ContextSnapshot::from_fields(&[Field::new("tenant", "acme")]);
let future = explicit.wrap(async { /* every poll sees tenant=acme */ });
# drop(future);
```

`ContextSnapshot::wrap` re-installs the snapshot around every poll, so
it works with any executor, including work-stealing ones. It boxes the
future once.

## Macros

```text
info!(LOGGER, [target: EXPR,] "format string" [, ARG]* [, KEY = VALUE]* [,])
info!([target: EXPR,] "format string" ...)        // global logger
info!(LOGGER, &str_expression [, KEY = VALUE]*)   // non-literal message
log_at!(LOGGER, LEVEL, ...)  /  log_at!(LEVEL, "...")
```

`trace!`, `debug!`, `info!`, `warn!`, `error!` and `log_at!` share the
syntax. Keys are identifiers, dotted paths, or string literals. Values
go through `Value::from`; `%v` records `v` via `Display` and `?v` via
`Debug`, lazily. The filter is checked once, before any argument is
evaluated; the target defaults to `module_path!()` and the source
location is always attached.

```rust
use log_io::{Level, Logger};

let logger = Logger::builder().level(Level::Debug).stdout_logfmt().build();
let user = "alice";
let peer = std::net::Ipv4Addr::LOCALHOST;
let roles = ["admin"];

log_io::info!(logger, "user {user} logged in from {}", peer, session = 7_u64);
log_io::debug!(logger, "request", http.method = "GET", "content-type" = "text/html",
    peer = %peer, roles = ?roles);
log_io::warn!(logger, target: "billing", "braces are escaped: {{}}");
log_io::log_at!(logger, Level::Error, "runtime-chosen level");
let owned = String::from("not a format string {}");
log_io::trace!(logger, owned.as_str()); // filtered out at Debug, never formatted
```

Named format arguments (`"{x}", x = 1`) are not available because
`name = value` always means a field; use inline captures or positional
arguments.

## Integrations: `log` and `tracing`

### `compat::LogBridge` (feature `log`)

```rust
use log_io::compat::LogBridge;
use log_io::{Level, Logger};

let logger = Logger::builder().level(Level::Info).stdout_logfmt().build();
let _guard = logger.flush_guard();
LogBridge::new(logger).install().ok(); // also sets log::max_level

log::info!(user = "alice", attempts = 3; "login");
```

Levels, target, file and line are preserved; key-values keep their
primitive types (strings, integers, floats, booleans, chars) and other
values are rendered with `Display` when written. No allocation for up
to 16 key-values.

### `compat::TracingLayer` (feature `tracing`)

```rust
use log_io::compat::TracingLayer;
use log_io::Logger;
use tracing_subscriber::layer::SubscriberExt;

let logger = Logger::builder().stdout_json().build();
let subscriber = tracing_subscriber::registry().with(TracingLayer::new(logger));
tracing::subscriber::with_default(subscriber, || {
    let span = tracing::info_span!("request", id = 7_u64);
    let _entered = span.enter();
    tracing::info!(status = 200_u16, "handled"); // carries id=7
});
```

The `message` field becomes the record message; other event fields
become fields; enclosing spans' fields (root first, inner shadowing
outer) become context. The logger's filter is applied per event; the
layer does not implement `Layer::enabled`, because returning `false`
there would also disable the event for other layers. Your application
needs `tracing-subscriber`'s `registry` feature.

## Errors

`Error` (`#[non_exhaustive]`): `Io(io::Error)`, `Format(fmt::Error)`,
`Configuration(&'static str)`, `Closed`, `Custom(Box<dyn Error + Send +
Sync>)` (build with `Error::custom`). `Result<T>` is
`core::result::Result<T, Error>`.

| Error | Returned by | Meaning |
|---|---|---|
| `Error` | `try_log`, `try_emit`, `flush`, `shutdown`, sinks | sink, flush or format failure |
| `BuildError` (`NoSink`, `InvalidFilter`) | `LoggerBuilder::try_build` | configuration problem |
| `ParseFilterError` (`kind()`, `segment()`) | `Filter::parse`, `try_filter_directive` | malformed directive |
| `ParseLevelError` | `Level::from_str` | unknown level name |
| `SetGlobalError` (`into_inner()`) | `init`, `set_global` | a global logger is already installed |

## Threading, async and allocation

- `Logger` and all sinks are `Send + Sync`; `Logger` is also
  `UnwindSafe` / `RefUnwindSafe` (its state survives panics).
- Synchronous sinks block the calling thread for the write. In async
  code, prefer `NonBlocking` for files and sockets.
- Context is per thread; use `ContextSnapshot::wrap` for futures.
- Steady-state emission through the synchronous sinks does not allocate
  (timestamps, context, defaults, dedup, redaction, `format!` messages,
  and the `log` / `tracing` bridges included) while each group of fields
  stays within 16 entries. `tests/alloc.rs` enforces this.
- Per-thread buffers are trimmed back to 64 KiB after an oversized
  record.

## `no_std`

With `default-features = false` and any of `json`, `logfmt`, `human`,
the data model and formatters work without `std` or `alloc`, writing to
any `core::fmt::Write`. Supply the timestamp yourself.

```rust
// Works unchanged in a no_std crate.
use core::fmt::Write;
use log_io::format::{Format, LogfmtFormat};
use log_io::{Field, Level, Metadata, Record};

fn emit(uart: &mut impl Write, now_ns: u128) -> core::fmt::Result {
    let fields = [Field::new("temp_c", 21_i8)];
    let meta = Metadata::new(Level::Info, "sensor").with_timestamp(now_ns);
    let record = Record::new(meta, "reading", &fields);
    LogfmtFormat::new().write_record_generic(&record, uart)
}

let mut out = String::new();
emit(&mut out, 0)?;
assert_eq!(out, "timestamp=0 level=info target=sensor message=reading temp_c=21
");
# Ok::<(), core::fmt::Error>(())
```

## Copyright

Copyright (C) 2026 James Gober. Licensed under the Apache License,
Version 2.0.

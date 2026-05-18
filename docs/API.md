# log-io - API Reference

> Offline-friendly companion to the rustdoc. The source of truth for
> signatures is the rustdoc; this document explains how the pieces fit
> together.

## Pipeline

A record flows through three stages:

```
caller -> Logger -> Filter -> [Sink #0 ... Sink #N]
                                  |
                                  v
                              Format -> Writer
```

- The **logger** captures wall-clock time (optional), splices in
  thread-local context (optional) and any default fields, and
  constructs the `Record`.
- The **filter** decides whether the record is admitted based on its
  target and level.
- Each **sink** receives the same `&Record`. Sinks run sequentially in
  insertion order. The first error is surfaced, but every sink still
  receives the record so a single failure does not starve the others.
- Each sink owns a **format** that writes the record's bytes to a
  destination.

## Choosing a format

| Format          | Use when                                                      |
|-----------------|---------------------------------------------------------------|
| `JsonFormat`    | Ingesting into a JSON-aware log store (Elastic, Loki, etc).   |
| `LogfmtFormat`  | grep-friendly `key=value` lines for filesystem or stdout.     |
| `HumanFormat`   | Local development; aligned columns, RFC 3339 timestamps.      |

`JsonFormat` supports `.pretty(true)` for indented output and
`.timestamp_rfc3339(true)` to emit ISO 8601 strings instead of
nanosecond integers.

## Choosing a sink

| Sink            | Notes                                                          |
|-----------------|----------------------------------------------------------------|
| `StdoutSink`    | Shared `Stdout` under a sink-local mutex.                      |
| `StderrSink`    | Same, but stderr.                                              |
| `FileSink`      | `BufWriter<File>`. `flush()` reaches the OS, not `fsync`.      |
| `WriterSink`    | Wrap any `Send + Write`. Tests use this for in-memory capture. |
| `NullSink`      | Discards. Useful for benches and tests.                        |

A single logger can carry many sinks. To fan out JSON to a file and
human-readable to stderr, install both via the builder.

## Building a logger

The shortest path uses format shortcuts:

```rust
let logger = log_io::Logger::builder()
    .level(log_io::Level::Info)
    .stdout_json()
    .build();
```

For explicit format configuration:

```rust
use log_io::format::JsonFormat;

let logger = log_io::Logger::builder()
    .level(log_io::Level::Info)
    .stdout(JsonFormat::new().pretty(true))
    .build();
```

For files (fallible to open):

```rust
use log_io::format::JsonFormat;
use log_io::sink::FileSink;

let sink = FileSink::create("app.log", JsonFormat::new())?;
let logger = log_io::Logger::builder()
    .level(log_io::Level::Info)
    .with_sink(sink)
    .build();
```

For multi-sink fan-out:

```rust
let logger = log_io::Logger::builder()
    .level(log_io::Level::Info)
    .stderr_human()
    .with_sink(file_sink)
    .build();
```

## Filter directives

`Filter::parse` accepts a directive string of the form

```
<default-level>?[, <target>=<level>]*
```

For example, `warn, hyper=off, app::auth=debug` admits warn-or-higher
globally, silences `hyper` entirely, and admits debug from
`app::auth` and its children. Matching is prefix-based on
`::` / `.` module boundaries: `app` matches `app` and `app::sub` but
not `application`.

## Default fields

Attach service-level identifiers once at builder time:

```rust
let logger = log_io::Logger::builder()
    .with_default_field("service", "billing")
    .with_default_field("version", env!("CARGO_PKG_VERSION"))
    .stdout_json()
    .build();
```

Every record carries these fields without the call site repeating
them.

## Context propagation

Context is per-thread:

```rust
let _t = log_io::context::with_trace_id("tx-7f3a");
let _r = log_io::context::with_request_id("req-001");
// downstream log calls automatically carry trace_id and request_id
```

For async tasks, snapshot context fields at spawn time and re-install
them on the destination thread. The crate is runtime-agnostic so this
is the caller's responsibility.

Context is reentrancy-safe: a sink that emits a log record from
inside its own `write_record` sees an empty context snapshot instead
of panicking.

## Macros

```rust
log_io::info!(logger, "served", path = "/api", duration_ms = 12_u64);
```

`module_path!()` becomes the record target, `file!()` and `line!()`
populate `Metadata::file` / `Metadata::line` so the human format's
`.show_source_location(true)` mode can render them.

## Error handling

By default `Logger::log` swallows sink errors. Two ways to surface
them:

- `Logger::try_log` returns `Result<()>` with the first sink error.
- `LoggerBuilder::on_error` installs a callback that fires for every
  failure across all sinks.

```rust
let logger = log_io::Logger::builder()
    .level(log_io::Level::Info)
    .stdout_json()
    .on_error(std::sync::Arc::new(|err| {
        eprintln!("log_io sink error: {err}");
    }))
    .build();
```

## Hardening

- All built-in sinks recover from mutex poisoning: a panic that
  briefly held a sink lock does not disable logging.
- `HumanFormat` escapes newlines / tabs / CRs in keys, values,
  messages, and targets so the line-per-record contract holds for
  any UTF-8 input.
- JSON output is fuzz-tested for shape invariants (balanced quotes,
  one record per line) and property-tested via `proptest`.

## Performance notes

The fast path is allocation-free in steady state. The format step
uses a thread-local `String` scratch buffer; only the first record
on a given thread (and ones that exceed the buffer's capacity)
trigger an allocation.

Filter decisions are essentially free (~1 ns). Put coarse-grained
filtering up front rather than guarding at the call site.

See [`BENCH.md`](../BENCH.md) for measured numbers.

## no_std support

With `default-features = false`, the crate compiles in `no_std`
mode. Available items: `Level`, `Value`, `Field`, `Metadata`,
`Record`, `Format` trait, and all three concrete formatters
(JSON / logfmt / human) writing into any `core::fmt::Write`. Not
available: `Logger`, sinks, context, std-backed timestamps. Embedded
callers typically wire a custom formatter into a UART-backed
`core::fmt::Write` and produce timestamps themselves via
`Metadata::with_timestamp`.

## Copyright

Copyright (C) 2026 James Gober. Licensed under Apache-2.0.

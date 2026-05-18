# log-io - API Reference

> Offline-friendly companion to the rustdoc. The source of truth for
> signatures is the rustdoc; this document explains how the pieces fit
> together.

## Pipeline

A record flows through three stages:

```
caller -> Logger -> Filter -> [Sink#0 .. Sink#N]
                                  |
                                  v
                              Format -> Writer
```

- The **logger** captures wall-clock time (optional), splices in
  thread-local context (optional), and constructs the `Record`.
- The **filter** decides whether the record is admitted based on its
  target and level.
- Each **sink** receives the same `&Record`. Sinks run sequentially in
  insertion order. If a sink returns an error the rest are skipped.
- Each sink owns a **format** that writes the record's bytes to a
  destination.

## Choosing a format

| Format          | Use when                                                      |
|-----------------|---------------------------------------------------------------|
| `JsonFormat`    | Ingesting into a JSON-aware log store (Elastic, Loki, etc).   |
| `LogfmtFormat`  | grep-friendly key=value lines for filesystem or stdout.       |
| `HumanFormat`   | Local development; aligned columns, RFC 3339 timestamps.      |

`JsonFormat` supports a `.pretty(true)` mode that emits indented
output. Use it only for hand inspection; line-oriented ingest pipelines
expect one record per line.

## Choosing a sink

| Sink            | Notes                                                          |
|-----------------|----------------------------------------------------------------|
| `StdoutSink`    | Shared `Stdout` under a sink-local mutex.                      |
| `StderrSink`    | Same, but stderr.                                              |
| `FileSink`      | `BufWriter<File>`. `flush()` reaches the OS, not `fsync`.      |
| `WriterSink`    | Wrap any `Send + Write`. Tests use this for in-memory capture. |
| `NullSink`      | Discards. Useful for benches and tests.                        |

A single logger can carry many sinks. To fan out JSON to a file and
human-readable to stderr, install two sinks via `add_sink`.

## Filter directives

`Filter::parse` accepts a directive string with the form

```
<default-level>?[, <target>=<level>]*
```

For example, `warn, hyper=off, app::auth=debug` admits warn-or-higher
globally, silences `hyper` entirely, and admits debug from
`app::auth` and its children. Matching is prefix-based on
`::` / `.` module boundaries: `app` matches `app` and `app::sub` but
not `application`.

## Context propagation

Context is per-thread. `context::with_field(key, value)` returns a
guard that pops the value when dropped. Inside a request handler:

```rust
let _t = log_io::context::with_trace_id("tx-7f3a");
let _r = log_io::context::with_request_id("req-001");
// downstream log calls automatically carry trace_id and request_id
```

For async tasks, snapshot the context fields at spawn time and
re-install them on the destination thread. The crate is runtime-
agnostic so this is the caller's responsibility.

## Macros

```rust
log_io::info!(logger, "served", path = "/api", duration_ms = 12_u64);
```

`module_path!()` is used as the record target, which lines up with
filter directives that match on the module path.

## Performance notes

The fast path is zero-allocation. `Record`, `Field`, `Value` all
borrow. Formatters write directly to a `core::fmt::Write` sink. The
sinks shipped here allocate one short `String` per record to buffer
the formatted output before taking their mutex; if that allocation
matters to your hot path, build a custom sink that uses a thread-local
buffer.

Filter decisions are cheap (a few comparisons). `try_log_with_target`
short-circuits before any other work when the record is filtered out.

## no_std support

With `default-features = false`, the crate compiles in `no_std` mode.
What you keep: `Level`, `Value`, `Field`, `Metadata`, `Record`,
`Format` trait and (optionally) `JsonFormat` / `LogfmtFormat` writing
into a `core::fmt::Write`. What you lose: the `Logger`, all sinks,
context, timestamps. Embedded callers typically wire a custom
formatter into a UART-backed `core::fmt::Write` and produce
timestamps themselves.

## Copyright

Copyright (C) 2026 James Gober. Licensed under Apache-2.0.

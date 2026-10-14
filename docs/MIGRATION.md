# Migrating from log-io 1.x to 2.0

log-io 2.0 fixes problems that could not be fixed compatibly in 1.x:
records lost at exit, context leaking between async tasks, terminal
injection through the human format, metadata spoofing through field
names, and first-match filter rules. This guide lists every change you
may need to act on, roughly in order of how likely it is to affect you.

Most applications that only build a `Logger`, call the macros, and use
the built-in formats and sinks need at most three edits:

1. escape literal braces in macro messages (`{` becomes `{{`);
2. keep the `FlushGuard` from `log_io::init` (or call
   `Logger::shutdown`) if your logger lives in a `static`;
3. add a wildcard arm wherever you `match` on `Value` or `Error`.

## Toolchain

- **MSRV is now Rust 1.85** (was 1.75). The crate uses edition 2024.

## Macros

- **The message is a format string.** `info!(logger, "user {id}")` now
  formats `id`, and a literal `{` or `}` must be written `{{` / `}}`.
  Positional arguments follow the message:
  `info!(logger, "took {} ms", elapsed, path = p)`.
- `name = value` after the message is still a field, so named format
  arguments (`"{x}", x = 1`) are not supported. Use an inline capture
  (`"{x}"` with a local `x`) or a positional argument.
- A non-literal message must be a `&str` expression and takes no format
  arguments: `info!(logger, msg.as_str(), k = v)`.
- The logger argument must dereference to `&Logger` (`Logger`,
  `&Logger`, `Arc<Logger>`, ...). In 1.x any type with an `enabled`
  method was accepted.
- New: omit the logger to use the global one (`info!("ready")`), pass
  `target: "..."`, use dotted or quoted keys (`http.status = 200`,
  `"content-type" = ct`), and record values lazily with `%` (Display)
  or `?` (Debug).

## Durability and shutdown

- **Records at `Level::Error` and above are flushed** to every sink
  before the logging call returns. Opt out with
  `LoggerBuilder::flush_on(Level::Off)`.
- **Dropping the last clone of a `Logger` shuts its sinks down** (flush,
  drain background queues). Errors go to the `on_error` handler.
- A logger in a `static` is never dropped. Use the new global logger and
  keep its guard:

  ```rust
  // 1.x
  static LOGGER: OnceLock<Logger> = OnceLock::new();
  LOGGER.set(Logger::builder().with_sink(file_sink).build()).ok();

  // 2.0
  let _guard = log_io::init(Logger::builder().with_sink(file_sink).build())?;
  log_io::info!("no logger argument needed");
  ```

  `Logger::flush_guard()` gives the same guard for loggers you manage
  yourself. Call `log_io::global::shutdown()` (or `Logger::shutdown`)
  before `std::process::exit`.

## Context

- **`ContextGuard` is `!Send` and `!Sync`.** Holding one across an
  `.await` in a future passed to a multi-threaded executor is now a
  compile error instead of a silent cross-request leak. Carry context
  into tasks with a snapshot:

  ```rust
  // 1.x (leaked the field into other requests on the same thread)
  let _g = context::with_request_id(id);
  do_work().await;

  // 2.0
  let snapshot = {
      let _g = context::with_request_id(id);
      context::capture()
  };
  tokio::spawn(snapshot.wrap(async move { do_work().await }));
  // or, for synchronous work on another thread:
  std::thread::spawn(move || snapshot.scope(|| work()));
  ```

- On executors that do not require `Send` (`join!` / `select!` within a
  task, `LocalSet`, `block_on`), the compiler cannot catch a guard held
  across an `.await`. Wrap such futures with `ContextSnapshot::wrap`
  (`ContextSnapshot::default().wrap(..)` if there is nothing to
  capture): a wrapped future keeps the context it holds to itself while
  it is suspended. An unwrapped one leaks it to its sibling futures.
- An inner `with_field` with the same key now **shadows** the outer one
  (the record carries the key once, with the inner value).
- `context::len()` counts the fields a record would carry, after
  shadowing.
- Pushing context or dropping a guard from inside a sink no longer
  panics or leaks; the change applies when the current dispatch ends.

## Output changes

If you parse log output, review these.

| Area | 1.x | 2.0 |
|---|---|---|
| Field named `level`, `message`, `timestamp`, `target`, `file` or `line` (JSON, logfmt) | written as-is, duplicating the real key | written as `fields.level`, ... |
| Same key in default fields, context and call fields | every copy written | one copy: call > context > default |
| Floats | `1`, 300-digit expansions | `1.0`, `1e300` (round-trippable `{:?}` form) |
| logfmt quoted escapes | `\x1b` | `\u001b` (JSON-compatible) |
| logfmt empty target / message | `target=` | `target=""` |
| logfmt / human keys with control characters, `=`, `"`, whitespace | partly raw | replaced with `_`; empty key becomes `_` |
| Human format control characters | only `\n`, `\r`, `\t` escaped | all C0, DEL, C1, U+2028/2029 and bidi controls escaped |
| Human format values with `=` or `\`, empty values | unquoted | quoted |
| Human format, empty target | `[] message` | `message` |
| Human format via `stdout_human` / `stderr_human` | never coloured | coloured on a terminal (honours `NO_COLOR`) |
| JSON strings | C0 escaped | also DEL, C1, U+2028/2029, bidi escaped as `\uXXXX` |
| RFC 3339 after year 9999 | wrapped / invalid | clamped to `9999-12-31T23:59:59.999999999Z` |
| Records at `Level::Off` | admitted by most filters | never admitted |

To keep the human format uncoloured, install it explicitly:
`.stdout(HumanFormat::new())`.

## Filtering

- **The most specific rule wins**, regardless of order (`RUST_LOG`
  semantics). In 1.x the first rule in insertion order won, so
  `info,app=warn,app::db=trace` silently ignored `app::db=trace`.
- `Filter::with_rule` for an existing target replaces it.
  `Filter::rules()` is ordered most specific first.
- `Filter::parse` rejects inputs it used to accept: an empty or blank
  directive, an empty target (`=debug`), a target ending in a separator
  (`app::=debug`), and a target containing whitespace.
- `ParseFilterError` is now a struct carrying `kind()` and `segment()`
  (it was a unit struct). Match on `kind()` instead of comparing with
  `ParseFilterError`.
- `LoggerBuilder::filter_directive` ignores blank input and records an
  invalid directive; `try_build` reports it as
  `BuildError::InvalidFilter`, `build` keeps the previous filter (as in
  1.x). Use `try_filter_directive` to fail immediately.

## Types and traits

- **`Format::write_record` takes `&mut dyn core::fmt::Write`** so the
  trait is object-safe (`Box<dyn Format>`, `&dyn Format`). Update custom
  formats:

  ```rust
  // 1.x
  fn write_record<W: fmt::Write + ?Sized>(&self, r: &Record<'_>, w: &mut W) -> fmt::Result

  // 2.0
  fn write_record(&self, r: &Record<'_>, w: &mut dyn fmt::Write) -> fmt::Result
  ```

  Calling `format.write_record(&record, &mut string)` still compiles.
  For hot paths, built-in sinks call the new provided method
  `write_record_generic`; override it if your format is performance
  critical. Closures work through `format::from_fn`.
- **`Value` is `#[non_exhaustive]`** and gained `I128`, `U128`, `Bytes`,
  `Display`, `Debug` and `Error` variants. Add a wildcard arm to matches.
  Because the lazy variants hold trait-object references, `Value`,
  `Field` and `Record` are no longer `Send`, `Sync`, `UnwindSafe` or
  `RefUnwindSafe` (the same as `log::Record` and `tracing::Event`).
  Build fields on the thread that logs them, and wrap closures that
  capture them in `std::panic::AssertUnwindSafe` if you pass them to
  `catch_unwind`. `PartialEq` compares the lazy variants by identity.
- **`Error` is `#[non_exhaustive]`** and gained `Closed` and `Custom`.
  Its `Display` text now says which stage failed
  (`log sink I/O error: ...`).
- **`Metadata` and `Record` are `#[non_exhaustive]`**: build them with
  `Metadata::new(...).with_location(...)` and `Record::new` /
  `Record::with_context`. Fields remain public for reading.
- `Sink` gained a provided `shutdown` method (default: `flush`).
  Background sinks override it.
- `StdoutSink` / `StderrSink` write under the standard stream's own lock
  instead of an extra mutex.
- `ErrorHandler` is now importable from the crate root
  (`log_io::ErrorHandler`), as the 1.x docs claimed.
- `LoggerBuilder::{stdout, stderr, writer}` require `F: Format +
  'static` (the redundant `Send + Sync` bounds are implied).
- `on_error` is called once per failing sink and also for flush errors
  (1.x called it once per record with the first error).

## New in 2.0 (no action needed)

`log_io::init` / `set_global` / `global`, `FlushGuard`,
`Logger::shutdown`, `LoggerBuilder::{try_build, flush_on, env_filter,
default_target, redact_key, redact_common_secrets}`, `FileSink::builder`
(buffer size, `sync_on_flush`), `sink::NonBlocking`,
`sink::RotatingFile`, `format::ColorChoice`, `format::from_fn`,
`context::capture` / `ContextSnapshot`, and the `log` and `tracing`
bridges in `log_io::compat` (features `log` and `tracing`). See
[`API.md`](API.md).

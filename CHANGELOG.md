# Changelog

All notable changes to this project are documented here. The format is
based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.0] - 2026-05-18

### Added

- Public API: `Level`, `Value`, `Field`, `Metadata`, `Record`.
- Filter with per-target overrides and a `parse` directive form
  (`info,app::auth=debug,hyper=off`).
- Three output formats behind feature flags: JSON
  (`feature = "json"`), logfmt (`feature = "logfmt"`), human-readable
  (`feature = "human"`).
- Sink trait and four built-in sinks: `StdoutSink`, `StderrSink`,
  `FileSink` (truncating or appending), `WriterSink` (any
  `Send + Write`), `NullSink`.
- Thread-local context propagation: `with_field`, `with_trace_id`,
  `with_request_id`, `with_snapshot`, RAII guard.
- `Logger` and `LoggerBuilder` with fluent format and sink selection,
  plus `add_sink` for multi-sink fanout.
- Convenience macros: `trace!`, `debug!`, `info!`, `warn_!`, `error!`,
  `log_at!`.
- Hand-rolled RFC 3339 timestamp formatter (used by JSON and human
  formats; no dependencies).
- Integration tests covering JSON, logfmt, human formats, filter
  directives, context propagation, custom sinks, macros, unicode
  fields, and concurrent writers.
- Benches: `format` (formatter micro-bench) and `pipeline` (end-to-end
  log-call cost).
- Examples: `basic`, `json`, `context`, `filter`, `file_sink`.
- `no_std` build mode (`default-features = false`) covering the data
  model and the two trivial formatters.

### Changed

- `Level` discriminants renumbered: `Trace=0, Debug=1, Info=2, Warn=3,
  Error=4, Off=255`. `Off` is now strictly greater than all real
  severities, which is the form filter thresholds expect.

## [0.1.0] - 2026-05-12

### Added

- Initial repository scaffold.
- Apache-2.0 license, README, REPS specification stub, CI workflow,
  `.dev/` planning structure (DIRECTIVES, ROADMAP, PROMPTS).
- Crate name reserved on crates.io.

[Unreleased]: https://github.com/jamesgober/log-io/compare/v0.9.0...HEAD
[0.9.0]: https://github.com/jamesgober/log-io/releases/tag/v0.9.0
[0.1.0]: https://github.com/jamesgober/log-io/releases/tag/v0.1.0

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

To be finalized as the design matures. See `.dev/ROADMAP.md` for the
milestone plan. This file becomes authoritative once the first
non-placeholder release ships.

## 4. Public API

To be specified.

## 5. Safety contract

Every `unsafe` block (if any) must carry a `// SAFETY:` comment per the
project's `DIRECTIVES.md`.

## 6. MSRV policy

Pinned at 1.75. Bumps require a minor version increment and a
CHANGELOG entry under `### Changed` with rationale.

## 7. Performance contract

To be specified once benches exist.

## 8. Stability guarantees

`0.x.y` releases are not API-stable. Stability begins at `1.0.0`.

## 9. Dependency policy

Zero runtime dependencies preferred. Any added dependency requires a
documented justification in `.dev/AUDIT.md` or its successor.

## 10. Testing requirements

To be specified.

## 11. Documentation requirements

Every public item must have a rustdoc block once the API stabilizes.

## 12. Out of scope

To be specified.

# log-io fuzz targets

Fuzzing harnesses for `log-io`. Requires `cargo-fuzz` and a nightly
toolchain (`libfuzzer-sys` needs nightly sanitizer support).

## Install

```
cargo install cargo-fuzz
```

## Targets

| Target             | Input                         | Invariants                                                                 |
|--------------------|-------------------------------|----------------------------------------------------------------------------|
| `json_format`      | arbitrary `Record`            | one safe line; parses with `serde_json`; level, target, message round-trip |
| `logfmt_format`    | arbitrary `Record`            | one safe line; parses with a strict logfmt parser; strings round-trip      |
| `human_format`     | arbitrary `Record`            | one line with no raw control, C1, separator or bidi characters             |
| `logger_pipeline`  | defaults, context, log calls  | valid JSON per call; metadata cannot be spoofed; redaction never leaks     |
| `filter_directive` | directive and target strings  | never panics; parsed filters are internally consistent                     |
| `level_parse`      | arbitrary string              | never panics; accepts exactly the documented spellings                     |

"Arbitrary `Record`" covers every `Value` variant (including floats such
as NaN and huge `u128` timestamps that exercise RFC 3339 clamping),
context fields, and source locations. `src/lib.rs` holds the shared
generator and the reference logfmt parser.

## Run

```
cargo +nightly fuzz run json_format
cargo +nightly fuzz run logfmt_format -- -max_total_time=300
```

CI runs every target for 60 seconds on each push (see
`.github/workflows/ci.yml`). Crash inputs land under
`fuzz/artifacts/<target>/`; seed corpora can be added under
`fuzz/corpus/<target>/`.

This crate is excluded from the main workspace and never published.

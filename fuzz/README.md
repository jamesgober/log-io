# log-io fuzz targets

Fuzzing harnesses for `log-io`. Requires `cargo-fuzz` and a nightly
Rust toolchain (`libfuzzer-sys` does not build on stable).

## Install

```
cargo install cargo-fuzz
```

## Targets

| Target              | Coverage                                       |
|---------------------|------------------------------------------------|
| `filter_directive`  | `Filter::parse` over arbitrary UTF-8           |
| `json_escape`       | `JsonFormat::write_record` shape invariants    |
| `logfmt_format`     | `LogfmtFormat::write_record` shape invariants  |
| `level_parse`       | `Level::from_str` over arbitrary UTF-8         |

## Run

```
cargo +nightly fuzz run filter_directive
cargo +nightly fuzz run json_escape
cargo +nightly fuzz run logfmt_format
cargo +nightly fuzz run level_parse
```

Crash inputs land under `fuzz/artifacts/<target>/`. Seed corpora can
be added under `fuzz/corpus/<target>/` for guided fuzzing.

This crate is excluded from the main workspace and not published.

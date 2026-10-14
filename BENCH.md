# log-io Benchmarks

> Numbers are indicative, not contractual. They depend on the CPU, the
> OS scheduler and background load. Re-run them on your hardware with
> `cargo bench --all-features`.

## Machine and toolchain

- **CPU:** AMD Ryzen 9 9950X3D (16 cores / 32 threads), 128 GB RAM
- **OS:** Windows 11 Pro (10.0.26200), x86_64-pc-windows-msvc
- **Toolchain:** rustc 1.95.0, release profile with `lto = "thin"`
- **Date:** 2026-10-13
- **Load:** the machine was shared with other build jobs while these
  numbers were taken (17-53% CPU load). Treat single-digit percentage
  differences as noise.

## Method

Two independent measurements.

1. **Criterion** (`benches/*.rs`, criterion 0.7): 100 samples per case
   after a 3-second warm-up. The tables report the median estimate.
   These are the absolute numbers for 2.0.0.
2. **Interleaved A/B against 1.0.0**: one probe program, compiled once
   against log-io 1.0.0 and once against 2.0.0, run alternately 8 times
   (each case: 15 timed batches of 100,000 calls after 50,000 warm-up
   calls, on a non-main thread). The tables report the minimum per case
   across runs. Running both versions alternately in the same window is
   the only fair comparison on a shared machine; the minimum filters out
   time slices lost to other processes.

All writer cases write into a discarding `io::Write`, so they measure
the pipeline, not disk or terminal throughput.

## Formatters (one record, 5 fields, into a reused `String`)

| Case | 1.0.0 (A/B min) | 2.0.0 (A/B min) | Change | 2.0.0 criterion median |
|---|---:|---:|---:|---:|
| JSON | 133.2 ns | 123.0 ns | -7.7% | 125.9 ns |
| logfmt | 156.7 ns | 115.2 ns | -26.5% | 118.7 ns |
| human | 199.2 ns | 196.6 ns | -1.3% | 218.6 ns |
| JSON, no fields | 12.5 ns | 12.2 ns | -2.4% | 12.9 ns |
| JSON through `&mut dyn fmt::Write` | | | | 200.6 ns |

2.0 escapes more characters (C1 controls, U+2028/U+2029, bidi
controls) yet is faster: escaping is now a 256-entry table lookup per
byte that copies clean runs in one write, and JSON metadata
punctuation is written as fixed strings. The last row shows the cost of
calling a format through the object-safe `write_record` instead of the
statically dispatched `write_record_generic` the sinks use.

## Pipeline (`Logger::log`, 3 fields unless noted)

| Case | 1.0.0 (A/B min) | 2.0.0 (A/B min) | Change | 2.0.0 criterion median |
|---|---:|---:|---:|---:|
| filtered out (below threshold) | 2.0 ns | 0.8 ns | -60% | 0.76 ns |
| `NullSink` (no formatting) | 7.3 ns | 8.9 ns | +1.6 ns | 9.0 ns |
| JSON writer | 63.2 ns | 63.8 ns | +0.9% | 66.8 ns |
| logfmt writer | 65.4 ns | 58.3 ns | -10.9% | 75.8 ns |
| JSON writer, no fields | 26.4 ns | 31.6 ns | +5.2 ns | 33.4 ns |
| 2 default fields | 94.2 ns | 91.6 ns | -2.8% | 114.5 ns |
| context capture, empty | 77.2 ns | 76.4 ns | -1.0% | 90.3 ns |
| context capture, 2 fields | 105.8 ns | 101.2 ns | -4.3% | 131.8 ns |
| default configuration (timestamps + context) | 113.3 ns | 114.1 ns | +0.7% | 135.9 ns |
| `info!` macro, default configuration | 138.9 ns | 137.5 ns | -1.0% | 144.7 ns |

- Filtered-out records are rejected by comparing against the filter's
  precomputed minimum level before any rule is scanned.
- 2.0 does more per record (key de-duplication, the error-handler
  reentrancy guard, flush-on-level, dispatch-depth tracking for the
  panic hook). That shows as a fixed 1.5-5 ns on the cheapest paths;
  on realistic records the faster formatters absorb it.
- The default configuration's largest cost is the wall-clock read
  (`SystemTime::now`), roughly 35-40 ns here.

## Concurrent throughput (one shared logger, JSON writer, 3 fields)

| Threads | 1.0.0 (A/B min) | 2.0.0 (A/B min) | Change |
|---:|---:|---:|---:|
| 1 | 62.9 ns/record | 64.4 ns/record | +2.4% |
| 2 | 45.7 ns/record | 40.0 ns/record | -12.5% |
| 4 | 32.7 ns/record | 24.4 ns/record | -25.4% |
| 8 | 71.4 ns/record | 26.8 ns/record | -62.5% |

Criterion (`benches/concurrent.rs`, which also pays for spawning the
threads in every sample, 20,000 records per thread) measured 12.7, 18.0,
25.0, 13.3 and 8.1 M records/s for 1, 2, 4, 8 and 16 threads.

The large multi-thread gain comes from keeping each sink's mutex on its
own cache lines (`CachePadded` in `src/sink.rs`). In 1.0 the mutex
could share a line with data that every logging thread reads, so each
lock and unlock stalled all threads. Beyond roughly 8 threads one
shared sink mutex still serializes writes; use `NonBlocking` (one
writer thread) or one sink per thread group for more.

## Re-audit fixes (2026-10-14)

Interleaved A/B on the same machine and toolchain family (rustc 1.99.0,
thin LTO): one probe compiled against `0139f53` (2.0.0 as first
audited) and against the fixed tree, run alternately 8 times, minimum
of 60 batches of 20,000 calls per case. Same 3 call-site fields as the
pipeline table.

| Case | before | after | Change |
|---|---:|---:|---:|
| `NullSink` (no formatting) | 7.4 ns | 10.1 ns | +2.7 ns |
| `NullSink`, 8 fields | 7.5 ns | 15.3 ns | +7.8 ns |
| JSON writer | 60.3 ns | 62.0 ns | +2.8% |
| JSON writer, no fields | 29.5 ns | 29.3 ns | -0.6% |
| logfmt writer | 51.8 ns | 55.3 ns | +6.7% |
| 2 default fields | 88.4 ns | 89.0 ns | +0.6% |
| context capture, 2 fields | 97.4 ns | 101.8 ns | +4.6% |
| default configuration | 109.2 ns | 110.2 ns | +0.9% |
| `info!` macro, default configuration | 134.6 ns | 133.3 ns | -1.0% |
| `Error` record, `NonBlocking` + `DropNewest` | 960 ns | 151 ns | -84% |

- **Flush-level records under `DropNewest`** (LOG-R03) no longer wait
  for the worker. Behind a writer that takes 5 ms per write with a
  64-record queue, one `error!` took 341-347 ms before and 1-3 µs
  after (5 runs each). The criterion case
  `pipeline/non_blocking_error_drop_newest` tracks the hand-off.
- **Call-site key de-duplication** (LOG-R06) costs about 1 ns per
  call-site field: one pass over the keys that hashes the last two
  bytes of each into a 64-bit set, with a full comparison only on a
  hash collision. That exceeds the 5% gate on the two cheapest paths
  (`NullSink`, logfmt). Accepted: writing a key twice produces JSON
  whose meaning depends on the parser (RFC 8259 leaves duplicate names
  undefined), and on realistic configurations the cost is within noise.

## Allocation

`tests/alloc.rs` counts allocations per thread with a counting global
allocator and asserts zero allocations in steady state for: the default
configuration (timestamps + context) with JSON, logfmt and human
output; default fields with context and RFC 3339 timestamps; key
collisions with redaction; the macros, including `format!`-style
messages; filtered-out records; the `log` bridge; and `tracing` events
inside a span.

## Reproduce

```text
cargo bench --all-features --bench format
cargo bench --all-features --bench pipeline
cargo bench --all-features --bench concurrent
cargo test  --all-features --test alloc
```

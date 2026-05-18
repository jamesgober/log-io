# log-io Benchmarks

> Numbers below are indicative, not contractual. Rerun with
> `cargo bench` on your target hardware. Hardware: developer laptop
> (Windows, x86_64, release build with `lto = "thin"`).

## Methodology

All benches are hand-rolled (no Criterion) to keep the crate
dependency-free. Each timing reports the mean over 200,000 iterations
after a 1,000-iteration warm-up.

* `format` bench: serializes one record into a reusable `String` buffer.
  Measures formatter cost alone.
* `pipeline` bench: full `Logger::log` call. Includes filter check,
  thread-local context snapshot, dispatch, format, and write.
* `concurrent` bench: N threads logging into a shared logger backed
  by a discarding writer.

## Format-only (ns / record, 5 fields)

| Format | ns/record |
|--------|----------:|
| JSON   | 131       |
| logfmt | 159       |
| human  | 210       |
| JSON, no fields | 12 |

JSON is fastest because of the batched escaper: clean ASCII runs are
written as single chunks. logfmt's per-key whitespace replacement and
human's level-column alignment add overhead.

## Full pipeline (ns / call, with 3 fields)

| Path                                    | ns/call |
|-----------------------------------------|--------:|
| `dispatch_only` (null sink)             | 6       |
| `below_threshold` (filtered out)        | 1       |
| `json + writer` (full pipeline)         | 59      |
| `logfmt + writer` (full pipeline)       | 63      |
| `json + writer` (no fields)             | 25      |
| with 2 default fields                   | 92      |
| with empty context snapshot             | 74      |

* The 1 ns filtered-out path means filtering is essentially free -
  put coarse target-based filtering up front rather than guarding at
  the call site.
* Default fields cost roughly 30 ns per record per two fields, because
  each default field is materialized from owned storage into a
  borrowed `Field`.
* The thread-local format buffer (introduced in 0.10.0) eliminates the
  per-record `String::with_capacity` allocation; the steady-state hot
  path now allocates only when the buffer is too small to hold the
  formatted output.

## Concurrent throughput (records / second)

| Threads | Total records | Wall time | Throughput |
|--------:|--------------:|----------:|-----------:|
| 1       | 100 K         | 6 ms      | ~15 M rec/s |
| 2       | 200 K         | 8 ms      | ~22 M rec/s |
| 4       | 400 K         | 14 ms     | ~28 M rec/s |
| 8       | 800 K         | 82 ms     | ~10 M rec/s |
| 16      | 1.6 M         | 211 ms    | ~7.6 M rec/s |

Single-thread throughput is ~15 M records / sec. The pipeline scales
roughly linearly up to about 4 threads on this hardware; beyond that
mutex contention on the writer's lock pulls throughput down.

A user who needs more scale than the shared-mutex sinks provide can
implement a sharded sink (one writer per thread, periodic flushing)
without leaving the `Sink` trait. That work is outside this crate.

## How to reproduce

```
cargo bench --bench format       # formatter cost
cargo bench --bench pipeline     # full Logger::log cost
cargo bench --bench concurrent   # multi-thread throughput
```

## What's not measured

* IO cost. `WriterSink<DevNull>` short-circuits the bytes. Real-world
  numbers depend on disk / socket throughput, not the formatter.
* Allocation pressure outside the format buffer. Records carry
  borrowed slices; nothing on the record path allocates.
* `cargo bench` against rotating file output. If you build a rotating
  sink (the crate doesn't ship one), benchmark with realistic record
  sizes and disk latency on the target system.

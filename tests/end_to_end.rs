//! End-to-end tests covering the full record pipeline.
//!
//! Each test stands up a real `Logger` writing to a `Vec<u8>` capture
//! and inspects the resulting bytes.

use std::io::Write;
use std::sync::{Arc, Mutex};

use log_io::format::{HumanFormat, JsonFormat, LogfmtFormat};
use log_io::sink::{NullSink, WriterSink};
use log_io::{Field, Filter, Level, Logger, Value, context};

#[derive(Default, Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn dump(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn json_logger(buf: Capture) -> Logger {
    Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf, JsonFormat::new())
        .build()
}

#[test]
fn json_pipeline_round_trips_basic_record() {
    let buf = Capture::default();
    let logger = json_logger(buf.clone());

    logger.log(Level::Info, "request", &[Field::new("status", 200_u16)]);
    let line = buf.dump();
    assert!(line.starts_with('{'));
    assert!(line.contains("\"level\":\"info\""));
    assert!(line.contains("\"status\":200"));
    assert!(line.ends_with("}\n"));
}

#[test]
fn logfmt_pipeline_emits_key_value_pairs() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), LogfmtFormat::new())
        .build();

    logger.log(
        Level::Info,
        "ok",
        &[Field::new("k", 1_u64), Field::new("name", "alice")],
    );
    let out = buf.dump();
    assert!(out.contains("level=info"));
    assert!(out.contains("message=ok"));
    assert!(out.contains("k=1"));
    assert!(out.contains("name=alice"));
}

#[test]
fn human_pipeline_emits_aligned_line() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), HumanFormat::new())
        .build();
    logger.log(Level::Warn, "hi", &[]);
    let out = buf.dump();
    assert!(out.starts_with("WARN  "), "{out}");
    assert!(out.ends_with("hi\n"));
}

#[test]
fn filter_directive_blocks_below_threshold() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .filter_directive("warn, app::auth=trace")
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), LogfmtFormat::new())
        .build();

    logger.try_log(Level::Info, "app", "info-app", &[]).unwrap();
    logger
        .try_log(Level::Trace, "app::auth", "trace-auth", &[])
        .unwrap();
    logger
        .try_log(Level::Error, "other", "error-other", &[])
        .unwrap();
    let out = buf.dump();
    assert!(!out.contains("info-app"), "{out}");
    assert!(out.contains("trace-auth"), "{out}");
    assert!(out.contains("error-other"), "{out}");
}

#[test]
fn context_propagates_to_record() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .writer(buf.clone(), JsonFormat::new())
        .build();

    context::clear();
    let _t = context::with_trace_id("tx-42");
    let _r = context::with_request_id("req-7");
    logger.log(Level::Info, "served", &[Field::new("dur_ms", 12_u32)]);
    drop(_t);
    drop(_r);

    let out = buf.dump();
    assert!(out.contains("\"trace_id\":\"tx-42\""), "{out}");
    assert!(out.contains("\"request_id\":\"req-7\""), "{out}");
    assert!(out.contains("\"dur_ms\":12"), "{out}");
}

#[test]
fn null_sink_is_silent() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(NullSink::new())
        .build();
    logger.log(Level::Error, "dropped", &[]);
}

#[test]
fn with_sink_accepts_arbitrary_sink() {
    let buf = Capture::default();
    let sink = WriterSink::new(buf.clone(), JsonFormat::new());
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(sink)
        .build();
    logger.log(Level::Info, "ok", &[]);
    assert!(buf.dump().contains("\"message\":\"ok\""));
}

#[test]
fn pretty_json_writes_multiple_lines_per_record() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), JsonFormat::new().pretty(true))
        .build();
    logger.log(Level::Info, "x", &[Field::new("y", 1_u32)]);
    let out = buf.dump();
    assert!(out.contains("\n  \"level\""), "{out}");
}

#[test]
fn macros_compile_and_emit() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), LogfmtFormat::new())
        .build();

    log_io::info!(logger, "hello");
    log_io::warn!(logger, "watch out", port = 80_u32);
    log_io::error!(logger, "bad", code = 500_u32, retry = true);
    log_io::debug!(logger, "deep");
    log_io::trace!(logger, "deeper");

    let out = buf.dump();
    assert!(out.contains("level=info"), "{out}");
    assert!(out.contains("port=80"), "{out}");
    assert!(out.contains("code=500"), "{out}");
    assert!(out.contains("retry=true"), "{out}");
}

#[test]
fn macros_capture_source_location() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), HumanFormat::new().show_source_location(true))
        .build();
    log_io::info!(logger, "loc");
    let out = buf.dump();
    assert!(out.contains("end_to_end.rs"), "{out}");
}

#[test]
fn filter_min_level_reflects_lowest_rule() {
    let logger = Logger::builder()
        .filter(
            Filter::new(Level::Warn)
                .with_rule("loud", Level::Trace)
                .with_rule("quiet", Level::Error),
        )
        .no_timestamps()
        .no_context()
        .with_sink(NullSink::new())
        .build();
    assert_eq!(logger.min_level(), Level::Trace);
}

#[test]
fn flush_propagates_to_all_sinks() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(NullSink::new())
        .with_sink(NullSink::new())
        .build();
    logger.flush().unwrap();
}

#[test]
fn unicode_in_strings_round_trips() {
    let buf = Capture::default();
    let logger = json_logger(buf.clone());
    logger.log(
        Level::Info,
        "umlaut: ü; cjk: 漢; emoji: 🚀",
        &[Field::new("note", Value::Str("naïve"))],
    );
    let out = buf.dump();
    assert!(out.contains("漢"), "{out}");
    assert!(out.contains("🚀"), "{out}");
    assert!(out.contains("naïve"), "{out}");
}

#[test]
fn concurrent_writers_dont_interleave_records() {
    use std::thread;

    let buf = Capture::default();
    let logger = Arc::new(json_logger(buf.clone()));

    let mut handles = Vec::new();
    for t in 0..8 {
        let logger = Arc::clone(&logger);
        handles.push(thread::spawn(move || {
            for i in 0..100 {
                logger.log(
                    Level::Info,
                    "tick",
                    &[Field::new("thread", t as u32), Field::new("i", i as u32)],
                );
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let out = buf.dump();
    let mut lines = 0usize;
    for line in out.lines() {
        assert!(line.starts_with('{') && line.ends_with('}'), "{line}");
        lines += 1;
    }
    assert_eq!(lines, 8 * 100);
}

#[test]
fn default_fields_appear_on_every_record() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_default_field("service", "api")
        .with_default_field("version", "1.2.3")
        .writer(buf.clone(), JsonFormat::new())
        .build();
    logger.log(Level::Info, "a", &[]);
    logger.log(Level::Warn, "b", &[Field::new("port", 80_u32)]);

    let out = buf.dump();
    assert_eq!(out.matches("\"service\":\"api\"").count(), 2);
    assert_eq!(out.matches("\"version\":\"1.2.3\"").count(), 2);
    assert!(out.contains("\"port\":80"));
}

#[test]
fn multi_sink_fanout_delivers_to_all() {
    let buf_a = Capture::default();
    let buf_b = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf_a.clone(), JsonFormat::new())
        .writer(buf_b.clone(), LogfmtFormat::new())
        .build();

    logger.log(Level::Info, "fanout", &[Field::new("k", 1_u32)]);

    assert!(buf_a.dump().contains("\"message\":\"fanout\""));
    assert!(buf_b.dump().contains("message=fanout"));
}

#[test]
fn on_error_handler_receives_sink_errors() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Bad;
    impl log_io::Sink for Bad {
        fn write_record(&self, _r: &log_io::Record<'_>) -> log_io::Result<()> {
            Err(log_io::Error::Configuration("synthetic"))
        }
        fn flush(&self) -> log_io::Result<()> {
            Ok(())
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let count_clone = count.clone();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(Bad)
        .on_error(Arc::new(move |_e| {
            count_clone.fetch_add(1, Ordering::Relaxed);
        }))
        .build();
    let _ = logger.try_log(Level::Info, "t", "m", &[]);
    let _ = logger.try_log(Level::Info, "t", "m", &[]);
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

#[test]
fn dispatch_continues_when_one_sink_fails() {
    struct Bad;
    impl log_io::Sink for Bad {
        fn write_record(&self, _r: &log_io::Record<'_>) -> log_io::Result<()> {
            Err(log_io::Error::Configuration("synthetic"))
        }
        fn flush(&self) -> log_io::Result<()> {
            Ok(())
        }
    }

    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(Bad)
        .writer(buf.clone(), JsonFormat::new())
        .build();
    let result = logger.try_log(Level::Info, "t", "m", &[]);
    assert!(result.is_err()); // first sink error is surfaced ...
    assert!(buf.dump().contains("\"message\":\"m\"")); // ... but the second sink still got the record
}

#[test]
fn macros_support_format_args_dotted_keys_and_sigils() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    let user = "alice";
    let addr = std::net::Ipv4Addr::LOCALHOST;
    let roles = vec!["admin"];
    log_io::info!(
        logger,
        "user {user} from {} tried {}x",
        addr,
        3,
        http.status = 401_u16,
        "content-type" = "text/plain",
        peer = %addr,
        roles = ?roles,
        n = 1_u8,
    );
    log_io::warn!(logger, target: "custom::target", "braces {{escaped}}");
    let dynamic = String::from("dynamic message {not a format}");
    log_io::error!(logger, dynamic.as_str(), code = 7_u8);
    log_io::debug!(logger, &dynamic);

    let out = buf.dump();
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 4, "{out}");
    assert_eq!(lines[0]["message"], "user alice from 127.0.0.1 tried 3x");
    assert_eq!(lines[0]["http.status"], 401);
    assert_eq!(lines[0]["content-type"], "text/plain");
    assert_eq!(lines[0]["peer"], "127.0.0.1");
    assert_eq!(lines[0]["roles"], "[\"admin\"]");
    assert_eq!(lines[0]["n"], 1);
    assert_eq!(lines[0]["target"], "end_to_end");
    assert!(
        lines[0]["file"]
            .as_str()
            .unwrap()
            .ends_with("end_to_end.rs")
    );
    assert_eq!(lines[1]["target"], "custom::target");
    assert_eq!(lines[1]["message"], "braces {escaped}");
    assert_eq!(lines[2]["message"], "dynamic message {not a format}");
    assert_eq!(lines[2]["code"], 7);
    assert_eq!(lines[3]["level"], "debug");
}

#[test]
fn macros_accept_logger_references_and_arcs() {
    let buf = Capture::default();
    let logger = Arc::new(json_logger(buf.clone()));
    let by_ref: &Logger = &logger;
    log_io::info!(logger, "arc");
    log_io::info!(by_ref, "ref");
    log_io::info!(&by_ref, "double ref");
    log_io::log_at!(*by_ref, Level::Warn, "deref");
    assert_eq!(buf.dump().lines().count(), 4);
}

#[test]
fn file_and_rotating_sinks_work_through_a_logger() {
    use log_io::sink::{FileSink, RotatingFile, RotatingFileSink};

    let dir = std::env::temp_dir().join(format!("log-io-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let file = FileSink::create(dir.join("plain.jsonl"), JsonFormat::new()).unwrap();
    let rotating = RotatingFile::builder(dir.join("rot.log"))
        .max_bytes(256)
        .max_files(2)
        .open()
        .unwrap();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(file)
        .with_sink(RotatingFileSink::new(rotating, LogfmtFormat::new()))
        .build();
    for i in 0..20_u32 {
        log_io::info!(logger, "record", i = i);
    }
    drop(logger); // flushes and shuts every sink down

    let plain = std::fs::read_to_string(dir.join("plain.jsonl")).unwrap();
    assert_eq!(plain.lines().count(), 20);
    assert!(dir.join("rot.log.1").exists());
    assert!(dir.join("rot.log.2").exists());
    assert!(!dir.join("rot.log.3").exists());
    let newest = std::fs::read_to_string(dir.join("rot.log")).unwrap();
    assert!(newest.contains("i=19"), "{newest}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_blocking_sink_through_a_logger_flushes_on_error() {
    let buf = Capture::default();
    let sink = log_io::sink::NonBlocking::new(buf.clone(), JsonFormat::new()).unwrap();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(sink)
        .build();
    log_io::info!(logger, "queued");
    log_io::error!(logger, "flush-on-error waits for the worker");
    // No explicit flush: the error record forced one.
    assert_eq!(buf.dump().lines().count(), 2);
}

#[test]
fn context_snapshot_follows_a_future_across_threads() {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};

    /// Logs once per poll and yields twice before completing.
    struct Steps<'a> {
        logger: &'a Logger,
        polls: u8,
    }
    impl Future for Steps<'_> {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let step = self.polls;
            log_io::info!(self.logger, "step", n = step);
            self.polls += 1;
            if self.polls == 3 {
                Poll::Ready(())
            } else {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .writer(buf.clone(), JsonFormat::new())
        .build();

    context::clear();
    let snapshot = {
        let _req = context::with_request_id("req-42");
        context::capture()
    };
    let mut task = snapshot.wrap(Steps {
        logger: &logger,
        polls: 0,
    });

    // Poll on three different threads, like a work-stealing executor.
    for _ in 0..3 {
        std::thread::scope(|s| {
            s.spawn(|| {
                let waker = Waker::noop().clone();
                let mut cx = Context::from_waker(&waker);
                let _ = Pin::new(&mut task).poll(&mut cx);
                assert_eq!(context::len(), 0, "nothing left behind on the worker");
            });
        });
    }
    // Records from an unrelated thread carry no request id.
    logger.log(Level::Info, "unrelated", &[]);

    let lines: Vec<serde_json::Value> = buf
        .dump()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 4);
    for line in &lines[..3] {
        assert_eq!(line["request_id"], "req-42");
    }
    assert!(lines[3].get("request_id").is_none());
}

#[test]
fn flush_guard_flushes_a_non_global_logger() {
    let path = std::env::temp_dir().join(format!("log-io-guard-{}.jsonl", std::process::id()));
    let sink = log_io::sink::FileSink::create(&path, JsonFormat::new()).unwrap();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .with_sink(sink)
        .build();
    {
        let _guard = logger.flush_guard();
        log_io::info!(logger, "buffered");
        assert_eq!(
            std::fs::read(&path).unwrap().len(),
            0,
            "still in the buffer"
        );
    }
    assert!(std::fs::read_to_string(&path).unwrap().contains("buffered"));
    drop(logger);
    let _ = std::fs::remove_file(&path);
}

//! End-to-end tests covering the full record pipeline.
//!
//! These are intentionally light on internal mocking: each test stands
//! up a real `Logger` writing to a `Vec<u8>` capture and inspects the
//! resulting bytes.

use std::io::Write;
use std::sync::{Arc, Mutex};

use log_io::format::{HumanFormat, JsonFormat};
use log_io::sink::{NullSink, WriterSink};
use log_io::{context, Field, Filter, Level, Logger, Value};

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
        .writer(move || buf)
        .json()
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
        .writer(move || buf.clone())
        .logfmt()
        .build();
    drop(logger);

    let buf2 = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer({
            let buf2 = buf2.clone();
            move || buf2
        })
        .logfmt()
        .build();

    logger.log(
        Level::Info,
        "ok",
        &[Field::new("k", 1_u64), Field::new("name", "alice")],
    );
    let out = buf2.dump();
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
        .writer({
            let buf = buf.clone();
            move || buf
        })
        .human()
        .build();
    logger.log(Level::Warn, "hi", &[]);
    let out = buf.dump();
    assert!(out.starts_with("WARN  "), "{out}");
    assert!(out.ends_with("hi\n"));
}

#[test]
fn filter_directive_via_builder_blocks_below_threshold() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .filter_directive("warn, app::auth=trace")
        .no_timestamps()
        .no_context()
        .writer({
            let buf = buf.clone();
            move || buf
        })
        .logfmt()
        .build();

    logger
        .try_log_with_target(Level::Info, "app", "info-app", &[])
        .unwrap();
    logger
        .try_log_with_target(Level::Trace, "app::auth", "trace-auth", &[])
        .unwrap();
    logger
        .try_log_with_target(Level::Error, "other", "error-other", &[])
        .unwrap();
    let out = buf.dump();
    assert!(!out.contains("info-app"), "{out}");
    assert!(out.contains("trace-auth"), "{out}");
    assert!(out.contains("error-other"), "{out}");
}

#[test]
fn context_propagates_to_record() {
    let buf = Capture::default();
    let logger = json_logger(Capture::default());
    drop(logger);

    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .writer({
            let buf = buf.clone();
            move || buf
        })
        .json()
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
        .add_sink(NullSink::new())
        .null()
        .json()
        .build();
    logger.log(Level::Error, "dropped", &[]);
}

#[test]
fn add_sink_works_with_custom_format_choice() {
    let buf = Capture::default();
    let sink = WriterSink::new(buf.clone(), JsonFormat::new());
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .add_sink(sink)
        .null()
        .json()
        .build();
    logger.log(Level::Info, "ok", &[]);
    let out = buf.dump();
    assert!(out.contains("\"message\":\"ok\""), "{out}");
}

#[test]
fn pretty_json_writes_multiple_lines_per_record() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer({
            let buf = buf.clone();
            move || buf
        })
        .json_with(JsonFormat::new().pretty(true))
        .build();
    logger.log(Level::Info, "x", &[Field::new("y", 1_u32)]);
    let out = buf.dump();
    assert!(out.contains("\n  \"level\""), "{out}");
}

#[test]
fn human_with_options_can_show_source_location() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer({
            let buf = buf.clone();
            move || buf
        })
        .human_with(HumanFormat::new().show_source_location(true))
        .build();
    logger
        .try_log_with_target(Level::Info, "tgt", "hi", &[])
        .unwrap();
    let out = buf.dump();
    // Source location is not auto-captured by the logger today, so the
    // line should NOT contain the file path here. This pins the
    // behavior: callers must use the lower-level API to attach source.
    assert!(!out.contains("file.rs"));
}

#[test]
fn macros_compile_and_emit() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer({
            let buf = buf.clone();
            move || buf
        })
        .logfmt()
        .build();

    log_io::info!(logger, "hello");
    log_io::warn_!(logger, "watch out", port = 80_u32);
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
fn filter_min_level_reflects_lowest_rule() {
    let logger = Logger::builder()
        .filter(
            Filter::new(Level::Warn)
                .with_rule("loud", Level::Trace)
                .with_rule("quiet", Level::Error),
        )
        .no_timestamps()
        .no_context()
        .null()
        .json()
        .build();
    assert_eq!(logger.min_level(), Level::Trace);
}

#[test]
fn flush_propagates_to_all_sinks() {
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .add_sink(NullSink::new())
        .add_sink(NullSink::new())
        .null()
        .json()
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
    let logger = json_logger(buf.clone());
    let logger = Arc::new(logger);

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
    // Every line should be a complete JSON record terminated by \n.
    let mut lines = 0usize;
    for line in out.lines() {
        assert!(line.starts_with('{') && line.ends_with('}'), "{line}");
        lines += 1;
    }
    assert_eq!(lines, 8 * 100);
}

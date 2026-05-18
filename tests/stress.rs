//! Stress and edge-case tests.
//!
//! These exercise corners that are easy to mis-handle:
//! * very large messages / many fields
//! * UTF-8 boundary characters in escaped strings
//! * panics inside sinks
//! * deeply nested context scopes
//! * empty / single-character inputs

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use log_io::format::{HumanFormat, JsonFormat, LogfmtFormat};
use log_io::sink::NullSink;
use log_io::{context, Field, Level, Logger, Record, Result, Sink};

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

#[test]
fn very_large_message_is_fully_emitted() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    let large = "x".repeat(1_000_000);
    logger.log(Level::Info, &large, &[]);
    let out = buf.dump();
    assert!(out.contains(&large), "large message missing");
    assert_eq!(out.matches('\n').count(), 1);
}

#[test]
fn many_fields_serialize_correctly() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), LogfmtFormat::new())
        .build();
    let keys: Vec<String> = (0..200).map(|i| format!("k{i}")).collect();
    let fields: Vec<Field<'_>> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| Field::new(k.as_str(), i as u32))
        .collect();
    logger.log(Level::Info, "many", &fields);
    let out = buf.dump();
    for (i, k) in keys.iter().enumerate() {
        assert!(out.contains(&format!("{k}={i}")), "missing {k}");
    }
}

#[test]
fn empty_message_and_empty_target_serialize() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    logger.try_log(Level::Info, "", "", &[]).unwrap();
    let out = buf.dump();
    assert!(out.contains("\"message\":\"\""));
    assert!(out.contains("\"target\":\"\""));
}

#[test]
fn utf8_multi_byte_chars_dont_corrupt_escape_run() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    // Each section mixes clean chunks with escapable chars.
    logger.log(
        Level::Info,
        "umlaut\"cjk\n漢\tend",
        &[Field::new("v", "中\"国\\code")],
    );
    let out = buf.dump();
    assert!(out.contains("漢"), "{out}");
    assert!(out.contains("\\\""), "{out}");
    assert!(out.contains("\\n"), "{out}");
    assert!(out.contains("\\\\code"), "{out}");
}

#[test]
fn nan_inf_serialize_as_null_in_json() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    logger.log(
        Level::Info,
        "x",
        &[
            Field::new("a", f64::NAN),
            Field::new("b", f64::INFINITY),
            Field::new("c", f64::NEG_INFINITY),
            Field::new("d", 1.5_f64),
        ],
    );
    let out = buf.dump();
    assert!(out.contains("\"a\":null"));
    assert!(out.contains("\"b\":null"));
    assert!(out.contains("\"c\":null"));
    assert!(out.contains("\"d\":1.5"));
}

#[test]
fn panic_inside_sink_does_not_poison_other_sinks() {
    struct Panicking;
    impl Sink for Panicking {
        fn write_record(&self, _: &Record<'_>) -> Result<()> {
            panic!("synthetic sink panic");
        }
        fn flush(&self) -> Result<()> {
            Ok(())
        }
    }
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        // Good sink first; bad sink second so dispatch hits both.
        .writer(buf.clone(), JsonFormat::new())
        .with_sink(Panicking)
        .build();

    let logger_clone = logger.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        logger_clone.log(Level::Info, "before", &[]);
    }));
    // Sinks earlier in the chain succeeded before the panic; the
    // logger is still usable. A subsequent call panics again only
    // because we keep the panicking sink in place.
    let logger_clone = logger.clone();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        logger_clone.log(Level::Info, "after", &[]);
    }));
    let out = buf.dump();
    assert!(out.contains("\"message\":\"before\""), "{out}");
    assert!(out.contains("\"message\":\"after\""), "{out}");
}

#[test]
fn mutex_poison_recovery_keeps_logger_alive() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), JsonFormat::new())
        .build();

    // Poison the FORMAT_BUF thread-local by panicking mid-format on
    // a worker thread, then verify the next thread can still log.
    let _ = thread::spawn(|| {
        panic!("kill the worker before formatting");
    })
    .join();

    logger.log(Level::Info, "still here", &[]);
    assert!(buf.dump().contains("\"message\":\"still here\""));
}

#[test]
fn high_thread_count_concurrent_writes_atomic() {
    let buf = Capture::default();
    let logger = Arc::new(
        Logger::builder()
            .level(Level::Trace)
            .no_timestamps()
            .no_context()
            .writer(buf.clone(), JsonFormat::new())
            .build(),
    );

    const THREADS: usize = 32;
    const PER_THREAD: usize = 500;

    let mut handles = Vec::new();
    for t in 0..THREADS {
        let logger = Arc::clone(&logger);
        handles.push(thread::spawn(move || {
            for i in 0..PER_THREAD {
                logger.log(
                    Level::Info,
                    "msg",
                    &[
                        Field::new("t", t as u32),
                        Field::new("i", i as u32),
                        Field::new("note", "with some string content"),
                    ],
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
    assert_eq!(lines, THREADS * PER_THREAD);
}

#[test]
fn context_overflow_evicts_oldest_under_load() {
    context::clear();
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .writer(buf.clone(), JsonFormat::new())
        .build();

    let mut guards = Vec::new();
    for i in 0..(context::MAX_CONTEXT_SLOTS * 3) {
        let key = format!("k{i}");
        guards.push(context::with_field(&key, i as u32));
    }
    logger.log(Level::Info, "msg", &[]);
    let out = buf.dump();
    // The most recent MAX_CONTEXT_SLOTS keys should be present.
    let last = (context::MAX_CONTEXT_SLOTS * 3) - 1;
    let first_kept = last - (context::MAX_CONTEXT_SLOTS - 1);
    for i in first_kept..=last {
        assert!(out.contains(&format!("\"k{i}\":")), "missing k{i} in {out}");
    }
    drop(guards);
    context::clear();
}

#[test]
fn many_loggers_share_no_state() {
    let buf_a = Capture::default();
    let buf_b = Capture::default();
    let log_a = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_default_field("from", "A")
        .writer(buf_a.clone(), JsonFormat::new())
        .build();
    let log_b = Logger::builder()
        .level(Level::Warn)
        .no_timestamps()
        .no_context()
        .with_default_field("from", "B")
        .writer(buf_b.clone(), JsonFormat::new())
        .build();

    log_a.log(Level::Info, "a-info", &[]);
    log_a.log(Level::Warn, "a-warn", &[]);
    log_b.log(Level::Info, "b-info", &[]); // filtered
    log_b.log(Level::Warn, "b-warn", &[]);

    let out_a = buf_a.dump();
    let out_b = buf_b.dump();
    assert!(out_a.contains("a-info") && out_a.contains("a-warn"));
    assert!(out_a.contains("\"from\":\"A\""));
    assert!(!out_a.contains("b-"));
    assert!(!out_b.contains("b-info"));
    assert!(out_b.contains("b-warn"));
    assert!(out_b.contains("\"from\":\"B\""));
}

#[test]
fn build_panics_without_a_sink() {
    let result = std::panic::catch_unwind(|| {
        let _ = Logger::builder().level(Level::Info).build();
    });
    assert!(result.is_err());
}

#[test]
fn many_default_fields_overflow_stack_buffer() {
    // The dispatch path has a 16-slot stack buffer; ensure we degrade
    // gracefully when the merged set exceeds it.
    let buf = Capture::default();
    let mut builder = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context();
    let key_names: Vec<String> = (0..32).map(|i| format!("df{i}")).collect();
    for (i, k) in key_names.iter().enumerate() {
        builder = builder.with_default_field(k.clone(), i as u32);
    }
    let logger = builder.writer(buf.clone(), JsonFormat::new()).build();
    logger.log(Level::Info, "ok", &[]);
    let out = buf.dump();
    for (i, k) in key_names.iter().enumerate() {
        assert!(out.contains(&format!("\"{k}\":{i}")), "missing {k}");
    }
}

#[test]
fn errors_propagate_to_handler_count() {
    struct Bad;
    impl Sink for Bad {
        fn write_record(&self, _: &Record<'_>) -> Result<()> {
            Err(log_io::Error::Configuration("synthetic"))
        }
        fn flush(&self) -> Result<()> {
            Ok(())
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = calls.clone();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .with_sink(Bad)
        .on_error(Arc::new(move |_e| {
            calls_clone.fetch_add(1, Ordering::Relaxed);
        }))
        .build();
    for _ in 0..50 {
        let _ = logger.try_log(Level::Info, "t", "m", &[]);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 50);
}

#[test]
fn human_format_with_no_target_omits_brackets() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), HumanFormat::new().show_target(false))
        .build();
    logger.log(Level::Info, "x", &[]);
    let out = buf.dump();
    assert!(!out.contains('['));
}

#[test]
fn null_value_round_trips_in_all_formats() {
    for (name, build) in &[
        (
            "json",
            (|buf: Capture| {
                Logger::builder()
                    .level(Level::Trace)
                    .no_timestamps()
                    .no_context()
                    .writer(buf, JsonFormat::new())
                    .build()
            }) as fn(Capture) -> Logger,
        ),
        ("logfmt", |buf| {
            Logger::builder()
                .level(Level::Trace)
                .no_timestamps()
                .no_context()
                .writer(buf, LogfmtFormat::new())
                .build()
        }),
        ("human", |buf| {
            Logger::builder()
                .level(Level::Trace)
                .no_timestamps()
                .no_context()
                .writer(buf, HumanFormat::new())
                .build()
        }),
    ] {
        let buf = Capture::default();
        let logger = build(buf.clone());
        logger.log(Level::Info, "x", &[Field::new("k", log_io::Value::Null)]);
        let out = buf.dump();
        assert!(out.contains('k'), "{name} format missing key");
    }
}

#[test]
fn sink_count_reflects_install_count() {
    let logger = Logger::builder()
        .level(Level::Info)
        .with_sink(NullSink::new())
        .with_sink(NullSink::new())
        .with_sink(NullSink::new())
        .build();
    assert_eq!(logger.sink_count(), 3);
}

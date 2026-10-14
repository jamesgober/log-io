//! Regression tests for the 2026-10-13 audit findings.
//!
//! Each test is named after the finding it guards (LOG-0xx) and, where
//! the audit had one, its reproduction (Rxx). Every test here failed on
//! log-io 1.0.0.

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use log_io::format::{Format, HumanFormat, JsonFormat, LogfmtFormat};
use log_io::{
    ErrorHandler, Field, Filter, Level, Logger, Metadata, ParseFilterErrorKind, Record, Sink,
    Value, context,
};

#[derive(Default, Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn text(&self) -> String {
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

fn render<F: Format>(format: F, record: &Record<'_>) -> String {
    let mut out = String::new();
    format.write_record_generic(record, &mut out).unwrap();
    out
}

#[test]
fn log_002_r01_context_cannot_leak_across_threads_via_snapshots() {
    // The guard itself is `!Send` (see the compile_fail doctest on
    // `ContextGuard`). The supported way to move context is a snapshot,
    // which must leave nothing behind on either thread.
    context::clear();
    let snapshot = {
        let _tenant = context::with_field("tenant", "A");
        context::capture()
    };
    std::thread::spawn(move || {
        let _attached = snapshot.attach();
        assert_eq!(context::len(), 1);
    })
    .join()
    .unwrap();
    assert_eq!(context::len(), 0, "origin thread keeps no slot");

    let buf = Capture::default();
    let logger = Logger::builder()
        .no_timestamps()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    logger.log(Level::Info, "unrelated request", &[]);
    assert!(!buf.text().contains("tenant"), "{}", buf.text());
}

#[test]
fn log_003_r02_with_field_inside_a_sink_does_not_panic() {
    struct PushesContext;
    impl Sink for PushesContext {
        fn write_record(&self, _: &Record<'_>) -> log_io::Result<()> {
            let _g = context::with_field("x", 1_u32);
            Ok(())
        }
        fn flush(&self) -> log_io::Result<()> {
            Ok(())
        }
    }
    context::clear();
    let logger = Logger::builder().with_sink(PushesContext).build();
    let outcome = std::panic::catch_unwind(|| logger.log(Level::Info, "m", &[]));
    assert!(
        outcome.is_ok(),
        "pushing context from a sink must not panic"
    );
    assert_eq!(context::len(), 0, "the guard was dropped inside the sink");
}

#[test]
fn log_004_r03_most_specific_filter_rule_wins() {
    let f = Filter::parse("info,app=warn,app::db=trace").unwrap();
    assert!(f.is_enabled("app::db", Level::Trace));
    assert!(!f.is_enabled("app::web", Level::Info));
}

#[test]
fn log_005_r04_human_format_neutralizes_terminal_injection() {
    let fields = [
        Field::new("c", Value::Char('\n')),
        Field::new("k\u{1b}[2J", 1_u32),
    ];
    let record = Record::new(
        Metadata::new(Level::Info, "t\u{1b}]0;pwn\u{7}"),
        "msg \u{1b}[31mRED\u{b}VT\u{2028}LS\u{85}NEL\u{202e}",
        &fields,
    );
    let out = render(HumanFormat::new(), &record);
    assert_eq!(out.matches('\n').count(), 1, "{out:?}");
    assert!(
        !out.trim_end_matches('\n')
            .chars()
            .any(|c| c.is_control()
                || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}')),
        "{out:?}"
    );
}

#[test]
fn log_006_r06_reserved_and_duplicate_keys_cannot_spoof_metadata() {
    context::clear();
    let buf = Capture::default();
    let logger = Logger::builder()
        .level(Level::Trace)
        .no_timestamps()
        .with_default_field("level", "fake")
        .writer(buf.clone(), JsonFormat::new())
        .build();
    let _a = context::with_field("user", "outer");
    let _b = context::with_field("user", "inner");
    logger.log(Level::Error, "m", &[Field::new("message", "spoofed")]);
    let out = buf.text();
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["level"], "error", "{out}");
    assert_eq!(v["message"], "m", "{out}");
    assert_eq!(v["user"], "inner", "{out}");
    assert_eq!(v["fields.level"], "fake", "{out}");
    assert_eq!(v["fields.message"], "spoofed", "{out}");
    assert_eq!(out.matches("\"user\"").count(), 1, "{out}");
    context::clear();
}

#[test]
fn log_007_thread_buffer_is_released_after_huge_record() {
    // Behavioural check through the public API; the exact capacity is
    // asserted by a unit test in `sink.rs`.
    let logger = Logger::builder()
        .no_timestamps()
        .no_context()
        .writer(std::io::sink(), JsonFormat::new())
        .build();
    let huge = "x".repeat(8 * 1024 * 1024);
    logger.log(Level::Info, &huge, &[]);
    logger.log(Level::Info, "small", &[]);
}

#[test]
fn log_008_r12_guard_dropped_inside_a_sink_does_not_leak() {
    // `ContextGuard` is `!Send`, so a `Sync` sink cannot hold one; model
    // the audit's scenario with a thread-local the sink drains.
    struct DropsGuard;
    thread_local! {
        static HELD: std::cell::RefCell<Option<context::ContextGuard>> =
            const { std::cell::RefCell::new(None) };
    }
    impl Sink for DropsGuard {
        fn write_record(&self, _: &Record<'_>) -> log_io::Result<()> {
            HELD.with(|h| drop(h.borrow_mut().take()));
            Ok(())
        }
        fn flush(&self) -> log_io::Result<()> {
            Ok(())
        }
    }
    context::clear();
    HELD.with(|h| *h.borrow_mut() = Some(context::with_field("k", 1_u32)));
    let logger = Logger::builder().with_sink(DropsGuard).build();
    logger.log(Level::Info, "m", &[]);
    assert_eq!(context::len(), 0);
}

#[test]
fn log_009_error_handler_is_reachable_from_the_crate_root() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls2 = Arc::clone(&calls);
    let handler: ErrorHandler = Arc::new(move |_| {
        calls2.fetch_add(1, Ordering::Relaxed);
    });
    struct Failing;
    impl Sink for Failing {
        fn write_record(&self, _: &Record<'_>) -> log_io::Result<()> {
            Err(log_io::Error::Closed)
        }
        fn flush(&self) -> log_io::Result<()> {
            Ok(())
        }
    }
    let logger = Logger::builder()
        .with_sink(Failing)
        .on_error(handler)
        .build();
    logger.log(Level::Info, "m", &[]);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn log_011_format_is_object_safe_and_closures_work() {
    let formats: Vec<Box<dyn Format>> = vec![
        Box::new(JsonFormat::new()),
        Box::new(LogfmtFormat::new()),
        Box::new(HumanFormat::new()),
        Box::new(log_io::format::from_fn(
            |r: &Record<'_>, w: &mut dyn core::fmt::Write| writeln!(w, "{}", r.message),
        )),
    ];
    let record = Record::new(Metadata::new(Level::Info, "t"), "hello", &[]);
    for f in &formats {
        let as_dyn: &dyn Format = &**f;
        let mut out = String::new();
        as_dyn.write_record(&record, &mut out).unwrap();
        assert!(out.contains("hello"), "{out}");
    }
}

#[test]
fn log_012_r07_floats_stay_floats_and_parse() {
    for x in [
        1.0,
        f64::MAX,
        1e300,
        5e-324,
        f64::MIN_POSITIVE,
        -0.0,
        1e21,
        1e-7,
    ] {
        let fields = [Field::new("f", x)];
        let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
        let out = render(JsonFormat::new(), &record);
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert!(v["f"].is_f64(), "{x:e}: {out}");
        assert_eq!(v["f"].as_f64().unwrap().to_bits(), x.to_bits(), "{x:e}");
        assert!(out.len() < 120, "{x:e} is short: {out}");
    }
    let fields = [
        Field::new("nan", f64::NAN),
        Field::new("inf", f64::INFINITY),
    ];
    let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
    assert!(render(LogfmtFormat::new(), &record).ends_with("nan=NaN inf=inf\n"));
}

#[test]
fn log_014_r09_handler_runs_once_per_failing_sink() {
    struct Bad;
    impl Sink for Bad {
        fn write_record(&self, _: &Record<'_>) -> log_io::Result<()> {
            Err(log_io::Error::Configuration("x"))
        }
        fn flush(&self) -> log_io::Result<()> {
            Ok(())
        }
    }
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = Arc::clone(&n);
    let logger = Logger::builder()
        .with_sink(Bad)
        .with_sink(Bad)
        .on_error(Arc::new(move |_| {
            n2.fetch_add(1, Ordering::SeqCst);
        }))
        .build();
    logger.log(Level::Info, "m", &[]);
    assert_eq!(n.load(Ordering::SeqCst), 2);
}

#[test]
fn log_016_r05_logfmt_keys_and_values_are_sanitized() {
    let fields = [
        Field::new("k\u{1b}[0m", 1_u32),
        Field::new("", "v"),
        Field::new("v", "\u{9b}31m c1"),
    ];
    let record = Record::new(Metadata::new(Level::Info, "t"), "m", &fields);
    let out = render(LogfmtFormat::new(), &record);
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(!out.contains('\u{9b}'), "{out:?}");
    assert!(!out.contains(" =v"), "{out:?}");
    assert!(out.contains(" _=v "), "{out:?}");
}

#[test]
fn log_017_r10_degenerate_directives_are_rejected() {
    assert_eq!(
        Filter::parse("").unwrap_err().kind(),
        ParseFilterErrorKind::Empty
    );
    assert_eq!(
        Filter::parse("=debug").unwrap_err().kind(),
        ParseFilterErrorKind::EmptyTarget
    );
    assert_eq!(
        Filter::parse("app::=debug").unwrap_err().kind(),
        ParseFilterErrorKind::TrailingSeparator
    );
    let err = Logger::builder()
        .filter_directive("info,app=loud")
        .with_sink(log_io::sink::NullSink::new())
        .try_build()
        .unwrap_err();
    assert!(matches!(err, log_io::BuildError::InvalidFilter(_)));
}

#[test]
fn log_018_r11_log_uses_configurable_default_target() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .filter_directive("myapp=debug")
        .default_target("myapp")
        .no_timestamps()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    logger.log(Level::Error, "kept", &[]);
    assert!(buf.text().contains("kept"));
}

#[test]
fn log_019_r08_rfc3339_never_wraps() {
    for ts in [
        u128::MAX,
        u128::from(u64::MAX) * 1_000_000_000,
        253_402_300_800 * 1_000_000_000,
    ] {
        let record = Record::new(Metadata::new(Level::Info, "t").with_timestamp(ts), "m", &[]);
        let out = render(HumanFormat::new(), &record);
        assert!(out.starts_with("9999-12-31T23:59:59.999999999Z "), "{out}");
    }
}

#[test]
fn log_021_r13_mutex_poison_is_really_recovered() {
    struct PanicOnce(bool, Capture);
    impl Write for PanicOnce {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            if !self.0 {
                self.0 = true;
                panic!("writer panic while the sink lock is held");
            }
            self.1.write(b)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let cap = Capture::default();
    let logger = Logger::builder()
        .no_timestamps()
        .writer(PanicOnce(false, cap.clone()), JsonFormat::new())
        .build();
    let first = std::panic::catch_unwind(|| logger.log(Level::Info, "first", &[]));
    assert!(first.is_err(), "the writer must have panicked");
    logger.log(Level::Info, "second", &[]);
    assert!(cap.text().contains("second"));
}

#[test]
fn log_025_macros_evaluate_fields_only_when_enabled() {
    let evaluated = AtomicUsize::new(0);
    let probe = || {
        evaluated.fetch_add(1, Ordering::Relaxed);
        1_u32
    };
    let logger = Logger::builder()
        .level(Level::Warn)
        .with_sink(log_io::sink::NullSink::new())
        .build();
    log_io::info!(logger, "skipped {}", probe(), v = probe());
    assert_eq!(evaluated.load(Ordering::Relaxed), 0);
    log_io::warn!(logger, "kept {}", probe(), v = probe());
    assert_eq!(evaluated.load(Ordering::Relaxed), 2);
}

// Re-audit of 2.0.0 (2026-10-14). Each of these failed on 2.0.0 as of
// commit 0139f53.

#[test]
fn log_r06_call_site_duplicate_keys_are_written_once() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .no_timestamps()
        .no_context()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    // Last one wins, as in most JSON parsers.
    log_io::info!(logger, "m", a = 1_u8, b = 2_u8, a = 3_u8);
    // `level` is written as `fields.level`, so the two collide.
    logger.log(
        Level::Info,
        "m",
        &[Field::new("level", "x"), Field::new("fields.level", "y")],
    );
    let text = buf.text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert_eq!(lines[0].matches("\"a\"").count(), 1, "{}", lines[0]);
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["a"], 3, "{}", lines[0]);
    assert_eq!(first["b"], 2, "{}", lines[0]);
    assert_eq!(lines[1].matches("fields.level").count(), 1, "{}", lines[1]);
    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(second["fields.level"], "y", "{}", lines[1]);
    assert_eq!(second["level"], "info", "{}", lines[1]);
}

#[test]
fn log_r06_context_and_defaults_collide_with_prefixed_call_keys() {
    context::clear();
    let buf = Capture::default();
    let logger = Logger::builder()
        .no_timestamps()
        .with_default_field("level", "default")
        .writer(buf.clone(), LogfmtFormat::new())
        .build();
    let _ctx = context::with_field("fields.message", "ctx");
    logger.log(
        Level::Info,
        "m",
        &[
            Field::new("message", "call"),
            Field::new("fields.level", "call"),
        ],
    );
    let text = buf.text();
    assert_eq!(text.matches("fields.level=").count(), 1, "{text}");
    assert_eq!(text.matches("fields.message=").count(), 1, "{text}");
    assert!(text.contains("fields.level=call"), "{text}");
    assert!(text.contains("fields.message=call"), "{text}");
    context::clear();
}

#[test]
fn log_r08_comma_only_directive_is_an_error_not_off() {
    for directive in [",", " , ,", ",,,"] {
        let err = Filter::parse(directive).unwrap_err();
        assert_eq!(err.kind(), ParseFilterErrorKind::Empty, "{directive:?}");
    }
    let buf = Capture::default();
    let built = Logger::builder()
        .level(Level::Info)
        .filter_directive(",")
        .no_timestamps()
        .writer(buf.clone(), LogfmtFormat::new());
    // `try_build` reports it; `build` keeps the previous filter.
    let logger = built.build();
    logger.log(Level::Info, "still logged", &[]);
    assert!(buf.text().contains("still logged"), "{}", buf.text());
    let err = Logger::builder()
        .filter_directive(",")
        .with_sink(log_io::sink::NullSink::new())
        .try_build()
        .unwrap_err();
    assert!(matches!(err, log_io::BuildError::InvalidFilter(_)));
}

#[test]
fn log_r05_common_secrets_cover_header_style_keys() {
    let buf = Capture::default();
    let logger = Logger::builder()
        .no_timestamps()
        .no_context()
        .redact_common_secrets()
        .writer(buf.clone(), JsonFormat::new())
        .build();
    let fields = [
        Field::new("x-api-key", "s1"),
        Field::new("Api-Key", "s2"),
        Field::new("set-cookie", "s3"),
        Field::new("Proxy-Authorization", "s4"),
        Field::new("auth_token", "s5"),
        Field::new("http.request.header.x-api-key", "s6"),
        Field::new("privateKey", "s7"),
        Field::new("session_token", "s8"),
        Field::new("user", "visible"),
    ];
    logger.log(Level::Info, "request", &fields);
    let text = buf.text();
    for secret in ["s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8"] {
        assert!(
            !text.contains(&format!("\"{secret}\"")),
            "{secret} leaked: {text}"
        );
    }
    assert!(text.contains("\"visible\""), "{text}");
}

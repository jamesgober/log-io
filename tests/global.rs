//! The process-wide logger. One test, because the global logger can be
//! installed once per process and tests in a binary share the process.

use std::io::Write;
use std::sync::{Arc, Mutex};

use log_io::format::LogfmtFormat;
use log_io::{Level, Logger};

#[derive(Default, Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn take(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.0.lock().unwrap())).unwrap()
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
fn global_logger_lifecycle() {
    // Before installation everything is a harmless no-op.
    assert!(log_io::global::get().is_none());
    assert!(log_io::global::flush().is_ok());
    assert!(log_io::global::shutdown().is_ok());
    log_io::info!("nobody is listening");
    log_io::log_at!(Level::Error, "still nobody");

    let out = Capture::default();
    let logger = Logger::builder()
        .level(Level::Debug)
        .no_timestamps()
        .no_context()
        .writer(out.clone(), LogfmtFormat::new())
        .build();
    let guard = log_io::init(logger).unwrap();
    assert!(log_io::global::get().is_some());
    assert_eq!(guard.logger().min_level(), Level::Debug);

    // A second installation is rejected and hands the logger back.
    let second = Logger::builder()
        .with_sink(log_io::sink::NullSink::new())
        .build();
    let err = log_io::set_global(second).unwrap_err();
    assert_eq!(err.to_string(), "a global logger is already installed");
    assert_eq!(format!("{err:?}"), "SetGlobalError(..)");
    assert_eq!(err.into_inner().sink_count(), 1);

    // Macros without a logger argument use the global logger.
    let n = 3;
    log_io::info!("ready in {n} ms", port = 8080_u16);
    log_io::debug!(target: "app::db", "pool size {}", 4, idle = 1_u8);
    log_io::trace!("filtered out");
    log_io::log_at!(Level::Warn, "runtime level");
    log_io::log_at!(Level::Error, target: "custom", "with target {}", "x");
    let text = out.take();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "{text}");
    assert!(lines[0].starts_with("level=info target=global message=\"ready in 3 ms\""));
    assert!(lines[0].ends_with(" port=8080"), "{}", lines[0]);
    assert!(lines[1].starts_with("level=debug target=app::db message=\"pool size 4\""));
    assert!(lines[1].ends_with(" idle=1"), "{}", lines[1]);
    assert!(lines[2].starts_with("level=warn target=global message=\"runtime level\""));
    assert!(lines[3].starts_with("level=error target=custom message=\"with target x\""));

    assert!(log_io::global::flush().is_ok());
    drop(guard);
    // The guard shut the sinks down; the logger keeps working.
    log_io::info!("after the guard");
    assert!(out.take().contains("after the guard"));
}

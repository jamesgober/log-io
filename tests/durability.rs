//! Durability of buffered records across process exit (LOG-001).
//!
//! Each scenario runs in a child process: the test re-executes this test
//! binary, selecting a single `child_*` test with an environment
//! variable. The child installs a global logger writing to a buffered
//! `FileSink`, logs, and exits in a specific way; the parent then reads
//! the file. Run normally (without the variable), the `child_*` tests do
//! nothing.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use log_io::format::JsonFormat;
use log_io::sink::{FileSink, NonBlocking};
use log_io::{Level, Logger};

const ROLE: &str = "LOG_IO_DURABILITY_CHILD";
const PATH: &str = "LOG_IO_DURABILITY_PATH";

fn child_role() -> Option<(String, PathBuf)> {
    let role = std::env::var(ROLE).ok()?;
    let path = std::env::var_os(PATH)?;
    Some((role, PathBuf::from(path)))
}

fn run_child(test: &str) -> (String, std::process::Output) {
    let path = std::env::temp_dir().join(format!(
        "log-io-durability-{}-{test}.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let exe = std::env::current_exe().unwrap();
    let output = Command::new(exe)
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(ROLE, test)
        .env(PATH, &path)
        .output()
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    (text, output)
}

fn install(path: &Path) -> log_io::FlushGuard {
    let sink = FileSink::create(path, JsonFormat::new()).unwrap();
    let logger = Logger::builder()
        .level(Level::Info)
        .no_timestamps()
        .with_sink(sink)
        .build();
    log_io::init(logger).unwrap()
}

#[test]
fn child_guard_flushes_static_logger_at_end_of_main() {
    let Some((role, path)) = child_role() else {
        return;
    };
    assert_eq!(role, "child_guard_flushes_static_logger_at_end_of_main");
    let _guard = install(&path);
    for i in 0..10_u32 {
        log_io::info!("buffered", i = i);
    }
    // `_guard` drops here, as it would at the end of `main`.
}

#[test]
fn guard_flushes_static_logger_at_end_of_main() {
    // The audit's `static_file` reproduction produced a 0-byte file.
    let (text, out) = run_child("child_guard_flushes_static_logger_at_end_of_main");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(text.lines().count(), 10, "{text}");
}

#[test]
fn child_error_records_survive_exit_without_destructors() {
    let Some((_, path)) = child_role() else {
        return;
    };
    let guard = install(&path);
    log_io::info!("before the error");
    log_io::error!("fatal condition");
    log_io::info!("after the error, still buffered");
    // Skip every destructor, including the guard's.
    std::mem::forget(guard);
    std::process::exit(0);
}

#[test]
fn error_records_survive_exit_without_destructors() {
    let (text, out) = run_child("child_error_records_survive_exit_without_destructors");
    assert!(out.status.success(), "{out:?}");
    // Flush-on-error pushed the error and everything before it to the
    // OS. The record after it was still buffered and is lost, as the
    // `FileSink` docs state.
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains("before the error"));
    assert!(lines[1].contains("fatal condition"));
}

#[test]
fn child_shutdown_before_process_exit_keeps_everything() {
    let Some((_, path)) = child_role() else {
        return;
    };
    let guard = install(&path);
    for _ in 0..5 {
        log_io::info!("record");
    }
    log_io::global::shutdown().unwrap();
    std::mem::forget(guard);
    std::process::exit(0);
}

#[test]
fn shutdown_before_process_exit_keeps_everything() {
    let (text, out) = run_child("child_shutdown_before_process_exit_keeps_everything");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(text.lines().count(), 5, "{text}");
}

#[test]
fn child_panic_hook_logs_and_flushes() {
    let Some((_, path)) = child_role() else {
        return;
    };
    let guard = install(&path);
    log_io::global::install_panic_hook();
    log_io::info!("before the panic");
    let joined = std::thread::Builder::new()
        .name("worker-7".into())
        .spawn(|| panic!("worker exploded"))
        .unwrap()
        .join();
    assert!(joined.is_err());
    std::mem::forget(guard);
    std::process::exit(0);
}

#[test]
fn panic_hook_logs_and_flushes() {
    let (text, out) = run_child("child_panic_hook_logs_and_flushes");
    assert!(out.status.success(), "{out:?}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    let panic: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(panic["level"], "error");
    assert_eq!(panic["target"], "panic");
    assert_eq!(panic["message"], "worker exploded");
    assert_eq!(panic["thread"], "worker-7");
    assert!(panic["line"].as_u64().unwrap() > 0, "{panic}");
    assert!(panic["column"].as_u64().unwrap() > 0, "{panic}");
    assert!(
        panic["file"].as_str().unwrap().ends_with("durability.rs"),
        "{panic}"
    );
}

/// Run a `child_*` test, killing it if it has not exited after
/// `limit`. Returns `None` if it had to be killed.
fn run_child_with_timeout(test: &str, limit: Duration) -> Option<std::process::Output> {
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new(exe)
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(ROLE, test)
        .env(PATH, std::env::temp_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + limit;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Some(child.wait_with_output().unwrap())
}

#[test]
fn child_panic_on_background_worker_does_not_hang() {
    if child_role().is_none() {
        return;
    }
    /// Panics on its first write, on the sink's worker thread.
    struct PanicsOnce(bool);
    impl Write for PanicsOnce {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if !self.0 {
                self.0 = true;
                panic!("writer exploded on the worker");
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let sink = NonBlocking::builder(PanicsOnce(false), JsonFormat::new())
        .buffer_capacity(0)
        .build()
        .unwrap();
    let logger = Logger::builder()
        .level(Level::Info)
        .no_timestamps()
        .with_sink(sink)
        .build();
    let _guard = log_io::init(logger).unwrap();
    log_io::global::install_panic_hook();
    log_io::info!("first record");
    // Wait until the worker is gone and the sink reports `Closed`.
    let logger = log_io::global::get().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while logger.try_log(Level::Info, "probe", "probe", &[]).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    eprintln!("child: main returning");
    // `_guard` drops here and shuts the sink down.
}

#[test]
fn panic_on_background_worker_does_not_hang() {
    // LOG-R02: with the panic hook installed, a panic on a `NonBlocking`
    // worker used to make the hook wait for that same worker: the
    // process hung forever and the panic message was never printed.
    let out = run_child_with_timeout(
        "child_panic_on_background_worker_does_not_hang",
        Duration::from_secs(30),
    )
    .expect("the child process hung and was killed after 30 s");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{out:?}");
    assert!(
        stderr.contains("writer exploded on the worker"),
        "the original panic message must be printed: {stderr}"
    );
    assert!(stderr.contains("child: main returning"), "{stderr}");
}

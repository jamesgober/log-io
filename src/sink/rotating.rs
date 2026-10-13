//! Size- and time-based rotating file writer.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Time-based rotation schedule for [`RotatingFile`].
///
/// Periods are aligned to UTC: `Daily` rotates at 00:00 UTC, `Hourly`
/// at the top of each UTC hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Rotation {
    /// Never rotate on time; only `max_bytes` (if set) triggers
    /// rotation.
    #[default]
    Never,
    /// Rotate at the start of every UTC minute. Mostly useful for
    /// testing a rotation setup.
    Minutely,
    /// Rotate at the start of every UTC hour.
    Hourly,
    /// Rotate at 00:00 UTC every day.
    Daily,
}

impl Rotation {
    const fn period_secs(self) -> Option<u64> {
        match self {
            Self::Never => None,
            Self::Minutely => Some(60),
            Self::Hourly => Some(3_600),
            Self::Daily => Some(86_400),
        }
    }
}

/// A file writer that rotates by size and/or time.
///
/// The active file is always at the configured path. On rotation it is
/// renamed to `<path>.1`, an existing `<path>.1` becomes `<path>.2`,
/// and so on; files beyond `max_files` are deleted. With `max_files(0)`
/// the old content is discarded instead of kept.
///
/// Rotation happens between records, never inside one: the size check
/// runs before each `write` call, and the built-in sinks write a whole
/// record per call. A record larger than `max_bytes` is written to a
/// fresh file on its own.
///
/// Use it as the writer of a [`super::WriterSink`] (aliased as
/// [`super::RotatingFileSink`]) or of a [`super::NonBlocking`] sink.
///
/// # Platform behaviour
///
/// The current file handle is flushed and closed before any rename, so
/// rotation works on Windows as well as Unix. If a rename still fails
/// (on Windows, typically because another process holds the file open
/// without delete sharing), no record is lost: the writer reopens the
/// existing file, keeps appending, retries after another `max_bytes`
/// or at the next period, and reports the rotation error from the next
/// [`Write::flush`] call (which the logger passes to its error
/// handler).
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```no_run")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// use log_io::format::JsonFormat;
/// use log_io::sink::{RotatingFile, Rotation};
/// use log_io::{Level, Logger};
///
/// let file = RotatingFile::builder("logs/app.jsonl")
///     .max_bytes(50 * 1024 * 1024)
///     .rotation(Rotation::Daily)
///     .max_files(14)
///     .open()?;
/// let logger = Logger::builder().level(Level::Info).writer(file, JsonFormat::new()).build();
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct RotatingFile {
    path: PathBuf,
    file: Option<BufWriter<File>>,
    size: u64,
    max_bytes: Option<u64>,
    rotation: Rotation,
    max_files: usize,
    buffer_capacity: usize,
    /// Index of the UTC period the active file belongs to.
    period: Option<u64>,
    /// A rotation failure waiting to be reported by `flush`.
    deferred_error: Option<io::Error>,
    now: fn() -> SystemTime,
}

impl RotatingFile {
    /// Start configuring a rotating file at `path`. Defaults: no size
    /// limit, [`Rotation::Never`], keep 5 rotated files, 8 KiB buffer.
    pub fn builder(path: impl AsRef<Path>) -> RotatingFileBuilder {
        RotatingFileBuilder {
            path: path.as_ref().to_path_buf(),
            max_bytes: None,
            rotation: Rotation::Never,
            max_files: 5,
            buffer_capacity: 8 * 1024,
        }
    }

    /// The path of the active file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Bytes written to the active file so far (including bytes that
    /// existed when it was opened).
    #[must_use]
    pub fn current_size(&self) -> u64 {
        self.size
    }

    /// Rotate immediately, regardless of size and time.
    ///
    /// # Errors
    ///
    /// Returns the I/O error from flushing, renaming, or reopening. On
    /// error the writer keeps appending to the active file.
    pub fn rotate_now(&mut self) -> io::Result<()> {
        self.rotate()
    }

    fn rotated_path(&self, index: usize) -> PathBuf {
        let mut name = OsString::from(self.path.as_os_str());
        name.push(format!(".{index}"));
        PathBuf::from(name)
    }

    fn current_period(&self) -> Option<u64> {
        let secs = unix_secs((self.now)());
        self.rotation.period_secs().map(|p| secs / p)
    }

    fn should_rotate(&mut self, incoming: usize) -> bool {
        if self.size == 0 {
            // Never produce an empty rotated file; just track the period.
            self.period = self.current_period();
            return false;
        }
        let too_big = self
            .max_bytes
            .is_some_and(|max| self.size.saturating_add(incoming as u64) > max);
        let new_period = match (self.period, self.current_period()) {
            (Some(open), Some(now)) => now != open,
            _ => false,
        };
        too_big || new_period
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut writer) = self.file.take() {
            if let Err(e) = writer.flush() {
                self.file = Some(writer);
                return Err(e);
            }
            // Close the handle before renaming: Windows refuses to
            // rename a file this process still has open.
            drop(writer);
        }
        let shifted = self.shift_files();
        let reopened = open_append(&self.path, self.buffer_capacity);
        match reopened {
            Ok((writer, size)) => {
                self.file = Some(writer);
                // After a failed shift `size` is the old file's length;
                // count from zero anyway so the next attempt happens
                // after another `max_bytes` rather than on every write.
                self.size = if shifted.is_ok() { size } else { 0 };
                self.period = self.current_period();
                shifted
            }
            Err(e) => Err(e),
        }
    }

    fn shift_files(&self) -> io::Result<()> {
        if self.max_files == 0 {
            return remove_if_exists(&self.path);
        }
        remove_if_exists(&self.rotated_path(self.max_files))?;
        for index in (1..self.max_files).rev() {
            rename_if_exists(&self.rotated_path(index), &self.rotated_path(index + 1))?;
        }
        fs::rename(&self.path, self.rotated_path(1))
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.should_rotate(buf.len()) {
            if let Err(e) = self.rotate() {
                // Keep the first failure; report it from `flush`.
                if self.deferred_error.is_none() {
                    self.deferred_error = Some(e);
                }
            }
        }
        let writer = match self.file.as_mut() {
            Some(w) => w,
            None => {
                let (writer, size) = open_append(&self.path, self.buffer_capacity)?;
                self.size = size;
                self.file.insert(writer)
            }
        };
        writer.write_all(buf)?;
        self.size = self.size.saturating_add(buf.len() as u64);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(writer) = self.file.as_mut() {
            writer.flush()?;
        }
        match self.deferred_error.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl Drop for RotatingFile {
    fn drop(&mut self) {
        if let Some(writer) = self.file.as_mut() {
            // Nowhere left to report an error from a destructor; the
            // logger flushes (and reports) before dropping its sinks.
            let _flush_in_drop = writer.flush();
        }
    }
}

impl core::fmt::Debug for RotatingFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RotatingFile")
            .field("path", &self.path)
            .field("size", &self.size)
            .field("max_bytes", &self.max_bytes)
            .field("rotation", &self.rotation)
            .field("max_files", &self.max_files)
            .finish_non_exhaustive()
    }
}

/// Options for a [`RotatingFile`]. Create with [`RotatingFile::builder`].
#[derive(Debug, Clone)]
#[must_use = "call `open()` to create the writer"]
pub struct RotatingFileBuilder {
    path: PathBuf,
    max_bytes: Option<u64>,
    rotation: Rotation,
    max_files: usize,
    buffer_capacity: usize,
}

impl RotatingFileBuilder {
    /// Rotate before a write would take the active file past `bytes`.
    pub const fn max_bytes(mut self, bytes: u64) -> Self {
        self.max_bytes = Some(bytes);
        self
    }

    /// Rotate on a UTC time schedule.
    pub const fn rotation(mut self, rotation: Rotation) -> Self {
        self.rotation = rotation;
        self
    }

    /// How many rotated files to keep (`<path>.1` ..= `<path>.N`).
    /// `0` keeps none.
    pub const fn max_files(mut self, count: usize) -> Self {
        self.max_files = count;
        self
    }

    /// Size of the in-memory write buffer (`0` = unbuffered).
    pub const fn buffer_capacity(mut self, bytes: usize) -> Self {
        self.buffer_capacity = bytes;
        self
    }

    /// Open (or create) the active file in append mode.
    ///
    /// If the file already exists and was last modified in an earlier
    /// period of the time schedule, it is rotated on the first write.
    ///
    /// # Errors
    ///
    /// Returns the I/O error from opening the file.
    pub fn open(self) -> io::Result<RotatingFile> {
        self.open_with_clock(SystemTime::now)
    }

    fn open_with_clock(self, now: fn() -> SystemTime) -> io::Result<RotatingFile> {
        let (writer, size) = open_append(&self.path, self.buffer_capacity)?;
        let period = self.rotation.period_secs().map(|p| {
            let started = if size > 0 {
                writer
                    .get_ref()
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or_else(|_| now())
            } else {
                now()
            };
            unix_secs(started) / p
        });
        Ok(RotatingFile {
            path: self.path,
            file: Some(writer),
            size,
            max_bytes: self.max_bytes,
            rotation: self.rotation,
            max_files: self.max_files,
            buffer_capacity: self.buffer_capacity,
            period,
            deferred_error: None,
            now,
        })
    }
}

fn open_append(path: &Path, capacity: usize) -> io::Result<(BufWriter<File>, u64)> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let size = file.metadata()?.len();
    Ok((BufWriter::with_capacity(capacity, file), size))
}

fn unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn rename_if_exists(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("log-io-rotate-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn test_rotating_file_rotates_by_size_and_keeps_max_files() {
        let dir = TempDir::new("size");
        let path = dir.file("app.log");
        let mut w = RotatingFile::builder(&path)
            .max_bytes(10)
            .max_files(2)
            .open()
            .unwrap();
        for record in ["aaaa1\n", "bbbb2\n", "cccc3\n", "dddd4\n"] {
            w.write_all(record.as_bytes()).unwrap();
        }
        w.flush().unwrap();
        assert_eq!(read(&path), "dddd4\n");
        assert_eq!(read(&dir.file("app.log.1")), "cccc3\n");
        assert_eq!(read(&dir.file("app.log.2")), "bbbb2\n");
        assert!(
            !dir.file("app.log.3").exists(),
            "max_files must be enforced"
        );
    }

    #[test]
    fn test_rotating_file_never_splits_a_record() {
        let dir = TempDir::new("whole");
        let path = dir.file("a.log");
        let mut w = RotatingFile::builder(&path).max_bytes(8).open().unwrap();
        w.write_all(b"0123456789abcdef\n").unwrap();
        w.write_all(b"xy\n").unwrap();
        w.flush().unwrap();
        assert_eq!(read(&dir.file("a.log.1")), "0123456789abcdef\n");
        assert_eq!(read(&path), "xy\n");
        assert_eq!(w.current_size(), 3);
    }

    #[test]
    fn test_rotating_file_max_files_zero_discards_history() {
        let dir = TempDir::new("zero");
        let path = dir.file("z.log");
        let mut w = RotatingFile::builder(&path)
            .max_bytes(4)
            .max_files(0)
            .open()
            .unwrap();
        w.write_all(b"one\n").unwrap();
        w.write_all(b"two\n").unwrap();
        w.flush().unwrap();
        assert_eq!(read(&path), "two\n");
        assert!(!dir.file("z.log.1").exists());
    }

    #[test]
    fn test_rotating_file_appends_to_existing_and_counts_its_size() {
        let dir = TempDir::new("existing");
        let path = dir.file("e.log");
        fs::write(&path, "12345").unwrap();
        let mut w = RotatingFile::builder(&path).max_bytes(8).open().unwrap();
        assert_eq!(w.current_size(), 5);
        w.write_all(b"6789\n").unwrap();
        w.flush().unwrap();
        assert_eq!(read(&dir.file("e.log.1")), "12345");
        assert_eq!(read(&path), "6789\n");
    }

    static FAKE_NOW: AtomicU64 = AtomicU64::new(0);

    fn fake_clock() -> SystemTime {
        UNIX_EPOCH + std::time::Duration::from_secs(FAKE_NOW.load(Ordering::SeqCst))
    }

    #[test]
    fn test_rotating_file_rotates_on_period_boundary() {
        let dir = TempDir::new("time");
        let path = dir.file("t.log");
        FAKE_NOW.store(86_400 * 10 + 5, Ordering::SeqCst);
        let mut w = RotatingFile::builder(&path)
            .rotation(Rotation::Daily)
            .open_with_clock(fake_clock)
            .unwrap();
        w.write_all(b"day10\n").unwrap();
        FAKE_NOW.store(86_400 * 10 + 80_000, Ordering::SeqCst);
        w.write_all(b"day10b\n").unwrap();
        FAKE_NOW.store(86_400 * 11 + 1, Ordering::SeqCst);
        w.write_all(b"day11\n").unwrap();
        w.flush().unwrap();
        assert_eq!(read(&dir.file("t.log.1")), "day10\nday10b\n");
        assert_eq!(read(&path), "day11\n");
    }

    #[test]
    fn test_rotating_file_rotate_now_and_accessors() {
        let dir = TempDir::new("manual");
        let path = dir.file("m.log");
        let mut w = RotatingFile::builder(&path)
            .rotation(Rotation::Hourly)
            .buffer_capacity(0)
            .open()
            .unwrap();
        assert_eq!(w.path(), path.as_path());
        w.write_all(b"before\n").unwrap();
        w.rotate_now().unwrap();
        w.write_all(b"after\n").unwrap();
        w.flush().unwrap();
        assert_eq!(read(&dir.file("m.log.1")), "before\n");
        assert_eq!(read(&path), "after\n");
        assert!(format!("{w:?}").contains("Hourly"));
        assert_eq!(Rotation::default(), Rotation::Never);
        assert_eq!(Rotation::Minutely.period_secs(), Some(60));
    }

    #[test]
    fn test_rotating_file_failed_rotation_keeps_writing_and_defers_error() {
        let dir = TempDir::new("fail");
        let path = dir.file("f.log");
        let mut w = RotatingFile::builder(&path)
            .max_bytes(4)
            .max_files(1)
            .open()
            .unwrap();
        w.write_all(b"one\n").unwrap();
        // Make the shift fail: a directory where `f.log.1` should go
        // cannot be removed with `remove_file` on any platform.
        fs::create_dir(dir.file("f.log.1")).unwrap();
        w.write_all(b"two\n").unwrap();
        assert!(w.flush().is_err(), "rotation failure must surface on flush");
        assert!(w.flush().is_ok(), "the error is reported once");
        assert_eq!(read(&path), "one\ntwo\n", "no record may be lost");
    }
}

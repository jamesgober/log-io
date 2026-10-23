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
/// and so on; the file that would move past `<path>.<max_files>` is
/// deleted. With `max_files(0)` the active file is deleted at rotation
/// instead of kept.
///
/// Rotation happens between records, never inside one: the size check
/// runs before each `write` call, and the built-in sinks write a whole
/// record per call. A record larger than `max_bytes` is written to a
/// fresh file on its own.
///
/// Use it as the writer of a [`super::WriterSink`] (aliased as
/// [`super::RotatingFileSink`]) or of a [`super::NonBlocking`] sink.
/// Behind a `NonBlocking` sink, records reach this writer in batches
/// through the worker's write buffer (64 KiB by default), so a file can
/// exceed `max_bytes` by up to that buffer size and period boundaries
/// are honoured per batch; set the worker's `buffer_capacity(0)` for
/// per-record precision.
///
/// # Failed rotation
///
/// The current file handle is flushed and closed before any rename, so
/// rotation works on Windows as well as Unix. A rotation is all or
/// nothing:
///
/// 1. the active file is moved to `<path>.rotating`. If that fails (on
///    Windows, typically because another process holds the file open
///    without delete sharing), nothing else has been touched;
/// 2. the history is shifted up one slot, the oldest file going to
///    `<path>.pruning` rather than being deleted;
/// 3. `<path>.rotating` becomes `<path>.1`, and only then is
///    `<path>.pruning` deleted.
///
/// If a rename in steps 2-3 fails, the renames already done are undone
/// in reverse order. So a failed rotation never deletes or overwrites
/// rotated history: every rotated file keeps its name and content. The
/// writer then reopens the active file, keeps appending to it, retries
/// after another `max_bytes` or at the next period, and reports the
/// error from the next [`Write::flush`] call (which the logger passes
/// to its error handler).
///
/// If an undo fails as well (a second fault, for example the directory
/// becoming read-only mid-rotation), the rollback stops and the files
/// keep whatever names they have at that point; still nothing is
/// deleted. Records that were in the active file stay in
/// `<path>.rotating`, the writer starts a new active file, and every
/// later rotation attempt fails with [`io::ErrorKind::AlreadyExists`]
/// naming that file (so it is never overwritten) until an operator
/// moves it aside.
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
    /// Unix permission bits for files this writer creates.
    mode: Option<u32>,
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
            mode: None,
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
        // Read the clock only when a schedule is configured: `now` costs
        // tens of nanoseconds per record on some platforms.
        self.rotation
            .period_secs()
            .map(|p| unix_secs((self.now)()) / p)
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
        too_big
            || self
                .period
                .is_some_and(|open| self.current_period() != Some(open))
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
        let rotated = self.move_active_into_history();
        let reopened = open_append(&self.path, self.buffer_capacity, self.mode);
        match reopened {
            Ok((writer, size)) => {
                self.file = Some(writer);
                // After a failed rotation `size` is the old file's
                // length; count from zero anyway so the next attempt
                // happens after another `max_bytes` rather than on every
                // write.
                self.size = if rotated.is_ok() { size } else { 0 };
                self.period = self.current_period();
                rotated
            }
            Err(e) => Err(e),
        }
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = OsString::from(self.path.as_os_str());
        name.push(suffix);
        PathBuf::from(name)
    }

    /// Move the (closed) active file to `<path>.1`, shifting and pruning
    /// the history, as one all-or-nothing step.
    ///
    /// The active file is first moved to a staging name, so the common
    /// failure (another process holds the active file) is detected
    /// before any history file is touched. The oldest file is moved to a
    /// pruning name instead of being deleted, and deleted only after
    /// every other rename has succeeded. If any rename fails, the ones
    /// already done are undone in reverse order and the error is
    /// returned: rotated history is never deleted or overwritten by a
    /// rotation that did not complete.
    fn move_active_into_history(&self) -> io::Result<()> {
        if self.max_files == 0 {
            // No history is kept: rotation discards the active file.
            return remove_if_exists(&self.path);
        }
        let staging = self.sibling(STAGING_SUFFIX);
        let pruning = self.sibling(PRUNING_SUFFIX);
        if fs::symlink_metadata(&staging).is_ok() {
            // Left behind by a rotation whose rollback also failed. It
            // holds records; never overwrite it.
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "{} was left by an interrupted rotation and holds log records;                      move it aside to resume rotation",
                    staging.display()
                ),
            ));
        }
        // Only a completed rotation leaves this file, and its content
        // was already pruned by that rotation.
        remove_if_exists(&pruning)?;

        let mut moves = Vec::with_capacity(self.max_files + 2);
        moves.push(Move::required(self.path.clone(), staging.clone()));
        moves.push(Move::optional(
            self.rotated_path(self.max_files),
            pruning.clone(),
        ));
        for index in (1..self.max_files).rev() {
            moves.push(Move::optional(
                self.rotated_path(index),
                self.rotated_path(index + 1),
            ));
        }
        moves.push(Move::required(staging, self.rotated_path(1)));

        let mut done = Vec::with_capacity(moves.len());
        for step in &moves {
            match rename_file(&step.from, &step.to) {
                Ok(()) => done.push(step),
                Err(e) if !step.required && e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    roll_back(&done);
                    return Err(e);
                }
            }
        }
        // The rotation is complete. If this fails, the pruned file stays
        // under the pruning name and is removed by the next rotation.
        remove_if_exists(&pruning)
    }
}

/// Name the active file is moved to while the history is shifted.
const STAGING_SUFFIX: &str = ".rotating";
/// Name the oldest rotated file is moved to until the rotation is
/// complete.
const PRUNING_SUFFIX: &str = ".pruning";

/// One rename of a rotation.
struct Move {
    from: PathBuf,
    to: PathBuf,
    /// `false` for history slots, which may legitimately be missing.
    required: bool,
}

impl Move {
    const fn required(from: PathBuf, to: PathBuf) -> Self {
        Self {
            from,
            to,
            required: true,
        }
    }

    const fn optional(from: PathBuf, to: PathBuf) -> Self {
        Self {
            from,
            to,
            required: false,
        }
    }
}

/// Undo completed renames, newest first.
///
/// Each undo moves a file back to the name the forward rename vacated,
/// and every later rename that targeted that name has already been
/// undone. If an undo fails, stop: undoing an earlier rename could then
/// replace a file that is still in the way. The files keep their
/// shifted names (nothing is deleted), and the staging file left behind
/// makes later rotations refuse to run until an operator steps in.
fn roll_back(done: &[&Move]) {
    for step in done.iter().rev() {
        if rename_file(&step.to, &step.from).is_err() {
            return;
        }
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
                let (writer, size) = open_append(&self.path, self.buffer_capacity, self.mode)?;
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
    mode: Option<u32>,
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

    /// Unix permission bits for every file this writer creates: the active
    /// file and each new one after a rotation, such as `0o600` or `0o640`.
    /// The process umask still applies, and an existing file keeps its
    /// mode. Without this call files get `0o666` minus the umask, as
    /// before 2.1. On Windows files inherit their directory's access
    /// control list and the mode has no effect.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use log_io::sink::{RotatingFile, Rotation};
    ///
    /// let file = RotatingFile::builder("app.log")
    ///     .rotation(Rotation::Daily)
    ///     .mode(0o640)
    ///     .open()?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub const fn mode(mut self, mode: u32) -> Self {
        self.mode = Some(mode);
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
        let (writer, size) = open_append(&self.path, self.buffer_capacity, self.mode)?;
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
            mode: self.mode,
            period,
            deferred_error: None,
            now,
        })
    }
}

fn open_append(
    path: &Path,
    capacity: usize,
    mode: Option<u32>,
) -> io::Result<(BufWriter<File>, u64)> {
    let mut options = OpenOptions::new();
    let _options = options.create(true).append(true);
    super::apply_mode(&mut options, mode);
    let file = options.open(path)?;
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

fn rename_file(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(test)]
    if tests::rename_is_blocked(from, to) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "rename blocked by the test",
        ));
    }
    fs::rename(from, to)
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
    fn test_rotating_file_prune_failure_is_reported_once_and_keeps_records() {
        let dir = TempDir::new("fail");
        let path = dir.file("f.log");
        let mut w = RotatingFile::builder(&path)
            .max_bytes(4)
            .max_files(1)
            .open()
            .unwrap();
        w.write_all(b"one\n").unwrap();
        // A directory in the oldest slot is moved to the pruning name
        // but cannot be removed with `remove_file` on any platform.
        fs::create_dir(dir.file("f.log.1")).unwrap();
        w.write_all(b"two\n").unwrap();
        assert!(w.flush().is_err(), "rotation failure must surface on flush");
        assert!(w.flush().is_ok(), "the error is reported once");
        assert_eq!(read(&dir.file("f.log.1")), "one\n");
        assert_eq!(read(&path), "two\n");
        assert!(dir.file("f.log.pruning").is_dir());
        // The leftover still cannot be removed, so the next rotation is
        // refused before anything moves.
        w.write_all(b"three\n").unwrap();
        assert!(w.flush().is_err());
        assert_eq!(read(&dir.file("f.log.1")), "one\n");
        assert_eq!(read(&path), "two\nthree\n", "no record may be lost");
    }

    #[test]
    fn test_rotating_file_rotates_at_exactly_max_bytes_not_before() {
        let dir = TempDir::new("edge");
        let path = dir.file("edge.log");
        let mut w = RotatingFile::builder(&path)
            .max_bytes(10)
            .buffer_capacity(0)
            .open()
            .unwrap();
        w.write_all(b"01234").unwrap();
        w.write_all(b"56789").unwrap();
        assert!(!dir.file("edge.log.1").exists(), "exactly max_bytes fits");
        w.write_all(b"a").unwrap();
        assert_eq!(read(&dir.file("edge.log.1")), "0123456789");
        assert_eq!(read(&path), "a");
    }

    #[test]
    fn test_rotating_file_drop_flushes_buffered_records() {
        let dir = TempDir::new("drop");
        let path = dir.file("d.log");
        let mut w = RotatingFile::builder(&path).open().unwrap();
        w.write_all(b"buffered\n").unwrap();
        assert_eq!(read(&path), "", "still in the write buffer");
        drop(w);
        assert_eq!(read(&path), "buffered\n");
    }

    #[test]
    fn test_rotating_file_never_reads_the_clock_without_a_schedule() {
        fn exploding_clock() -> SystemTime {
            panic!("the clock must not be read for Rotation::Never");
        }
        let dir = TempDir::new("noclock");
        let path = dir.file("n.log");
        let mut w = RotatingFile::builder(&path)
            .max_bytes(4)
            .open_with_clock(exploding_clock)
            .unwrap();
        w.write_all(b"one\n").unwrap();
        w.write_all(b"two\n").unwrap();
        w.flush().unwrap();
        assert_eq!(read(&dir.file("n.log.1")), "one\n");
    }

    thread_local! {
        /// Renames (from, to) that fail with `PermissionDenied`, to
        /// simulate on every platform what another process holding a
        /// file does on Windows.
        static BLOCKED: core::cell::RefCell<Vec<(PathBuf, PathBuf)>> =
            const { core::cell::RefCell::new(Vec::new()) };
    }

    pub(super) fn rename_is_blocked(from: &Path, to: &Path) -> bool {
        BLOCKED.with(|b| b.borrow().iter().any(|(f, t)| f == from && t == to))
    }

    fn block_rename(from: PathBuf, to: PathBuf) {
        BLOCKED.with(|b| b.borrow_mut().push((from, to)));
    }

    fn unblock_renames() {
        BLOCKED.with(|b| b.borrow_mut().clear());
    }

    /// Writer with `max_bytes(5)`, `max_files(3)`, the active file
    /// holding `11111` and history `.1..=.3` seeded.
    fn seeded_writer(dir: &TempDir, base: &str) -> RotatingFile {
        let mut w = RotatingFile::builder(dir.file(base))
            .max_bytes(5)
            .max_files(3)
            .buffer_capacity(0)
            .open()
            .unwrap();
        w.write_all(b"11111").unwrap();
        seed_history(dir, base, 3);
        w
    }

    fn rotation_files(dir: &TempDir) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn test_rotating_file_blocked_active_rename_keeps_history_on_every_platform() {
        let dir = TempDir::new("blocked-active");
        let mut w = seeded_writer(&dir, "a.log");
        block_rename(dir.file("a.log"), dir.file("a.log.rotating"));
        for i in 0..6 {
            w.write_all(format!("r{i}xxx").as_bytes()).unwrap();
            let _reported = w.flush();
        }
        unblock_renames();
        assert_history_intact(&dir, "a.log", 3);
        assert_eq!(
            read(&dir.file("a.log")),
            "11111r0xxxr1xxxr2xxxr3xxxr4xxxr5xxx"
        );
        assert_eq!(
            rotation_files(&dir),
            ["a.log", "a.log.1", "a.log.2", "a.log.3"]
        );
    }

    #[test]
    fn test_rotating_file_blocked_history_rename_rolls_back() {
        let dir = TempDir::new("blocked-history");
        let mut w = seeded_writer(&dir, "h.log");
        // The shift moves `.3` to the pruning name and then fails on
        // `.2 -> .3`; both the prune and the staging move are undone.
        block_rename(dir.file("h.log.2"), dir.file("h.log.3"));
        w.write_all(b"r0xxx").unwrap();
        let err = w.flush().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        unblock_renames();
        assert_history_intact(&dir, "h.log", 3);
        assert_eq!(read(&dir.file("h.log")), "11111r0xxx");
        assert_eq!(
            rotation_files(&dir),
            ["h.log", "h.log.1", "h.log.2", "h.log.3"]
        );
    }

    #[test]
    fn test_rotating_file_blocked_final_rename_rolls_back() {
        let dir = TempDir::new("blocked-final");
        let mut w = seeded_writer(&dir, "s.log");
        block_rename(dir.file("s.log.rotating"), dir.file("s.log.1"));
        w.write_all(b"r0xxx").unwrap();
        assert!(w.flush().is_err());
        unblock_renames();
        assert_history_intact(&dir, "s.log", 3);
        assert_eq!(read(&dir.file("s.log")), "11111r0xxx");
        // Unblocked, the next rotation completes.
        w.rotate_now().unwrap();
        w.write_all(b"r1xxx").unwrap();
        w.flush().unwrap();
        assert_eq!(read(&dir.file("s.log")), "r1xxx");
        assert_eq!(read(&dir.file("s.log.1")), "11111r0xxx");
        assert_eq!(read(&dir.file("s.log.2")), "OLD1\n");
        assert_eq!(read(&dir.file("s.log.3")), "OLD2\n");
        assert_eq!(
            rotation_files(&dir),
            ["s.log", "s.log.1", "s.log.2", "s.log.3"],
            "OLD3 is pruned only by the rotation that completed"
        );
    }

    #[test]
    fn test_rotating_file_failed_rollback_deletes_nothing_and_stops_rotating() {
        let dir = TempDir::new("blocked-rollback");
        let mut w = seeded_writer(&dir, "r.log");
        // Forward: the final staging rename fails. Rollback: undoing
        // `.1 -> .2` fails too, so the rollback stops there.
        block_rename(dir.file("r.log.rotating"), dir.file("r.log.1"));
        block_rename(dir.file("r.log.2"), dir.file("r.log.1"));
        w.write_all(b"r0xxx").unwrap();
        assert!(w.flush().is_err());
        unblock_renames();
        let mut contents: Vec<String> = rotation_files(&dir)
            .iter()
            .map(|name| read(&dir.file(name)))
            .collect();
        contents.sort();
        // Every record is still on disk under some name.
        assert_eq!(contents, ["11111", "OLD1\n", "OLD2\n", "OLD3\n", "r0xxx"]);
        assert_eq!(read(&dir.file("r.log.rotating")), "11111");
        // With records stranded in the staging file, rotation refuses to
        // run rather than overwrite it, and the new file keeps growing.
        w.write_all(b"r1xxx").unwrap();
        w.write_all(b"r2xxx").unwrap();
        let err = w.flush().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains("r.log.rotating"), "{err}");
        assert_eq!(read(&dir.file("r.log.rotating")), "11111");
        assert_eq!(read(&dir.file("r.log")), "r0xxxr1xxxr2xxx");
    }

    fn seed_history(dir: &TempDir, base: &str, count: usize) {
        for index in 1..=count {
            fs::write(
                dir.file(&format!("{base}.{index}")),
                format!(
                    "OLD{index}
"
                ),
            )
            .unwrap();
        }
    }

    fn assert_history_intact(dir: &TempDir, base: &str, count: usize) {
        for index in 1..=count {
            assert_eq!(
                read(&dir.file(&format!("{base}.{index}"))),
                format!(
                    "OLD{index}
"
                ),
                "{base}.{index} must survive a failed rotation"
            );
        }
    }

    /// LOG-R01: a rename blocked by another process (Windows opens
    /// without `FILE_SHARE_DELETE`) must not cost any rotated history,
    /// no matter how often rotation is retried.
    #[cfg(windows)]
    #[test]
    fn test_rotating_file_held_active_file_keeps_history_on_windows() {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;
        let dir = TempDir::new("held");
        let path = dir.file("held.log");
        let mut w = RotatingFile::builder(&path)
            .max_bytes(5)
            .max_files(3)
            .buffer_capacity(0)
            .open()
            .unwrap();
        w.write_all(b"11111").unwrap();
        seed_history(&dir, "held.log", 3);
        let holder = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        let mut failures = 0;
        for i in 0..6 {
            w.write_all(format!("r{i}xxx").as_bytes()).unwrap();
            if w.flush().is_err() {
                failures += 1;
            }
        }
        drop(holder);
        assert!(failures > 0, "the blocked rotation must be reported");
        assert_history_intact(&dir, "held.log", 3);
        assert_eq!(read(&path), "11111r0xxxr1xxxr2xxxr3xxxr4xxxr5xxx");
        assert!(!dir.file("held.log.4").exists());
        // Once the other handle is gone, rotation resumes normally.
        w.rotate_now().unwrap();
        assert_eq!(
            read(&dir.file("held.log.1")),
            "11111r0xxxr1xxxr2xxxr3xxxr4xxxr5xxx"
        );
        assert_eq!(
            read(&dir.file("held.log.2")),
            "OLD1
"
        );
        assert_eq!(
            read(&dir.file("held.log.3")),
            "OLD2
"
        );
        assert!(!dir.file("held.log.4").exists(), "max_files still applies");
    }

    #[cfg(unix)]
    #[test]
    fn test_rotating_mode_applies_to_active_and_rotated_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("mode");
        let mut w = RotatingFile::builder(dir.file("app.log"))
            .max_bytes(5)
            .max_files(2)
            .buffer_capacity(0)
            .mode(0o600)
            .open()
            .unwrap();
        w.write_all(b"11111").unwrap();
        w.write_all(b"22222").unwrap(); // rotates: a new active file is created
        w.flush().unwrap();
        for name in ["app.log", "app.log.1"] {
            let mode = fs::metadata(dir.file(name)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{name}");
        }
    }
}

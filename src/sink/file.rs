//! Buffered file sink.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use super::{Sink, lock_recover, write_locked};
use crate::error::{Error, Result};
use crate::format::Format;
use crate::record::Record;

/// Default size of the write buffer in front of the file.
const DEFAULT_BUFFER: usize = 8 * 1024;

/// Writes records to a file through a write buffer.
///
/// # Durability
///
/// Records are buffered in memory (8 KiB by default) and reach the
/// operating system when the buffer fills or the sink is flushed. A
/// flush happens:
///
/// * after every record at or above the logger's flush level (by
///   default `Level::Error`, see `LoggerBuilder::flush_on`);
/// * on [`crate::Logger::flush`] and [`crate::Logger::shutdown`];
/// * when a [`crate::FlushGuard`] is dropped;
/// * when the last clone of the [`crate::Logger`] is dropped.
///
/// Anything still buffered is **lost** if the process ends without
/// one of those happening: `std::process::abort`, `panic = "abort"`
/// without [`crate::global::install_panic_hook`], a fatal signal,
/// `std::process::exit` (which skips destructors), or simply returning
/// from `main` while the logger lives in a `static` (statics are never
/// dropped). Keep the [`crate::FlushGuard`] returned by
/// [`crate::init`] alive in `main`, call [`crate::Logger::shutdown`]
/// before `process::exit`, or build the sink with
/// [`FileSinkBuilder::buffer_capacity`]`(0)` to write every record
/// through immediately.
///
/// Flushing hands data to the OS, which survives a crash of this
/// process but not a power loss or kernel crash. Enable
/// [`FileSinkBuilder::sync_on_flush`] to also `fsync` (`sync_data`)
/// on every flush.
///
/// # Examples
///
/// ```no_run
/// use log_io::format::JsonFormat;
/// use log_io::sink::FileSink;
/// use log_io::{Level, Logger};
///
/// let sink = FileSink::builder("app.jsonl", JsonFormat::new())
///     .append(true)
///     .sync_on_flush(true)
///     .open()?;
/// let logger = Logger::builder().level(Level::Info).with_sink(sink).build();
/// log_io::error!(logger, "disk almost full"); // flushed and synced before returning
/// # Ok::<(), log_io::Error>(())
/// ```
pub struct FileSink<F: Format> {
    format: F,
    target: Mutex<BufWriter<File>>,
    sync_on_flush: bool,
}

impl<F: Format> FileSink<F> {
    /// Open `path` for appending (creating it if needed) with default
    /// options.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be opened or created.
    pub fn append<P: AsRef<Path>>(path: P, format: F) -> Result<Self> {
        Self::builder(path, format).append(true).open()
    }

    /// Open `path` for writing, truncating any existing content.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be opened or created.
    pub fn create<P: AsRef<Path>>(path: P, format: F) -> Result<Self> {
        Self::builder(path, format).append(false).open()
    }

    /// Start configuring a file sink. Defaults: append, 8 KiB buffer,
    /// no `fsync`.
    pub fn builder<P: AsRef<Path>>(path: P, format: F) -> FileSinkBuilder<F> {
        FileSinkBuilder {
            path: path.as_ref().to_path_buf(),
            format,
            append: true,
            buffer_capacity: DEFAULT_BUFFER,
            sync_on_flush: false,
        }
    }
}

impl<F: Format> Sink for FileSink<F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        write_locked(&self.target, &self.format, record)
    }

    fn flush(&self) -> Result<()> {
        let mut file = lock_recover(&self.target);
        file.flush()?;
        if self.sync_on_flush {
            file.get_ref().sync_data().map_err(Error::from)?;
        }
        Ok(())
    }
}

impl<F: Format + core::fmt::Debug> core::fmt::Debug for FileSink<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileSink")
            .field("format", &self.format)
            .field("sync_on_flush", &self.sync_on_flush)
            .finish_non_exhaustive()
    }
}

/// Options for a [`FileSink`]. Create with [`FileSink::builder`].
#[derive(Debug)]
#[must_use = "call `open()` to create the sink"]
pub struct FileSinkBuilder<F> {
    path: std::path::PathBuf,
    format: F,
    append: bool,
    buffer_capacity: usize,
    sync_on_flush: bool,
}

impl<F: Format> FileSinkBuilder<F> {
    /// Append to an existing file (`true`, the default) or truncate it
    /// (`false`). The file is created if it does not exist.
    pub const fn append(mut self, append: bool) -> Self {
        self.append = append;
        self
    }

    /// Size of the in-memory write buffer in bytes. `0` disables
    /// buffering: every record is written to the OS before
    /// `write_record` returns, at the cost of one system call per
    /// record.
    pub const fn buffer_capacity(mut self, bytes: usize) -> Self {
        self.buffer_capacity = bytes;
        self
    }

    /// Also call `File::sync_data` on every flush, so flushed records
    /// survive an OS crash or power loss. This makes each flush much
    /// slower (one disk sync).
    pub const fn sync_on_flush(mut self, enabled: bool) -> Self {
        self.sync_on_flush = enabled;
        self
    }

    /// Open the file and build the sink.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the file cannot be opened or created.
    pub fn open(self) -> Result<FileSink<F>> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(self.append)
            .truncate(!self.append)
            .open(&self.path)?;
        Ok(FileSink {
            format: self.format,
            target: Mutex::new(BufWriter::with_capacity(self.buffer_capacity, file)),
            sync_on_flush: self.sync_on_flush,
        })
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::format::JsonFormat;
    use crate::{Level, Metadata};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("log-io-file-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&dir);
        dir
    }

    fn record(msg: &str) -> Record<'_> {
        Record::new(Metadata::new(Level::Info, "t"), msg, &[])
    }

    #[test]
    fn test_file_sink_append_preserves_existing_content() {
        let path = temp_path("append");
        std::fs::write(&path, "existing\n").unwrap();
        let sink = FileSink::append(&path, JsonFormat::new()).unwrap();
        sink.write_record(&record("one")).unwrap();
        sink.flush().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("existing\n"), "{text}");
        assert!(text.contains("\"message\":\"one\""), "{text}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_file_sink_create_truncates() {
        let path = temp_path("create");
        std::fs::write(&path, "old content\n").unwrap();
        let sink = FileSink::create(&path, JsonFormat::new()).unwrap();
        sink.write_record(&record("fresh")).unwrap();
        sink.shutdown().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("old content"), "{text}");
        assert!(text.contains("fresh"), "{text}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_file_sink_buffers_until_flush() {
        let path = temp_path("buffered");
        let sink = FileSink::create(&path, JsonFormat::new()).unwrap();
        sink.write_record(&record("pending")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap().len(), 0, "still buffered");
        sink.flush().unwrap();
        assert!(!std::fs::read(&path).unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_file_sink_unbuffered_writes_through() {
        let path = temp_path("unbuffered");
        let sink = FileSink::builder(&path, JsonFormat::new())
            .append(false)
            .buffer_capacity(0)
            .sync_on_flush(true)
            .open()
            .unwrap();
        sink.write_record(&record("now")).unwrap();
        assert!(
            std::fs::read_to_string(&path).unwrap().contains("now"),
            "unbuffered sink must write before returning"
        );
        sink.flush().unwrap();
        assert!(format!("{sink:?}").contains("sync_on_flush: true"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_file_sink_open_error_is_reported() {
        let dir = std::env::temp_dir().join(format!("log-io-missing-{}", std::process::id()));
        let path = dir.join("nested").join("file.log");
        let err = FileSink::append(&path, JsonFormat::new()).unwrap_err();
        assert!(matches!(err, Error::Io(_)));
    }
}

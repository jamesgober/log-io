//! A bounded in-memory ring of the last formatted lines.

use std::fmt;
use std::sync::{Mutex, PoisonError, TryLockError};

use super::{Sink, lock_recover, with_formatted};
use crate::error::Result;
use crate::format::Format;
use crate::record::Record;

/// Default maximum length of one stored line, in bytes.
pub const DEFAULT_LINE_LIMIT: usize = 1024;

/// Keeps the last `N` formatted lines in memory, for a crash report or a
/// status page.
///
/// Every slot is allocated when the sink is built (`capacity` slots of
/// `line_limit` bytes each), so memory is fixed and writing a record does
/// not allocate. A line longer than the limit is cut at the last character
/// boundary that fits. Lines are formatted with the given [`Format`] from
/// the record the logger built, so values the logger redacts are redacted
/// here too.
///
/// Keep an `Arc` to read the lines and give the logger a clone (`Sink` is
/// implemented for `Arc<S>`). [`try_snapshot`](Self::try_snapshot) and
/// [`try_write_to`](Self::try_write_to) never block, so a panic hook can
/// read the ring even when the panic happened while this sink held its
/// lock.
///
/// # Examples
///
#[cfg_attr(all(feature = "logfmt"), doc = "```")]
#[cfg_attr(not(all(feature = "logfmt")), doc = "```ignore")]
/// use std::sync::Arc;
/// use log_io::format::LogfmtFormat;
/// use log_io::sink::RecentLines;
/// use log_io::Logger;
///
/// let recent = Arc::new(RecentLines::new(2, LogfmtFormat::new()));
/// let logger = Logger::builder()
///     .no_timestamps()
///     .with_sink(Arc::clone(&recent))
///     .build();
/// log_io::info!(logger, "one");
/// log_io::info!(logger, "two");
/// log_io::info!(logger, "three");
/// let lines = recent.snapshot();
/// assert_eq!(lines.len(), 2);
/// assert!(lines[0].contains("two") && lines[1].contains("three"));
/// ```
pub struct RecentLines<F: Format> {
    format: F,
    line_limit: usize,
    ring: Mutex<Ring>,
}

struct Ring {
    slots: Vec<String>,
    /// Index of the slot the next line goes into.
    next: usize,
    /// Number of slots holding a line.
    len: usize,
}

impl<F: Format> RecentLines<F> {
    /// A ring of `capacity` lines of at most [`DEFAULT_LINE_LIMIT`] bytes.
    /// A capacity of `0` keeps nothing.
    pub fn new(capacity: usize, format: F) -> Self {
        Self::with_line_limit(capacity, DEFAULT_LINE_LIMIT, format)
    }

    /// A ring of `capacity` lines of at most `line_limit` bytes each.
    pub fn with_line_limit(capacity: usize, line_limit: usize, format: F) -> Self {
        let slots = (0..capacity)
            .map(|_| String::with_capacity(line_limit))
            .collect();
        RecentLines {
            format,
            line_limit,
            ring: Mutex::new(Ring {
                slots,
                next: 0,
                len: 0,
            }),
        }
    }

    /// The number of lines the ring keeps.
    pub fn capacity(&self) -> usize {
        lock_recover(&self.ring).slots.len()
    }

    /// The number of lines held now.
    pub fn len(&self) -> usize {
        lock_recover(&self.ring).len
    }

    /// Whether no line has been written yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The lines held, oldest first, without their line endings.
    pub fn snapshot(&self) -> Vec<String> {
        lock_recover(&self.ring).lines()
    }

    /// Like [`snapshot`](Self::snapshot), but returns `None` instead of
    /// waiting when another thread (or this one) holds the ring.
    pub fn try_snapshot(&self) -> Option<Vec<String>> {
        match self.ring.try_lock() {
            Ok(ring) => Some(ring.lines()),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner().lines()),
            Err(TryLockError::WouldBlock) => None,
        }
    }

    /// Writes the lines held, oldest first, one per line.
    ///
    /// # Errors
    ///
    /// Returns the error `out` returns.
    pub fn write_to(&self, out: &mut impl fmt::Write) -> fmt::Result {
        lock_recover(&self.ring).write_to(out)
    }

    /// Like [`write_to`](Self::write_to), but writes nothing and returns
    /// `Ok(false)` instead of waiting when the ring is held. Returns
    /// `Ok(true)` when the lines were written.
    ///
    /// # Errors
    ///
    /// Returns the error `out` returns.
    pub fn try_write_to(
        &self,
        out: &mut impl fmt::Write,
    ) -> core::result::Result<bool, fmt::Error> {
        match self.ring.try_lock() {
            Ok(ring) => ring.write_to(out).map(|()| true),
            Err(TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().write_to(out).map(|()| true)
            }
            Err(TryLockError::WouldBlock) => Ok(false),
        }
    }

    /// Forgets every line held. The slots stay allocated.
    pub fn clear(&self) {
        let mut ring = lock_recover(&self.ring);
        for slot in &mut ring.slots {
            slot.clear();
        }
        ring.next = 0;
        ring.len = 0;
    }
}

impl Ring {
    fn push(&mut self, line: &str) {
        let capacity = self.slots.len();
        if capacity == 0 {
            return;
        }
        if let Some(slot) = self.slots.get_mut(self.next) {
            slot.clear();
            slot.push_str(line);
        }
        self.next = (self.next + 1) % capacity;
        self.len = (self.len + 1).min(capacity);
    }

    /// The slots holding lines, oldest first.
    fn ordered(&self) -> impl Iterator<Item = &String> {
        let capacity = self.slots.len();
        let start = if self.len < capacity { 0 } else { self.next };
        (0..self.len).filter_map(move |i| self.slots.get((start + i) % capacity.max(1)))
    }

    fn lines(&self) -> Vec<String> {
        self.ordered().cloned().collect()
    }

    fn write_to(&self, out: &mut impl fmt::Write) -> fmt::Result {
        for line in self.ordered() {
            out.write_str(line)?;
            out.write_char('\n')?;
        }
        Ok(())
    }
}

/// The longest prefix of `text` that fits in `limit` bytes without
/// splitting a character.
fn fit(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

impl<F: Format> Sink for RecentLines<F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        with_formatted(&self.format, record, |text| {
            let line = text.strip_suffix('\n').unwrap_or(text);
            let line = line.strip_suffix('\r').unwrap_or(line);
            lock_recover(&self.ring).push(fit(line, self.line_limit));
            Ok(())
        })
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }
}

impl<F: Format + fmt::Debug> fmt::Debug for RecentLines<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("RecentLines")
            .field("format", &self.format)
            .field("capacity", &ring.slots.len())
            .field("len", &ring.len)
            .field("line_limit", &self.line_limit)
            .finish_non_exhaustive()
    }
}

#[cfg(all(test, feature = "logfmt"))]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::format::LogfmtFormat;
    use crate::{Level, Logger, Metadata};

    fn record(msg: &str) -> Record<'_> {
        Record::new(Metadata::new(Level::Info, "t"), msg, &[])
    }

    #[test]
    fn test_recent_lines_keeps_the_last_n_oldest_first() {
        let sink = RecentLines::new(3, LogfmtFormat::new());
        for msg in ["a", "b", "c", "d", "e"] {
            assert!(sink.write_record(&record(msg)).is_ok());
        }
        let lines = sink.snapshot();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("message=c"), "{lines:?}");
        assert!(lines[2].contains("message=e"), "{lines:?}");
        assert!(lines.iter().all(|l| !l.ends_with('\n')));
    }

    #[test]
    fn test_recent_lines_before_wrap_keeps_order() {
        let sink = RecentLines::new(4, LogfmtFormat::new());
        assert!(sink.write_record(&record("first")).is_ok());
        assert!(sink.write_record(&record("second")).is_ok());
        let lines = sink.snapshot();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("first") && lines[1].contains("second"));
    }

    #[test]
    fn test_recent_lines_zero_capacity_keeps_nothing() {
        let sink = RecentLines::new(0, LogfmtFormat::new());
        assert!(sink.write_record(&record("x")).is_ok());
        assert!(sink.is_empty());
        assert!(sink.snapshot().is_empty());
    }

    #[test]
    fn test_recent_lines_cuts_long_lines_on_a_char_boundary() {
        let sink = RecentLines::with_line_limit(1, 12, LogfmtFormat::new());
        assert!(sink.write_record(&record("ééééééééééé")).is_ok());
        let line = &sink.snapshot()[0];
        assert!(line.len() <= 12, "{line}");
        assert!(std::str::from_utf8(line.as_bytes()).is_ok());
    }

    #[test]
    fn test_fit_never_splits_a_character() {
        assert_eq!(fit("abc", 10), "abc");
        assert_eq!(fit("abcdef", 3), "abc");
        assert_eq!(fit("é", 1), "");
        assert_eq!(fit("aé", 2), "a");
    }

    #[test]
    fn test_recent_lines_try_snapshot_does_not_block_while_held() {
        let sink = RecentLines::new(2, LogfmtFormat::new());
        assert!(sink.write_record(&record("x")).is_ok());
        let held = sink.ring.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(sink.try_snapshot(), None);
        let mut out = String::new();
        assert_eq!(sink.try_write_to(&mut out), Ok(false));
        assert!(out.is_empty());
        drop(held);
        assert_eq!(sink.try_snapshot().map(|l| l.len()), Some(1));
    }

    #[test]
    fn test_recent_lines_write_to_and_clear() {
        let sink = RecentLines::new(2, LogfmtFormat::new());
        assert!(sink.write_record(&record("one")).is_ok());
        assert!(sink.write_record(&record("two")).is_ok());
        let mut out = String::new();
        assert!(sink.write_to(&mut out).is_ok());
        assert_eq!(out.lines().count(), 2);
        assert!(out.ends_with('\n'));
        sink.clear();
        assert!(sink.is_empty());
        assert_eq!(sink.capacity(), 2);
    }

    #[test]
    fn test_recent_lines_hold_redacted_values() {
        let recent = Arc::new(RecentLines::new(4, LogfmtFormat::new()));
        let logger = Logger::builder()
            .redact_common_secrets()
            .with_sink(Arc::clone(&recent))
            .build();
        crate::info!(logger, "login", password = "hunter2");
        let lines = recent.snapshot();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains("hunter2"), "{lines:?}");
        assert!(lines[0].contains(crate::REDACTED), "{lines:?}");
    }

    #[test]
    fn test_recent_lines_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RecentLines<LogfmtFormat>>();
    }
}

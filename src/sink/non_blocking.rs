//! Background-thread sink with a bounded queue.

use std::io::{self, BufWriter, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;

use super::{Sink, lock_recover, trim_buffer};
use crate::error::{Error, Result};
use crate::format::Format;
use crate::record::Record;

/// What [`NonBlocking`] does when its queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Overflow {
    /// Wait for space. No record is ever dropped; under sustained
    /// overload the logging threads slow down to the writer's pace.
    #[default]
    Block,
    /// Drop the record that did not fit and count it (see
    /// [`NonBlocking::dropped`]). Logging never waits on the writer.
    DropNewest,
}

/// Handler for write errors that happen on the background thread.
pub type WorkerErrorHandler = Arc<dyn Fn(&io::Error) + Send + Sync>;

enum Message {
    Record(String),
    Flush(SyncSender<io::Result<()>>),
    Shutdown,
}

struct Shared {
    dropped: AtomicU64,
    write_errors: AtomicU64,
    /// First write error since the last flush; reported by `flush`.
    deferred: Mutex<Option<io::Error>>,
    on_error: Option<WorkerErrorHandler>,
}

/// State guarded by the shutdown lock. Producers hold the read side
/// while enqueueing, so no record can be queued after the shutdown
/// message.
struct Channel {
    sender: Option<SyncSender<Message>>,
    worker: Option<JoinHandle<Box<dyn Write + Send>>>,
}

/// A sink that formats on the calling thread and writes on a
/// background thread.
///
/// Records are formatted into a `String`, queued on a bounded channel,
/// and written by a dedicated thread through an internal write buffer.
/// The worker flushes whenever the queue drains, so records reach the
/// OS promptly once logging pauses, and in large batches under load.
/// This keeps file and socket I/O off request threads and async
/// executor threads.
///
/// * **Bounded:** at most `capacity` records are queued. What happens
///   when the queue is full is set by [`Overflow`]; the default,
///   [`Overflow::Block`], never drops a record.
/// * **Flush:** [`Sink::flush`] waits until every record queued before
///   it has been written and the writer flushed.
/// * **Shutdown:** [`Sink::shutdown`] (called by
///   [`crate::Logger::shutdown`], a [`crate::FlushGuard`], or dropping
///   the last logger clone) drains the queue, flushes, and joins the
///   thread. Records logged afterwards are written synchronously on the
///   caller's thread, so late records are not lost.
/// * **Errors:** a write error on the worker cannot be returned to the
///   caller that logged the record. It is counted
///   ([`NonBlocking::write_errors`]), passed to the optional
///   [`NonBlockingBuilder::on_error`] handler, and returned by the next
///   `flush`.
/// * **Allocation:** queued strings are recycled through a return
///   channel, so a single producer allocates only while the pool warms
///   up; with heavy multi-thread contention some records allocate.
///
/// As with any buffering sink, records still queued when the process
/// aborts are lost; see [`super::FileSink`] for the exit paths that
/// skip destructors.
///
/// The writer (and the worker's `on_error` handler) must not log
/// through a logger that feeds this same sink: with
/// [`Overflow::Block`], a full queue would make the worker wait for
/// itself.
///
/// # Examples
///
#[cfg_attr(all(feature = "json"), doc = "```")]
#[cfg_attr(not(all(feature = "json")), doc = "```ignore")]
/// use log_io::format::JsonFormat;
/// use log_io::sink::{NonBlocking, Overflow};
/// use log_io::{Level, Logger};
///
/// let sink = NonBlocking::builder(std::io::sink(), JsonFormat::new())
///     .capacity(4096)
///     .overflow(Overflow::DropNewest)
///     .build()?;
/// let logger = Logger::builder().level(Level::Info).with_sink(sink).build();
/// log_io::info!(logger, "off the hot path", worker = 3_u32);
/// logger.shutdown()?; // drain and join
/// # Ok::<(), log_io::Error>(())
/// ```
pub struct NonBlocking<F: Format> {
    format: F,
    overflow: Overflow,
    channel: RwLock<Channel>,
    /// The writer handed back by the worker at shutdown. Records that
    /// arrive later are written to it synchronously. Set while the
    /// channel's write lock is held, so any producer that observes a
    /// closed channel also observes the fallback.
    fallback: Mutex<Option<Box<dyn Write + Send>>>,
    pool: Mutex<Receiver<String>>,
    shared: Arc<Shared>,
}

impl<F: Format> NonBlocking<F> {
    /// Start a background writer with default options: capacity 8192,
    /// [`Overflow::Block`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the worker thread cannot be spawned.
    pub fn new<W: Write + Send + 'static>(writer: W, format: F) -> Result<Self> {
        Self::builder(writer, format).build()
    }

    /// Start configuring a background writer.
    pub fn builder<W: Write + Send + 'static>(writer: W, format: F) -> NonBlockingBuilder<F> {
        NonBlockingBuilder {
            writer: Box::new(writer),
            format,
            capacity: 8192,
            overflow: Overflow::Block,
            buffer_capacity: 64 * 1024,
            thread_name: String::from("log-io-writer"),
            on_error: None,
        }
    }

    /// Records dropped because the queue was full under
    /// [`Overflow::DropNewest`].
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }

    /// Write errors encountered by the background thread.
    #[must_use]
    pub fn write_errors(&self) -> u64 {
        self.shared.write_errors.load(Ordering::Relaxed)
    }

    fn take_buffer(&self) -> String {
        // `try_lock`: under contention, allocating is cheaper than
        // queueing on the pool lock.
        self.pool
            .try_lock()
            .ok()
            .and_then(|rx| rx.try_recv().ok())
            .unwrap_or_default()
    }

    fn take_deferred(&self) -> Result<()> {
        match lock_recover(&self.shared.deferred).take() {
            Some(e) => Err(Error::Io(e)),
            None => Ok(()),
        }
    }
}

impl<F: Format> Sink for NonBlocking<F> {
    fn write_record(&self, record: &Record<'_>) -> Result<()> {
        let mut text = self.take_buffer();
        text.clear();
        self.format
            .write_record_generic(record, &mut text)
            .map_err(Error::Format)?;

        let channel = self
            .channel
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(sender) = &channel.sender {
            return match self.overflow {
                Overflow::Block => sender
                    .send(Message::Record(text))
                    .map_err(|_| Error::Closed),
                Overflow::DropNewest => match sender.try_send(Message::Record(text)) {
                    Ok(()) => Ok(()),
                    Err(TrySendError::Full(_)) => {
                        let _previous = self.shared.dropped.fetch_add(1, Ordering::Relaxed);
                        Ok(())
                    }
                    Err(TrySendError::Disconnected(_)) => Err(Error::Closed),
                },
            };
        }
        drop(channel);
        // Shut down: write synchronously so late records are kept.
        match lock_recover(&self.fallback).as_mut() {
            Some(writer) => writer.write_all(text.as_bytes()).map_err(Error::from),
            None => Err(Error::Closed),
        }
    }

    fn flush(&self) -> Result<()> {
        let channel = self
            .channel
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(sender) = &channel.sender {
            let (ack_tx, ack_rx) = mpsc::sync_channel(1);
            sender
                .send(Message::Flush(ack_tx))
                .map_err(|_| Error::Closed)?;
            drop(channel);
            let flushed = ack_rx.recv().map_err(|_| Error::Closed)?;
            self.take_deferred()?;
            return flushed.map_err(Error::from);
        }
        drop(channel);
        if let Some(writer) = lock_recover(&self.fallback).as_mut() {
            writer.flush()?;
        }
        self.take_deferred()
    }

    fn shutdown(&self) -> Result<()> {
        let mut channel = self
            .channel
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(sender) = channel.sender.take() else {
            drop(channel);
            return self.flush();
        };
        // Producers enqueue while holding the read lock, so nothing can
        // be queued behind this message.
        let sent = sender.send(Message::Shutdown);
        drop(sender);
        let joined = channel.worker.take().map(JoinHandle::join);
        match joined {
            Some(Ok(writer)) => *lock_recover(&self.fallback) = Some(writer),
            Some(Err(_)) => return Err(Error::Closed),
            None => {}
        }
        drop(channel);
        sent.map_err(|_| Error::Closed)?;
        self.take_deferred()
    }
}

impl<F: Format> Drop for NonBlocking<F> {
    fn drop(&mut self) {
        // The logger shuts its sinks down (and reports errors) before
        // dropping them; this covers sinks used without a logger.
        let _shutdown_in_drop = self.shutdown();
    }
}

impl<F: Format + core::fmt::Debug> core::fmt::Debug for NonBlocking<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NonBlocking")
            .field("format", &self.format)
            .field("overflow", &self.overflow)
            .field("dropped", &self.dropped())
            .field("write_errors", &self.write_errors())
            .finish_non_exhaustive()
    }
}

/// Options for a [`NonBlocking`] sink. Create with
/// [`NonBlocking::builder`].
#[must_use = "call `build()` to start the background writer"]
pub struct NonBlockingBuilder<F> {
    writer: Box<dyn Write + Send>,
    format: F,
    capacity: usize,
    overflow: Overflow,
    buffer_capacity: usize,
    thread_name: String,
    on_error: Option<WorkerErrorHandler>,
}

impl<F: Format> NonBlockingBuilder<F> {
    /// Maximum number of queued records (minimum 1). Default 8192.
    pub fn capacity(mut self, records: usize) -> Self {
        self.capacity = records.max(1);
        self
    }

    /// Behaviour when the queue is full. Default [`Overflow::Block`].
    pub fn overflow(mut self, overflow: Overflow) -> Self {
        self.overflow = overflow;
        self
    }

    /// Size of the worker's write buffer in bytes (`0` = unbuffered).
    /// Default 64 KiB.
    pub fn buffer_capacity(mut self, bytes: usize) -> Self {
        self.buffer_capacity = bytes;
        self
    }

    /// Name of the background thread. Default `log-io-writer`.
    pub fn thread_name(mut self, name: impl Into<String>) -> Self {
        self.thread_name = name.into();
        self
    }

    /// Called on the background thread for every write error.
    pub fn on_error(mut self, handler: WorkerErrorHandler) -> Self {
        self.on_error = Some(handler);
        self
    }

    /// Spawn the background thread and build the sink.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the thread cannot be spawned.
    pub fn build(self) -> Result<NonBlocking<F>> {
        let (tx, rx) = mpsc::sync_channel::<Message>(self.capacity);
        let (pool_tx, pool_rx) = mpsc::sync_channel::<String>(self.capacity);
        let shared = Arc::new(Shared {
            dropped: AtomicU64::new(0),
            write_errors: AtomicU64::new(0),
            deferred: Mutex::new(None),
            on_error: self.on_error,
        });
        let worker_shared = Arc::clone(&shared);
        let writer = BufWriter::with_capacity(self.buffer_capacity, self.writer);
        let worker = std::thread::Builder::new()
            .name(self.thread_name)
            .spawn(move || run_worker(writer, &rx, &pool_tx, &worker_shared))?;
        Ok(NonBlocking {
            format: self.format,
            overflow: self.overflow,
            channel: RwLock::new(Channel {
                sender: Some(tx),
                worker: Some(worker),
            }),
            fallback: Mutex::new(None),
            pool: Mutex::new(pool_rx),
            shared,
        })
    }
}

impl<F: core::fmt::Debug> core::fmt::Debug for NonBlockingBuilder<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NonBlockingBuilder")
            .field("format", &self.format)
            .field("capacity", &self.capacity)
            .field("overflow", &self.overflow)
            .field("thread_name", &self.thread_name)
            .finish_non_exhaustive()
    }
}

fn report(shared: &Shared, error: io::Error) {
    let _previous = shared.write_errors.fetch_add(1, Ordering::Relaxed);
    if let Some(handler) = &shared.on_error {
        handler(&error);
    }
    let mut deferred = lock_recover(&shared.deferred);
    if deferred.is_none() {
        *deferred = Some(error);
    }
}

fn run_worker(
    mut writer: BufWriter<Box<dyn Write + Send>>,
    rx: &Receiver<Message>,
    pool: &SyncSender<String>,
    shared: &Shared,
) -> Box<dyn Write + Send> {
    let mut dirty = false;
    loop {
        let message = match rx.try_recv() {
            Ok(m) => m,
            Err(TryRecvError::Empty) => {
                // Queue drained: push what we have to the OS before
                // sleeping, so records are not held back while idle.
                if dirty {
                    if let Err(e) = writer.flush() {
                        report(shared, e);
                    }
                }
                match rx.recv() {
                    Ok(m) => m,
                    Err(_) => break,
                }
            }
            Err(TryRecvError::Disconnected) => break,
        };
        match message {
            Message::Record(mut text) => {
                if let Err(e) = writer.write_all(text.as_bytes()) {
                    report(shared, e);
                }
                dirty = true;
                trim_buffer(&mut text);
                // Recycle the buffer; if the pool is full it is freed.
                let _recycled = pool.try_send(text);
            }
            Message::Flush(ack) => {
                dirty = false;
                let _requester_may_be_gone = ack.send(writer.flush());
            }
            Message::Shutdown => break,
        }
    }
    if let Err(e) = writer.flush() {
        report(shared, e);
    }
    match writer.into_inner() {
        Ok(inner) => inner,
        Err(e) => {
            let (error, writer) = e.into_parts();
            report(shared, error);
            // Hand back the unflushed buffer too; the synchronous
            // fallback path retries through it.
            Box::new(writer)
        }
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;
    use crate::format::JsonFormat;
    use crate::{Level, Metadata};
    use std::sync::Barrier;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Capture {
        fn lines(&self) -> usize {
            String::from_utf8(self.0.lock().unwrap().clone())
                .unwrap()
                .lines()
                .count()
        }
    }

    fn record(msg: &str) -> Record<'_> {
        Record::new(Metadata::new(Level::Info, "t"), msg, &[])
    }

    #[test]
    fn test_non_blocking_flush_waits_for_queued_records() {
        let out = Capture::default();
        let sink = NonBlocking::new(out.clone(), JsonFormat::new()).unwrap();
        for _ in 0..1_000 {
            sink.write_record(&record("x")).unwrap();
        }
        sink.flush().unwrap();
        assert_eq!(out.lines(), 1_000);
    }

    #[test]
    fn test_non_blocking_shutdown_drains_then_writes_synchronously() {
        let out = Capture::default();
        let sink = NonBlocking::new(out.clone(), JsonFormat::new()).unwrap();
        for _ in 0..100 {
            sink.write_record(&record("queued")).unwrap();
        }
        sink.shutdown().unwrap();
        assert_eq!(out.lines(), 100);
        // Late record after shutdown: written inline, not lost.
        sink.write_record(&record("late")).unwrap();
        sink.flush().unwrap();
        sink.shutdown().unwrap();
        assert_eq!(out.lines(), 101);
    }

    #[test]
    fn test_non_blocking_drop_newest_counts_drops() {
        // A writer that blocks until released, so the queue fills.
        struct Gate(Arc<Barrier>, Capture, bool);
        impl Write for Gate {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                if !self.2 {
                    self.2 = true;
                    let _leader = self.0.wait();
                }
                self.1.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let gate = Arc::new(Barrier::new(2));
        let out = Capture::default();
        let sink = NonBlocking::builder(Gate(gate.clone(), out.clone(), false), JsonFormat::new())
            .capacity(2)
            .overflow(Overflow::DropNewest)
            .buffer_capacity(0)
            .build()
            .unwrap();
        for _ in 0..50 {
            sink.write_record(&record("burst")).unwrap();
        }
        let _leader = gate.wait();
        sink.flush().unwrap();
        let written = out.lines() as u64;
        assert!(sink.dropped() > 0, "the queue must have overflowed");
        assert_eq!(
            written + sink.dropped(),
            50,
            "every record is written or counted"
        );
    }

    #[test]
    fn test_non_blocking_write_errors_are_counted_and_deferred_to_flush() {
        struct Failing;
        impl Write for Failing {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let seen = Arc::new(AtomicU64::new(0));
        let seen2 = Arc::clone(&seen);
        let sink = NonBlocking::builder(Failing, JsonFormat::new())
            .buffer_capacity(0)
            .thread_name("log-io-test-writer")
            .on_error(Arc::new(move |_| {
                let _n = seen2.fetch_add(1, Ordering::Relaxed);
            }))
            .build()
            .unwrap();
        sink.write_record(&record("lost")).unwrap();
        let err = sink.flush().unwrap_err();
        assert!(matches!(err, Error::Io(_)), "{err}");
        assert_eq!(sink.write_errors(), 1);
        assert_eq!(seen.load(Ordering::Relaxed), 1);
        assert!(sink.flush().is_ok(), "the deferred error is reported once");
        assert!(format!("{sink:?}").contains("write_errors: 1"));
    }

    #[test]
    fn test_non_blocking_many_producers_lose_nothing_with_block() {
        let out = Capture::default();
        let sink = Arc::new(
            NonBlocking::builder(out.clone(), JsonFormat::new())
                .capacity(16)
                .build()
                .unwrap(),
        );
        std::thread::scope(|s| {
            for _ in 0..8 {
                let sink = Arc::clone(&sink);
                let _handle = s.spawn(move || {
                    for _ in 0..500 {
                        sink.write_record(&record("p")).unwrap();
                    }
                });
            }
        });
        sink.shutdown().unwrap();
        assert_eq!(out.lines(), 8 * 500);
        assert_eq!(sink.dropped(), 0);
    }

    #[test]
    fn test_non_blocking_worker_panic_reports_closed() {
        struct Panics;
        impl Write for Panics {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                panic!("writer exploded");
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let sink = NonBlocking::builder(Panics, JsonFormat::new())
            .buffer_capacity(0)
            .build()
            .unwrap();
        let _queued = sink.write_record(&record("boom"));
        assert!(matches!(sink.shutdown(), Err(Error::Closed)));
        assert!(matches!(
            sink.write_record(&record("after")),
            Err(Error::Closed)
        ));
        let builder = NonBlocking::builder(Vec::<u8>::new(), JsonFormat::new()).capacity(0);
        assert!(format!("{builder:?}").contains("capacity: 1"));
    }
}

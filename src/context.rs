//! Thread-local logging context.
//!
//! Context carries ambient key-value data attached to every record
//! emitted on the current thread: stash a `trace_id` or `request_id`
//! at the start of a request and every log line below it carries the
//! value. [`with_field`] (and friends) push a value and return a
//! [`ContextGuard`] that removes it when dropped.
//!
//! # Rules
//!
//! * **Shadowing.** When two active entries share a key, the most
//!   recent one wins and the record carries the key once.
//! * **Capacity.** A snapshot carries at most [`MAX_CONTEXT_SLOTS`]
//!   fields (the most recent ones) and the thread keeps at most that
//!   many entries; pushing beyond it evicts the oldest entry, whose
//!   guard then becomes a no-op.
//! * **Reentrancy.** Sinks and formats may log, push context, or drop
//!   guards while a record is being dispatched. Changes made during
//!   dispatch take effect when the outermost dispatch on the thread
//!   finishes; a record emitted from inside a sink sees the context
//!   that was active when the outer record started.
//! * **Thread teardown.** Guards dropped while thread-local storage is
//!   being destroyed do nothing instead of aborting the process.
//!
//! # Async tasks
//!
//! Context is per thread, and a task in a work-stealing runtime can
//! resume on a different thread after every `.await`. That is why
//! [`ContextGuard`] is neither `Send` nor `Sync`: a guard held across
//! an `.await` makes the future `!Send`, so `tokio::spawn` and similar
//! reject it at compile time instead of silently attaching one
//! request's fields to another request's records.
//!
//! For tasks, capture the context as a [`ContextSnapshot`] (an owned,
//! `Send + Sync`, cheaply clonable value) and re-install it where the
//! work runs:
//!
//! ```
//! use log_io::context;
//!
//! let _req = context::with_request_id("req-7");
//! let snapshot = context::capture();
//!
//! std::thread::spawn(move || {
//!     // Re-install on the worker thread for the duration of a closure.
//!     snapshot.scope(|| {
//!         assert_eq!(context::len(), 1);
//!     });
//! })
//! .join()
//! .unwrap();
//! ```
//!
//! For futures, [`ContextSnapshot::wrap`] re-installs the snapshot
//! around every `poll`, which works with any executor:
//!
//! ```ignore
//! let snapshot = log_io::context::capture();
//! tokio::spawn(snapshot.wrap(async move {
//!     handle_request().await; // every record carries the captured fields
//! }));
//! ```

use core::cell::{Cell, RefCell};
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context as TaskContext, Poll};
use std::sync::Arc;

use crate::record::Field;
use crate::value::{OwnedField, Value};

/// Maximum number of context fields attached to one record, and the
/// maximum number of live context entries per thread.
pub const MAX_CONTEXT_SLOTS: usize = 16;

enum Entry {
    Field { id: u64, field: OwnedField },
    Snapshot { id: u64, fields: Arc<[OwnedField]> },
}

impl Entry {
    const fn id(&self) -> u64 {
        match self {
            Self::Field { id, .. } | Self::Snapshot { id, .. } => *id,
        }
    }
}

enum Pending {
    Push(Entry),
    Remove(u64),
    Clear,
}

struct ThreadContext {
    entries: RefCell<Vec<Entry>>,
    /// Changes requested while `entries` was borrowed by a snapshot.
    pending: RefCell<Vec<Pending>>,
    has_pending: Cell<bool>,
    next_id: Cell<u64>,
}

impl ThreadContext {
    const fn new() -> Self {
        Self {
            entries: RefCell::new(Vec::new()),
            pending: RefCell::new(Vec::new()),
            has_pending: Cell::new(false),
            next_id: Cell::new(1),
        }
    }

    fn next_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id.wrapping_add(1).max(1));
        id
    }

    /// Apply `op` now if possible, otherwise queue it until the current
    /// snapshot borrow ends.
    fn apply_or_defer(&self, op: Pending) {
        match self.entries.try_borrow_mut() {
            Ok(mut entries) => apply(&mut entries, op),
            Err(_) => {
                if let Ok(mut pending) = self.pending.try_borrow_mut() {
                    pending.push(op);
                    self.has_pending.set(true);
                }
            }
        }
    }

    fn apply_pending(&self) {
        if !self.has_pending.get() {
            return;
        }
        let Ok(mut entries) = self.entries.try_borrow_mut() else {
            // An outer snapshot is still active; it applies them later.
            return;
        };
        let Ok(mut pending) = self.pending.try_borrow_mut() else {
            return;
        };
        for op in pending.drain(..) {
            apply(&mut entries, op);
        }
        self.has_pending.set(false);
    }
}

fn apply(entries: &mut Vec<Entry>, op: Pending) {
    match op {
        Pending::Push(entry) => {
            if entries.len() >= MAX_CONTEXT_SLOTS {
                drop(entries.remove(0));
            }
            entries.push(entry);
        }
        Pending::Remove(id) => {
            if let Some(pos) = entries.iter().rposition(|e| e.id() == id) {
                drop(entries.remove(pos));
            }
        }
        Pending::Clear => entries.clear(),
    }
}

thread_local! {
    static CONTEXT: ThreadContext = const { ThreadContext::new() };
}

/// RAII guard that removes a context entry when dropped.
///
/// Returned by [`with_field`], [`with_trace_id`], [`with_request_id`]
/// and [`ContextSnapshot::attach`]. Guards may be dropped in any order.
///
/// The guard is deliberately `!Send` and `!Sync`: context lives in the
/// thread that created the guard, so the guard must be dropped there.
/// Holding one across an `.await` makes the future `!Send`. Use
/// [`ContextSnapshot`] to carry context into other threads or tasks.
///
/// ```compile_fail
/// let guard = log_io::context::with_field("tenant", "a");
/// std::thread::spawn(move || drop(guard)); // error: `ContextGuard` is not `Send`
/// ```
#[must_use = "context is removed when this guard is dropped; bind it to a variable"]
#[derive(Debug)]
pub struct ContextGuard {
    /// `0` means the entry was never installed (thread-local storage
    /// was unavailable), so dropping does nothing.
    id: u64,
    _not_send: PhantomData<*const ()>,
}

impl ContextGuard {
    const fn new(id: u64) -> Self {
        Self {
            id,
            _not_send: PhantomData,
        }
    }
}

impl Drop for ContextGuard {
    fn drop(&mut self) {
        if self.id == 0 {
            return;
        }
        let id = self.id;
        // `try_with`: during thread-local teardown there is nothing to
        // remove, and panicking in a destructor there aborts.
        let _tls_may_be_gone = CONTEXT.try_with(|ctx| ctx.apply_or_defer(Pending::Remove(id)));
    }
}

fn push(make: impl FnOnce(u64) -> Entry) -> ContextGuard {
    CONTEXT
        .try_with(|ctx| {
            let id = ctx.next_id();
            ctx.apply_or_defer(Pending::Push(make(id)));
            ContextGuard::new(id)
        })
        .unwrap_or(ContextGuard::new(0))
}

/// Push a key/value pair onto the current thread's context.
///
/// The value is copied into owned storage (lazy `Display` / `Debug` /
/// `Error` values are rendered now), so `with_field` allocates; the
/// per-record cost of attaching context does not.
///
/// # Examples
///
/// ```
/// use log_io::context;
///
/// {
///     let _user = context::with_field("user_id", 42_u64);
///     assert_eq!(context::len(), 1);
/// }
/// assert_eq!(context::len(), 0);
/// ```
pub fn with_field<'a, V: Into<Value<'a>>>(key: &str, value: V) -> ContextGuard {
    let field = OwnedField::new(key, value.into());
    push(move |id| Entry::Field { id, field })
}

/// Push a `trace_id` field. See [`with_field`].
pub fn with_trace_id(id: &str) -> ContextGuard {
    with_field("trace_id", Value::Str(id))
}

/// Push a `request_id` field. See [`with_field`].
pub fn with_request_id(id: &str) -> ContextGuard {
    with_field("request_id", Value::Str(id))
}

/// Run `f` with the current thread's context as a borrowed slice of
/// [`Field`]s, oldest first, with shadowed keys removed.
///
/// No allocation: the slice lives on the stack. Context changes made
/// inside `f` (including from sinks) are applied when the outermost
/// snapshot on this thread ends.
///
/// # Examples
///
/// ```
/// use log_io::{context, Value};
///
/// let _a = context::with_field("user", "outer");
/// let _b = context::with_field("user", "inner");
/// context::with_snapshot(|fields| {
///     assert_eq!(fields.len(), 1);
///     assert_eq!(fields[0].value, Value::Str("inner"));
/// });
/// ```
pub fn with_snapshot<R>(f: impl FnOnce(&[Field<'_>]) -> R) -> R {
    // During thread-local teardown the context is gone: no fields.
    // Probing first keeps `f` in this frame, and nothing between the
    // probe and `with` can run this thread's TLS destructors, so `with`
    // cannot hit its "destroyed" panic path.
    if CONTEXT.try_with(|_| ()).is_err() {
        return f(&[]);
    }
    CONTEXT.with(|ctx| {
        let out = match ctx.entries.try_borrow() {
            Ok(entries) => {
                let mut buf = [EMPTY; MAX_CONTEXT_SLOTS];
                let len = materialize(&entries, &mut buf);
                f(&buf[..len])
            }
            // Only possible while a mutation is in progress, which never
            // spans user code; treat as empty rather than panic.
            Err(_) => f(&[]),
        };
        ctx.apply_pending();
        out
    })
}

const EMPTY: Field<'static> = Field {
    key: "",
    value: Value::Null,
};

/// Fill `buf` with the visible fields (newest per key, at most
/// `MAX_CONTEXT_SLOTS`, the most recent ones), oldest first.
fn materialize<'e>(entries: &'e [Entry], buf: &mut [Field<'e>; MAX_CONTEXT_SLOTS]) -> usize {
    let mut len = 0;
    'entries: for entry in entries.iter().rev() {
        let fields: &[OwnedField] = match entry {
            Entry::Field { field, .. } => core::slice::from_ref(field),
            Entry::Snapshot { fields, .. } => fields,
        };
        for owned in fields.iter().rev() {
            if len == MAX_CONTEXT_SLOTS {
                break 'entries;
            }
            if buf[..len].iter().any(|f| f.key == &*owned.key) {
                continue;
            }
            buf[len] = owned.as_field();
            len += 1;
        }
    }
    buf[..len].reverse();
    len
}

/// Remove every context entry on the current thread. Existing guards
/// become no-ops.
///
/// Intended for tests and for resetting pooled threads; normal code
/// relies on [`ContextGuard`] drops.
pub fn clear() {
    let _tls_may_be_gone = CONTEXT.try_with(|ctx| ctx.apply_or_defer(Pending::Clear));
}

/// Number of context fields a record emitted now would carry (after
/// shadowing and the [`MAX_CONTEXT_SLOTS`] cap).
#[must_use]
pub fn len() -> usize {
    with_snapshot(|fields| fields.len())
}

/// Capture the current thread's context as an owned snapshot.
///
/// Allocates once (an `Arc`). Cloning the snapshot is a reference-count
/// increment.
///
/// # Examples
///
/// ```
/// use log_io::context;
///
/// let snapshot = {
///     let _t = context::with_trace_id("abc");
///     context::capture()
/// };
/// assert_eq!(context::len(), 0);
/// let _restored = snapshot.attach();
/// assert_eq!(context::len(), 1);
/// ```
#[must_use]
pub fn capture() -> ContextSnapshot {
    with_snapshot(ContextSnapshot::from_fields)
}

/// An owned, immutable copy of a set of context fields.
///
/// `Send + Sync` and cheap to clone, so it can be moved into threads,
/// tasks, and closures. Install it on the current thread with
/// [`ContextSnapshot::attach`], [`ContextSnapshot::scope`] or
/// [`ContextSnapshot::wrap`]. An attached snapshot behaves like a
/// group of [`with_field`] entries: fields pushed later shadow it, and
/// it shadows fields pushed earlier.
#[derive(Clone, Default)]
pub struct ContextSnapshot {
    fields: Option<Arc<[OwnedField]>>,
}

impl ContextSnapshot {
    /// Build a snapshot from explicit fields, without touching the
    /// thread's context. Later fields shadow earlier ones with the same
    /// key.
    ///
    /// # Examples
    ///
    /// ```
    /// use log_io::context::ContextSnapshot;
    /// use log_io::Field;
    ///
    /// let snap = ContextSnapshot::from_fields(&[Field::new("tenant", "acme")]);
    /// assert_eq!(snap.len(), 1);
    /// ```
    #[must_use]
    pub fn from_fields(fields: &[Field<'_>]) -> Self {
        if fields.is_empty() {
            return Self::default();
        }
        let mut owned: Vec<OwnedField> = Vec::with_capacity(fields.len());
        for field in fields {
            if let Some(existing) = owned.iter_mut().find(|o| &*o.key == field.key) {
                *existing = OwnedField::new(field.key, field.value);
            } else {
                owned.push(OwnedField::new(field.key, field.value));
            }
        }
        Self {
            fields: Some(owned.into()),
        }
    }

    /// Number of fields in the snapshot.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.as_deref().map_or(0, <[OwnedField]>::len)
    }

    /// `true` if the snapshot has no fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate over the snapshot's fields in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = Field<'_>> {
        self.fields
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(OwnedField::as_field)
    }

    /// Install the snapshot on the current thread until the returned
    /// guard is dropped. Does not allocate.
    pub fn attach(&self) -> ContextGuard {
        match &self.fields {
            Some(fields) => {
                let fields = Arc::clone(fields);
                push(move |id| Entry::Snapshot { id, fields })
            }
            None => ContextGuard::new(0),
        }
    }

    /// Run `f` with the snapshot installed on the current thread.
    pub fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.attach();
        f()
    }

    /// Wrap a future so the snapshot is installed around every poll.
    ///
    /// Works with any executor, including work-stealing ones: whichever
    /// thread polls the future sees the snapshot for exactly the
    /// duration of the poll. The future is boxed once (one allocation
    /// per wrapped future) so the wrapper can be `Unpin` without
    /// `unsafe` code.
    pub fn wrap<F: Future>(&self, future: F) -> WithContext<F> {
        WithContext {
            future: Box::pin(future),
            snapshot: self.clone(),
        }
    }
}

impl core::fmt::Debug for ContextSnapshot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map()
            .entries(self.iter().map(|field| (field.key, field.value)))
            .finish()
    }
}

/// A future with a [`ContextSnapshot`] installed around every poll.
/// Created by [`ContextSnapshot::wrap`].
#[must_use = "futures do nothing unless polled"]
pub struct WithContext<F> {
    future: Pin<Box<F>>,
    snapshot: ContextSnapshot,
}

impl<F: Future> Future for WithContext<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let _guard = this.snapshot.attach();
        this.future.as_mut().poll(cx)
    }
}

impl<F> core::fmt::Debug for WithContext<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WithContext")
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        with_snapshot(|f| f.iter().map(|f| f.key.to_owned()).collect())
    }

    #[test]
    fn test_context_is_isolated_per_thread() {
        clear();
        let _g = with_field("k", 1_u64);
        assert_eq!(len(), 1);
        std::thread::spawn(|| assert_eq!(len(), 0)).join().unwrap();
        clear();
    }

    #[test]
    fn test_context_guard_removes_on_drop() {
        clear();
        {
            let _g = with_field("k", 1_u64);
            assert_eq!(len(), 1);
        }
        assert_eq!(len(), 0);
    }

    #[test]
    fn test_context_nested_scopes_layer_and_unwind() {
        clear();
        let outer = with_field("outer", 1_u64);
        {
            let _inner = with_field("inner", 2_u64);
            assert_eq!(keys(), ["outer", "inner"]);
        }
        assert_eq!(keys(), ["outer"]);
        drop(outer);
        assert_eq!(len(), 0);
    }

    #[test]
    fn test_context_out_of_order_drop() {
        clear();
        let a = with_field("a", 1_u64);
        let b = with_field("b", 2_u64);
        drop(a);
        assert_eq!(keys(), ["b"]);
        drop(b);
        assert_eq!(len(), 0);
    }

    #[test]
    fn test_context_inner_scope_shadows_outer_key() {
        // The audit's R06 duplicate-key half.
        clear();
        let _a = with_field("user", "outer");
        let b = with_field("user", "inner");
        with_snapshot(|f| {
            assert_eq!(f.len(), 1);
            assert_eq!(f[0].value, Value::Str("inner"));
        });
        drop(b);
        with_snapshot(|f| assert_eq!(f[0].value, Value::Str("outer")));
        clear();
    }

    #[test]
    fn test_context_slot_overflow_evicts_oldest() {
        clear();
        let mut guards = Vec::new();
        for i in 0..(MAX_CONTEXT_SLOTS + 5) {
            guards.push(with_field(&format!("k{i}"), i as u64));
        }
        with_snapshot(|f| {
            assert_eq!(f.len(), MAX_CONTEXT_SLOTS);
            assert_eq!(f[0].key, "k5", "oldest entries are evicted");
            assert_eq!(f[MAX_CONTEXT_SLOTS - 1].key, "k20");
        });
        drop(guards);
        assert_eq!(len(), 0);
    }

    #[test]
    fn test_context_trace_and_request_id_helpers() {
        clear();
        let _t = with_trace_id("abc");
        let _r = with_request_id("xyz");
        with_snapshot(|f| {
            assert_eq!(f[0].key, "trace_id");
            assert_eq!(f[0].value, Value::Str("abc"));
            assert_eq!(f[1].key, "request_id");
            assert_eq!(f[1].value, Value::Str("xyz"));
        });
        clear();
    }

    #[test]
    fn test_context_with_field_inside_snapshot_is_deferred_not_panicking() {
        // The audit's R02 reproduction, at the module level.
        clear();
        let guard = with_snapshot(|_| with_field("x", 1_u32));
        assert_eq!(keys(), ["x"], "applied after the snapshot ended");
        drop(guard);
        assert_eq!(len(), 0);
    }

    #[test]
    fn test_context_guard_dropped_inside_snapshot_is_removed_afterwards() {
        // The audit's R12 reproduction, at the module level.
        clear();
        let guard = with_field("k", 1_u32);
        with_snapshot(|f| {
            assert_eq!(f.len(), 1);
            drop(guard);
        });
        assert_eq!(len(), 0, "removal must not leak");
    }

    #[test]
    fn test_context_nested_snapshots_see_outer_context() {
        clear();
        let _g = with_field("k", 1_u32);
        with_snapshot(|outer| {
            assert_eq!(outer.len(), 1);
            with_snapshot(|inner| assert_eq!(inner.len(), 1));
            let _late = with_field("late", 2_u32);
        });
        assert_eq!(keys(), ["k"], "the late guard was pushed and removed");
        clear();
    }

    #[test]
    fn test_context_clear_inside_snapshot_is_deferred() {
        clear();
        let _g = with_field("k", 1_u32);
        with_snapshot(|_| clear());
        assert_eq!(len(), 0);
    }

    #[test]
    fn test_context_guard_drop_during_tls_teardown_does_not_abort() {
        // The audit's LOG-008 reproduction: a guard stored in another
        // thread-local that is destroyed after `CONTEXT`.
        thread_local! {
            static HOLD: RefCell<Option<ContextGuard>> = const { RefCell::new(None) };
        }
        for order in 0..2 {
            std::thread::spawn(move || {
                if order == 0 {
                    HOLD.with(|h| drop(h.borrow_mut().take()));
                    let g = with_field("k", 1_u32);
                    HOLD.with(|h| *h.borrow_mut() = Some(g));
                } else {
                    let g = with_field("k", 1_u32);
                    HOLD.with(|h| *h.borrow_mut() = Some(g));
                }
            })
            .join()
            .expect("thread must exit cleanly");
        }
    }

    #[test]
    fn test_snapshot_capture_attach_scope() {
        clear();
        let snapshot = {
            let _t = with_trace_id("t-1");
            let _u = with_field("user", "u-1");
            capture()
        };
        assert_eq!(len(), 0);
        assert_eq!(snapshot.len(), 2);
        assert!(!snapshot.is_empty());
        let seen = snapshot.scope(keys);
        assert_eq!(seen, ["trace_id", "user"]);
        assert_eq!(len(), 0);
        let handle = {
            let snapshot = snapshot.clone();
            std::thread::spawn(move || snapshot.scope(keys))
        };
        assert_eq!(handle.join().unwrap(), ["trace_id", "user"]);
        let debug = format!("{snapshot:?}");
        assert!(debug.contains("trace_id"), "{debug}");
    }

    #[test]
    fn test_snapshot_shadowing_with_thread_fields() {
        clear();
        let snapshot = ContextSnapshot::from_fields(&[
            Field::new("tenant", "a"),
            Field::new("tenant", "b"),
            Field::new("region", "eu"),
        ]);
        assert_eq!(snapshot.len(), 2);
        let _early = with_field("region", "us");
        let _attached = snapshot.attach();
        let _late = with_field("tenant", "c");
        with_snapshot(|f| {
            let pairs: Vec<(&str, Value<'_>)> = f.iter().map(|f| (f.key, f.value)).collect();
            assert_eq!(
                pairs,
                [("region", Value::Str("eu")), ("tenant", Value::Str("c"))]
            );
        });
        clear();
    }

    #[test]
    fn test_empty_snapshot_attach_is_noop() {
        clear();
        let empty = capture();
        assert!(empty.is_empty());
        assert_eq!(empty.iter().count(), 0);
        let _g = empty.attach();
        assert_eq!(len(), 0);
        assert_eq!(ContextSnapshot::from_fields(&[]).len(), 0);
    }

    #[test]
    fn test_with_context_future_installs_snapshot_per_poll() {
        use std::task::Waker;

        /// Yields once, then reports the context length.
        struct YieldThenLen(bool);
        impl Future for YieldThenLen {
            type Output = usize;
            fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<usize> {
                if self.0 {
                    Poll::Ready(len())
                } else {
                    self.0 = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
        }

        clear();
        let snapshot = ContextSnapshot::from_fields(&[Field::new("req", "r1")]);
        let mut fut = snapshot.wrap(YieldThenLen(false));
        assert!(format!("{fut:?}").starts_with("WithContext"));
        let waker = Waker::noop().clone();
        let mut cx = TaskContext::from_waker(&waker);
        assert!(Pin::new(&mut fut).poll(&mut cx).is_pending());
        assert_eq!(len(), 0, "nothing leaks between polls");
        // Resume on a different thread, as a work-stealing executor might.
        let out = std::thread::spawn(move || {
            let waker = Waker::noop().clone();
            let mut cx = TaskContext::from_waker(&waker);
            Pin::new(&mut fut).poll(&mut cx)
        })
        .join()
        .unwrap();
        assert_eq!(out, Poll::Ready(1));
    }

    #[test]
    fn test_snapshot_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ContextSnapshot>();
        assert_send_sync::<WithContext<core::future::Ready<()>>>();
    }
}

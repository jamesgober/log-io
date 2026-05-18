//! Thread-local logging context.
//!
//! Context carries ambient key-value data that gets attached to every
//! record emitted from the current thread. Typical use: stash a
//! `trace_id` / `request_id` at the entry of a request handler so all
//! downstream log lines correlate. The [`ContextGuard`] returned by
//! [`with_field`] (and friends) removes the value when dropped.
//!
//! Context is intentionally thread-scoped. For async task propagation,
//! capture context fields at task spawn and re-install them on the
//! task's executor thread; this crate does not depend on a specific
//! async runtime.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::record::Field;
use crate::value::Value;

thread_local! {
    static CONTEXT: RefCell<ContextSlots> = const { RefCell::new(ContextSlots::new()) };
}

/// Maximum number of context slots per thread. Fixed-size so the
/// snapshot path is allocation-free.
pub const MAX_CONTEXT_SLOTS: usize = 16;

#[derive(Debug)]
struct ContextSlot {
    key: String,
    value: ContextValue,
    id: u64,
}

#[derive(Debug)]
enum ContextValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Char(char),
}

impl ContextValue {
    fn as_value(&self) -> Value<'_> {
        match self {
            Self::Null => Value::Null,
            Self::Bool(b) => Value::Bool(*b),
            Self::I64(n) => Value::I64(*n),
            Self::U64(n) => Value::U64(*n),
            Self::F64(n) => Value::F64(*n),
            Self::Str(s) => Value::Str(s.as_str()),
            Self::Char(c) => Value::Char(*c),
        }
    }
}

fn owned_from_value(v: Value<'_>) -> ContextValue {
    match v {
        Value::Null => ContextValue::Null,
        Value::Bool(b) => ContextValue::Bool(b),
        Value::I64(n) => ContextValue::I64(n),
        Value::U64(n) => ContextValue::U64(n),
        Value::F64(n) => ContextValue::F64(n),
        Value::Str(s) => ContextValue::Str(s.to_owned()),
        Value::Char(c) => ContextValue::Char(c),
    }
}

#[derive(Debug)]
struct ContextSlots {
    slots: Vec<ContextSlot>,
}

impl ContextSlots {
    const fn new() -> Self {
        Self { slots: Vec::new() }
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// RAII guard that removes a context slot when dropped.
///
/// Returned by [`with_field`], [`with_trace_id`], and
/// [`with_request_id`]. Drop the guard at the end of the scope where
/// the context applies. Dropping guards out-of-order is supported.
#[must_use = "context is removed when this guard is dropped; bind it to a variable"]
#[derive(Debug)]
pub struct ContextGuard {
    id: u64,
}

impl Drop for ContextGuard {
    fn drop(&mut self) {
        CONTEXT.with(|cell| {
            if let Ok(mut ctx) = cell.try_borrow_mut() {
                ctx.slots.retain(|s| s.id != self.id);
            }
        });
    }
}

/// Push a key/value pair onto the current thread's context.
///
/// Returns a guard; when the guard drops, the value is removed. If the
/// fixed-size slot limit ([`MAX_CONTEXT_SLOTS`]) is exceeded, the
/// oldest slot is dropped to make room.
///
/// # Example
///
/// ```
/// use log_io::context;
/// use log_io::Value;
///
/// {
///     let _g = context::with_field("user_id", Value::U64(42));
///     // log calls in this scope carry user_id=42
/// }
/// // user_id is gone here.
/// ```
pub fn with_field<'a, V: Into<Value<'a>>>(key: &str, value: V) -> ContextGuard {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let owned = owned_from_value(value.into());
    CONTEXT.with(|cell| {
        let mut ctx = cell.borrow_mut();
        if ctx.slots.len() >= MAX_CONTEXT_SLOTS {
            ctx.slots.remove(0);
        }
        ctx.slots.push(ContextSlot {
            key: key.to_owned(),
            value: owned,
            id,
        });
    });
    ContextGuard { id }
}

/// Convenience: push a `trace_id` field.
pub fn with_trace_id(id: &str) -> ContextGuard {
    with_field("trace_id", Value::Str(id))
}

/// Convenience: push a `request_id` field.
pub fn with_request_id(id: &str) -> ContextGuard {
    with_field("request_id", Value::Str(id))
}

/// Run `f` with the current thread's context exposed as a borrowed
/// slice of [`Field`]. No allocation: the slice is filled from a
/// stack-resident buffer.
///
/// Reentrancy-safe: if `f` is invoked transitively from inside another
/// `with_snapshot` call (or while a `with_field` guard is being
/// constructed), the inner snapshot sees an empty slice rather than
/// panicking. This makes the context system safe to use inside sinks
/// that happen to emit log records of their own.
pub fn with_snapshot<R>(f: impl FnOnce(&[Field<'_>]) -> R) -> R {
    CONTEXT.with(|cell| match cell.try_borrow() {
        Ok(ctx) => {
            const EMPTY: Field<'static> = Field {
                key: "",
                value: Value::Null,
            };
            let mut buf: [Field<'_>; MAX_CONTEXT_SLOTS] = [EMPTY; MAX_CONTEXT_SLOTS];
            let len = ctx.slots.len().min(MAX_CONTEXT_SLOTS);
            for (i, slot) in ctx.slots.iter().take(len).enumerate() {
                buf[i] = Field::new(slot.key.as_str(), slot.value.as_value());
            }
            f(&buf[..len])
        }
        Err(_) => f(&[]),
    })
}

/// Remove all context entries on the current thread.
///
/// Intended for tests; production code should rely on
/// [`ContextGuard`] drop semantics.
pub fn clear() {
    CONTEXT.with(|cell| {
        if let Ok(mut ctx) = cell.try_borrow_mut() {
            ctx.slots.clear();
        }
    });
}

/// Number of context entries on the current thread.
pub fn len() -> usize {
    CONTEXT.with(|cell| cell.borrow().slots.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_is_isolated_per_thread() {
        clear();
        let _g = with_field("k", Value::U64(1));
        assert_eq!(len(), 1);

        std::thread::spawn(|| {
            assert_eq!(len(), 0);
        })
        .join()
        .unwrap();
        clear();
    }

    #[test]
    fn guard_removes_on_drop() {
        clear();
        {
            let _g = with_field("k", Value::U64(1));
            with_snapshot(|f| assert_eq!(f.len(), 1));
        }
        with_snapshot(|f| assert_eq!(f.len(), 0));
    }

    #[test]
    fn nested_scopes_layer_and_unwind() {
        clear();
        let outer = with_field("outer", Value::U64(1));
        {
            let _inner = with_field("inner", Value::U64(2));
            with_snapshot(|f| assert_eq!(f.len(), 2));
        }
        with_snapshot(|f| {
            assert_eq!(f.len(), 1);
            assert_eq!(f[0].key, "outer");
        });
        drop(outer);
        with_snapshot(|f| assert_eq!(f.len(), 0));
    }

    #[test]
    fn slot_overflow_evicts_oldest() {
        clear();
        let mut guards = Vec::new();
        for i in 0..(MAX_CONTEXT_SLOTS + 5) {
            guards.push(with_field("k", Value::U64(i as u64)));
        }
        with_snapshot(|f| {
            assert_eq!(f.len(), MAX_CONTEXT_SLOTS);
            if let Value::U64(n) = f[0].value {
                assert!(n >= 5, "oldest entries should have been evicted");
            } else {
                panic!("unexpected variant");
            }
        });
        drop(guards);
        with_snapshot(|f| assert_eq!(f.len(), 0));
    }

    #[test]
    fn trace_and_request_id_helpers() {
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
}

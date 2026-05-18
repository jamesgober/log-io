//! Time helpers.
//!
//! Internal-only. Records carry timestamps as Unix nanoseconds; this
//! module produces them.

use std::time::{SystemTime, UNIX_EPOCH};

/// Read the current wall-clock time as Unix nanoseconds.
///
/// Falls back to `0` if the system clock is before the Unix epoch
/// (which is impossible on any sane machine; this is for soundness).
pub(crate) fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_2024() {
        // 2024-01-01T00:00:00Z = 1_704_067_200 seconds.
        let cutoff = 1_704_067_200_u128 * 1_000_000_000;
        assert!(now_unix_nanos() > cutoff);
    }
}

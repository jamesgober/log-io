//! RFC 3339 timestamp formatting from Unix nanoseconds.
//!
//! The date conversion is the civil-from-days algorithm from Howard
//! Hinnant's "chrono-Compatible Low-Level Date Algorithms", which is
//! exact for the proleptic Gregorian calendar and needs no dependency.

use core::fmt;

const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// The last instant RFC 3339 can express: `9999-12-31T23:59:59.999999999Z`.
pub(crate) const MAX_RFC3339_NANOS: u128 = 253_402_300_799 * NANOS_PER_SECOND + 999_999_999;

/// Write `unix_nanos` (nanoseconds since 1970-01-01T00:00:00Z) as
/// `YYYY-MM-DDTHH:MM:SS.fffffffffZ`, always in UTC.
///
/// RFC 3339 has four-digit years, so instants after
/// `9999-12-31T23:59:59.999999999Z` are clamped to that value rather
/// than wrapping or producing an invalid five-digit year. Instants
/// before 1970 cannot occur because the input is unsigned.
pub(crate) fn write_rfc3339_nanos<W: fmt::Write + ?Sized>(
    unix_nanos: u128,
    w: &mut W,
) -> fmt::Result {
    let clamped = unix_nanos.min(MAX_RFC3339_NANOS);
    // Both conversions are in range after the clamp: the seconds fit in
    // 38 bits and the sub-second part is below 10^9.
    let secs_total = u64::try_from(clamped / NANOS_PER_SECOND).unwrap_or(u64::MAX);
    let nanos_part = u32::try_from(clamped % NANOS_PER_SECOND).unwrap_or(0);

    let secs_of_day = secs_total % 86_400;
    let days_since_epoch = secs_total / 86_400;

    let (year, month, day) = civil_from_days(days_since_epoch);
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;

    write!(
        w,
        "{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{nanos_part:09}Z",
    )
}

/// Convert "days since 1970-01-01" to civil (year, month, day).
///
/// Only non-negative day counts reach this function (the input is
/// unsigned and clamped to year 9999), which keeps the arithmetic in
/// `u64` without sign handling.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn fmt_ts(ns: u128) -> String {
        let mut s = String::new();
        write_rfc3339_nanos(ns, &mut s).unwrap();
        s
    }

    #[test]
    fn test_rfc3339_epoch_renders_as_1970() {
        assert_eq!(fmt_ts(0), "1970-01-01T00:00:00.000000000Z");
    }

    #[test]
    fn test_rfc3339_known_instant() {
        assert_eq!(
            fmt_ts(1_700_000_000_000_000_000),
            "2023-11-14T22:13:20.000000000Z"
        );
    }

    #[test]
    fn test_rfc3339_sub_second_precision_preserved() {
        assert_eq!(
            fmt_ts(1_700_000_000_123_456_789),
            "2023-11-14T22:13:20.123456789Z"
        );
    }

    #[test]
    fn test_rfc3339_leap_days() {
        assert_eq!(
            fmt_ts(1_709_164_800_000_000_000),
            "2024-02-29T00:00:00.000000000Z"
        );
        // 2000 is a leap year (divisible by 400); 2100 is not.
        assert_eq!(
            fmt_ts(951_782_400_000_000_000),
            "2000-02-29T00:00:00.000000000Z"
        );
        assert_eq!(
            fmt_ts(4_107_542_400_000_000_000),
            "2100-03-01T00:00:00.000000000Z"
        );
    }

    #[test]
    fn test_rfc3339_last_representable_instant() {
        assert_eq!(fmt_ts(MAX_RFC3339_NANOS), "9999-12-31T23:59:59.999999999Z");
    }

    #[test]
    fn test_rfc3339_out_of_range_is_clamped_not_wrapped() {
        for ns in [
            MAX_RFC3339_NANOS + 1,
            u128::from(u64::MAX) * NANOS_PER_SECOND,
            u128::MAX,
        ] {
            assert_eq!(fmt_ts(ns), "9999-12-31T23:59:59.999999999Z", "{ns}");
        }
    }

    #[test]
    fn test_rfc3339_day_boundaries_match_reference_walk() {
        // Walk day by day from the epoch through 2400 and compare with a
        // straightforward calendar counter.
        let mut y = 1970_u64;
        let mut m = 1_u64;
        let mut d = 1_u64;
        let is_leap = |y: u64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        for day in 0..157_000_u64 {
            assert_eq!(civil_from_days(day), (y, m, d), "day {day}");
            let len = match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                _ if is_leap(y) => 29,
                _ => 28,
            };
            d += 1;
            if d > len {
                d = 1;
                m += 1;
                if m > 12 {
                    m = 1;
                    y += 1;
                }
            }
        }
    }
}

//! RFC 3339 timestamp formatting from Unix nanoseconds.
//!
//! The algorithm is the standard civil-from-days conversion from
//! Howard Hinnant's "chrono-Compatible Low-Level Date Algorithms".
//! It is correct for the Gregorian range and avoids any dependency.

use core::fmt;

/// Write `unix_nanos` since the Unix epoch as `YYYY-MM-DDTHH:MM:SS.fffffffffZ`.
///
/// The output is always UTC. Negative values (instants before 1970)
/// are supported by the algorithm. No quoting is applied.
pub(crate) fn write_rfc3339_nanos<W: fmt::Write + ?Sized>(
    unix_nanos: u128,
    w: &mut W,
) -> fmt::Result {
    const NANOS_PER_SECOND: u128 = 1_000_000_000;
    let secs_total = (unix_nanos / NANOS_PER_SECOND) as i64;
    #[allow(clippy::cast_possible_truncation)]
    let nanos_part = (unix_nanos % NANOS_PER_SECOND) as u32;

    let secs_of_day = secs_total.rem_euclid(86_400);
    let days_since_epoch = secs_total.div_euclid(86_400);

    let (year, month, day) = civil_from_days(days_since_epoch);
    #[allow(clippy::cast_possible_truncation)]
    let h = (secs_of_day / 3600) as u32;
    #[allow(clippy::cast_possible_truncation)]
    let m = ((secs_of_day % 3600) / 60) as u32;
    #[allow(clippy::cast_possible_truncation)]
    let s = (secs_of_day % 60) as u32;

    write!(
        w,
        "{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{nanos_part:09}Z",
    )
}

/// Convert "days since 1970-01-01" to civil (year, month, day).
///
/// Algorithm from Hinnant 2013; correct for any 64-bit day count.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = y + i64::from(m <= 2);
    (y as i32, m, d)
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
    fn epoch_renders_as_1970() {
        assert_eq!(fmt_ts(0), "1970-01-01T00:00:00.000000000Z");
    }

    #[test]
    fn known_instant_2023_11_14() {
        // 1_700_000_000 seconds since epoch
        assert_eq!(
            fmt_ts(1_700_000_000_000_000_000),
            "2023-11-14T22:13:20.000000000Z"
        );
    }

    #[test]
    fn sub_second_precision_preserved() {
        assert_eq!(
            fmt_ts(1_700_000_000_123_456_789),
            "2023-11-14T22:13:20.123456789Z"
        );
    }

    #[test]
    fn leap_year_feb_29() {
        // 2024-02-29T00:00:00Z = 1_709_164_800 seconds.
        assert_eq!(
            fmt_ts(1_709_164_800_000_000_000),
            "2024-02-29T00:00:00.000000000Z"
        );
    }
}

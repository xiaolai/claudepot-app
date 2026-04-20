//! Shared local-datetime formatting.
//!
//! One module, one format convention: `HH:MM (+08)` /
//! `YYYY-MM-DD HH:MM (+08)`, with a `(UTC)` marker on the zero offset
//! and half-hour / quarter-hour offsets rendered as `(+05:45)`. Used
//! by every command that prints local timestamps so reset times,
//! last-switch times, and created-at dates all read the same.
//!
//! Extracted from the ad-hoc formatter previously buried in
//! `commands/account.rs`. That version also had a subtle sign-loss
//! bug (offsets in `(-00:00, -01:00)` rendered with `+` because
//! integer division truncates toward zero); `format_offset` fixes it
//! by tracking the sign explicitly.

use chrono::{DateTime, Local};

/// Render a `DateTime<Local>` as `HH:MM (+08)`.
pub fn format_local_time_of_day(dt: &DateTime<Local>) -> String {
    let offset = format_offset(dt.offset().local_minus_utc());
    format!("{} {offset}", dt.format("%H:%M"))
}

/// Render a `DateTime<Local>` as `YYYY-MM-DD HH:MM (+08)`.
pub fn format_local_datetime(dt: &DateTime<Local>) -> String {
    let offset = format_offset(dt.offset().local_minus_utc());
    format!("{} {offset}", dt.format("%Y-%m-%d %H:%M"))
}

/// Render a UTC offset in seconds as `(+08)` / `(-05:30)` / `(UTC)`.
/// Separated so sign / half-hour / UTC-zero branches are unit-testable
/// without spinning up a full `DateTime`.
pub fn format_offset(offset_secs: i32) -> String {
    if offset_secs == 0 {
        return "(UTC)".to_string();
    }
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    let hours = abs / 3600;
    let mins = (abs % 3600) / 60;
    if mins == 0 {
        format!("({sign}{hours:02})")
    } else {
        format!("({sign}{hours:02}:{mins:02})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    #[test]
    fn test_format_offset_zero_is_utc_marker() {
        assert_eq!(format_offset(0), "(UTC)");
    }

    #[test]
    fn test_format_offset_positive_whole_hour() {
        assert_eq!(format_offset(8 * 3600), "(+08)");
        assert_eq!(format_offset(3600), "(+01)");
    }

    #[test]
    fn test_format_offset_negative_whole_hour() {
        assert_eq!(format_offset(-5 * 3600), "(-05)");
    }

    #[test]
    fn test_format_offset_positive_half_hour() {
        // India: UTC+05:30, Nepal: UTC+05:45.
        assert_eq!(format_offset(5 * 3600 + 30 * 60), "(+05:30)");
        assert_eq!(format_offset(5 * 3600 + 45 * 60), "(+05:45)");
    }

    #[test]
    fn test_format_offset_negative_half_hour() {
        // Newfoundland: UTC-03:30. Marquesas: UTC-09:30.
        assert_eq!(format_offset(-(3 * 3600 + 30 * 60)), "(-03:30)");
        assert_eq!(format_offset(-(9 * 3600 + 30 * 60)), "(-09:30)");
    }

    #[test]
    fn test_format_offset_sub_hour_negative_preserves_sign() {
        // Regression guard for the bug in the old formatter: offsets
        // strictly between -3600 and 0 have hours=0 after integer
        // division, and `{:+03}` on 0 prints `+00`, losing the sign.
        // No real timezone uses -00:30, but the formatter must be
        // correct for any offset the runtime hands us.
        assert_eq!(format_offset(-1800), "(-00:30)");
        assert_eq!(format_offset(-900), "(-00:15)");
    }

    #[test]
    fn test_format_offset_sub_hour_positive() {
        assert_eq!(format_offset(1800), "(+00:30)");
    }

    #[test]
    fn test_format_local_time_of_day_utc() {
        let dt = FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 4, 20, 14, 59, 0)
            .unwrap();
        let local: DateTime<Local> = dt.into();
        // Only assert the UTC marker to avoid depending on the test
        // host's local-zone conversion of 14:59 UTC.
        assert!(
            format_local_time_of_day(&local).ends_with(
                &format_offset(local.offset().local_minus_utc())
            )
        );
    }

    #[test]
    fn test_format_local_datetime_shape() {
        // "YYYY-MM-DD HH:MM (tz)" — dash-separated date, colon time,
        // space before paren-wrapped offset.
        let dt = FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 4, 20, 14, 59, 30)
            .unwrap();
        let local: DateTime<Local> = dt.into();
        let s = format_local_datetime(&local);
        assert_eq!(s.matches('-').count(), 2); // date only
        assert_eq!(s.matches(':').count(), 1); // HH:MM only, no seconds
        assert!(s.ends_with(')'));
    }
}

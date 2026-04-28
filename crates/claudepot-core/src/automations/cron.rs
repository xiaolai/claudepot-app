//! Five-field cron parser.
//!
//! Format: `min hour dom mon dow`
//!   min  0–59
//!   hour 0–23
//!   dom  1–31
//!   mon  1–12 OR `jan..dec` (case-insensitive 3-letter)
//!   dow  0–7  (0 and 7 = Sunday) OR `sun..sat` (case-insensitive 3-letter)
//!
//! Each field accepts:
//!   `*`            wildcard
//!   `N`            literal
//!   `A-B`          range, inclusive
//!   `A,B,C`        list (combinable with ranges and step)
//!   `*/N` `A-B/N`  step (1..=field-width)
//!
//! No seconds. No `@reboot`/`@daily` macros. No L/W/#. We expand
//! to a list of [`LaunchSlot`]s — every `(minute, hour,
//! day-of-month, month, day-of-week)` tuple that fires. The
//! adapters in `super::scheduler` translate those tuples into
//! launchd `StartCalendarInterval` dicts, Task Scheduler triggers,
//! or systemd `OnCalendar=` lines.

use super::error::AutomationError;

/// Hard cap on the number of (m, h, dom, mon, dow) slots. Catches
/// `* * * * *` (44640) and similar typos. 200 is enough for every
/// realistic cron spec we expect (hourly = 24, every weekday at 5
/// times = 25, business-hours every 15 min = 160).
pub const MAX_SLOTS: usize = 200;

/// One concrete launch slot expanded from a cron expression. All
/// fields are absolute (no wildcards). Day-of-month and day-of-week
/// are kept independent — translating to "OR semantics" matches
/// classic Vixie-cron behavior. When both `dom` and `dow` are
/// unrestricted (full range), we collapse the dow axis so we don't
/// emit 7× redundant slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchSlot {
    pub minute: u8,
    pub hour: u8,
    /// 1..=31 or `None` for "any day-of-month."
    pub day_of_month: Option<u8>,
    /// 1..=12 or `None` for "any month."
    pub month: Option<u8>,
    /// 0..=6 (Sun..Sat) or `None` for "any day-of-week."
    pub day_of_week: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    /// Sorted, deduplicated set of values that match this field.
    values: Vec<u8>,
    /// True iff every value in the field's domain is present
    /// (used to suppress redundant per-value enumeration).
    is_full: bool,
}

/// Parse a cron expression and expand to launch slots. Bounded by
/// [`MAX_SLOTS`].
pub fn expand(expr: &str) -> Result<Vec<LaunchSlot>, AutomationError> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(AutomationError::InvalidCron(
            expr.to_string(),
            format!("expected 5 fields, got {}", parts.len()),
        ));
    }

    let minute = parse_field(parts[0], 0, 59, expr, FieldKind::Minute)?;
    let hour = parse_field(parts[1], 0, 23, expr, FieldKind::Hour)?;
    let dom = parse_field(parts[2], 1, 31, expr, FieldKind::Dom)?;
    let mon = parse_field(parts[3], 1, 12, expr, FieldKind::Mon)?;
    let dow = parse_field(parts[4], 0, 7, expr, FieldKind::Dow)?;

    // Collapse 7 → 0 (both are Sunday) and dedupe.
    let mut dow_values: Vec<u8> = dow.values.iter().map(|&v| v % 7).collect();
    dow_values.sort_unstable();
    dow_values.dedup();
    // dow domain is 0..=6 after the modulo; "is_full" if all 7 weekdays present.
    let dow_full = dow_values.len() == 7;
    let dow = Field {
        values: dow_values,
        is_full: dow_full,
    };

    let mut slots = Vec::new();
    for &m in &minute.values {
        for &h in &hour.values {
            // Vixie-cron semantics: when only one of dom/dow is
            // restricted, that one applies. When both are restricted,
            // we OR them (union of matching days). When both are
            // unrestricted, we emit a single "any-day" slot.
            match (dom.is_full, dow.is_full) {
                (true, true) => {
                    // dom + dow both unrestricted. If `mon` is also
                    // unrestricted, emit a single any-day slot. If
                    // `mon` is restricted, we MUST iterate every
                    // selected month — `pick_month` returned only the
                    // first one, so `0 9 * jan,feb *` was silently
                    // collapsing to January only.
                    for mo in iter_months(&mon) {
                        slots.push(LaunchSlot {
                            minute: m,
                            hour: h,
                            day_of_month: None,
                            month: mo,
                            day_of_week: None,
                        });
                        check_cap(&slots, expr)?;
                    }
                }
                (false, true) => {
                    for &d in &dom.values {
                        for mo in iter_months(&mon) {
                            slots.push(LaunchSlot {
                                minute: m,
                                hour: h,
                                day_of_month: Some(d),
                                month: mo,
                                day_of_week: None,
                            });
                            check_cap(&slots, expr)?;
                        }
                    }
                }
                (true, false) => {
                    for &w in &dow.values {
                        for mo in iter_months(&mon) {
                            slots.push(LaunchSlot {
                                minute: m,
                                hour: h,
                                day_of_month: None,
                                month: mo,
                                day_of_week: Some(w),
                            });
                            check_cap(&slots, expr)?;
                        }
                    }
                }
                (false, false) => {
                    // Vixie semantics: union. Emit slots for both
                    // axes; consumers downstream OR them naturally
                    // (a launchd entry with `Day` set fires on that
                    // calendar day; an entry with `Weekday` set
                    // fires on that weekday — both registered, both
                    // honored).
                    for &d in &dom.values {
                        for mo in iter_months(&mon) {
                            slots.push(LaunchSlot {
                                minute: m,
                                hour: h,
                                day_of_month: Some(d),
                                month: mo,
                                day_of_week: None,
                            });
                            check_cap(&slots, expr)?;
                        }
                    }
                    for &w in &dow.values {
                        for mo in iter_months(&mon) {
                            slots.push(LaunchSlot {
                                minute: m,
                                hour: h,
                                day_of_month: None,
                                month: mo,
                                day_of_week: Some(w),
                            });
                            check_cap(&slots, expr)?;
                        }
                    }
                }
            }
            check_cap(&slots, expr)?;
        }
    }

    if slots.is_empty() {
        return Err(AutomationError::InvalidCron(
            expr.to_string(),
            "expression matches no times".into(),
        ));
    }
    Ok(slots)
}

fn check_cap(slots: &[LaunchSlot], expr: &str) -> Result<(), AutomationError> {
    if slots.len() > MAX_SLOTS {
        return Err(AutomationError::CronTooDense(
            expr.to_string(),
            slots.len(),
            MAX_SLOTS,
        ));
    }
    Ok(())
}

fn pick_month(mon: &Field) -> Option<u8> {
    if mon.is_full {
        None
    } else {
        // Should not happen — if mon is restricted we use iter_months.
        Some(mon.values[0])
    }
}

fn iter_months(mon: &Field) -> Box<dyn Iterator<Item = Option<u8>> + '_> {
    if mon.is_full {
        Box::new(std::iter::once(None))
    } else {
        Box::new(mon.values.iter().map(|&v| Some(v)))
    }
}

#[derive(Copy, Clone)]
enum FieldKind {
    Minute,
    Hour,
    Dom,
    Mon,
    Dow,
}

fn parse_field(
    raw: &str,
    min: u8,
    max: u8,
    expr: &str,
    kind: FieldKind,
) -> Result<Field, AutomationError> {
    let mut values: Vec<u8> = Vec::new();
    for part in raw.split(',') {
        parse_part(part, min, max, expr, kind, &mut values)?;
    }
    values.sort_unstable();
    values.dedup();
    if values.is_empty() {
        return Err(AutomationError::InvalidCron(
            expr.to_string(),
            format!("field '{raw}' produced no values"),
        ));
    }
    let domain = (max - min + 1) as usize;
    let is_full = values.len() == domain
        && values.first().copied() == Some(min)
        && values.last().copied() == Some(max);
    Ok(Field { values, is_full })
}

fn parse_part(
    part: &str,
    min: u8,
    max: u8,
    expr: &str,
    kind: FieldKind,
    out: &mut Vec<u8>,
) -> Result<(), AutomationError> {
    let (range_part, step) = match part.split_once('/') {
        Some((r, s)) => {
            let n: u8 = s.parse().map_err(|_| {
                AutomationError::InvalidCron(
                    expr.to_string(),
                    format!("invalid step in '{part}'"),
                )
            })?;
            if n == 0 {
                return Err(AutomationError::InvalidCron(
                    expr.to_string(),
                    format!("step must be >= 1 in '{part}'"),
                ));
            }
            (r, n)
        }
        None => (part, 1u8),
    };

    let (start, end) = if range_part == "*" {
        (min, max)
    } else if let Some((a, b)) = range_part.split_once('-') {
        let a = parse_value(a, kind, expr)?;
        let b = parse_value(b, kind, expr)?;
        if a > b {
            return Err(AutomationError::InvalidCron(
                expr.to_string(),
                format!("inverted range '{range_part}'"),
            ));
        }
        (a, b)
    } else {
        let v = parse_value(range_part, kind, expr)?;
        (v, v)
    };

    if start < min || end > max {
        return Err(AutomationError::InvalidCron(
            expr.to_string(),
            format!("'{range_part}' out of range {min}..={max}"),
        ));
    }

    let mut v = start;
    while v <= end {
        out.push(v);
        v = match v.checked_add(step) {
            Some(x) => x,
            None => break,
        };
    }
    Ok(())
}

fn parse_value(raw: &str, kind: FieldKind, expr: &str) -> Result<u8, AutomationError> {
    if let Ok(n) = raw.parse::<u8>() {
        return Ok(n);
    }
    let lower = raw.to_ascii_lowercase();
    let table: &[(&str, u8)] = match kind {
        FieldKind::Mon => &[
            ("jan", 1), ("feb", 2), ("mar", 3), ("apr", 4),
            ("may", 5), ("jun", 6), ("jul", 7), ("aug", 8),
            ("sep", 9), ("oct", 10), ("nov", 11), ("dec", 12),
        ],
        FieldKind::Dow => &[
            ("sun", 0), ("mon", 1), ("tue", 2), ("wed", 3),
            ("thu", 4), ("fri", 5), ("sat", 6),
        ],
        _ => &[],
    };
    table
        .iter()
        .find_map(|(n, v)| (*n == lower).then_some(*v))
        .ok_or_else(|| {
            AutomationError::InvalidCron(
                expr.to_string(),
                format!("unrecognized value '{raw}'"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(expr: &str) -> Vec<LaunchSlot> {
        expand(expr).unwrap_or_else(|e| panic!("expected ok for {expr:?}: {e}"))
    }

    #[test]
    fn star_collapses_to_any_day_any_month() {
        // 0 9 * * * → one slot, 9:00, no day/month/weekday filters
        let s = ok("0 9 * * *");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].minute, 0);
        assert_eq!(s[0].hour, 9);
        assert_eq!(s[0].day_of_month, None);
        assert_eq!(s[0].month, None);
        assert_eq!(s[0].day_of_week, None);
    }

    #[test]
    fn weekdays_only() {
        // Mon..Fri at 09:00 → 5 slots, all dow set, no dom
        let s = ok("0 9 * * 1-5");
        assert_eq!(s.len(), 5);
        let dows: Vec<u8> = s.iter().filter_map(|x| x.day_of_week).collect();
        assert_eq!(dows, vec![1, 2, 3, 4, 5]);
        for slot in &s {
            assert_eq!(slot.day_of_month, None);
        }
    }

    #[test]
    fn step_quarter_hour() {
        // Every 15 min on the hour 9 → 4 slots
        let s = ok("*/15 9 * * *");
        assert_eq!(s.len(), 4);
        let mins: Vec<u8> = s.iter().map(|x| x.minute).collect();
        assert_eq!(mins, vec![0, 15, 30, 45]);
    }

    #[test]
    fn comma_list_in_minute_field() {
        let s = ok("0,30 9 * * *");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].minute, 0);
        assert_eq!(s[1].minute, 30);
    }

    #[test]
    fn month_names_and_dow_names() {
        let s = ok("0 9 * jan-feb mon");
        // Two months × one weekday slot each → 2
        assert_eq!(s.len(), 2);
        let months: Vec<Option<u8>> = s.iter().map(|x| x.month).collect();
        assert!(months.contains(&Some(1)));
        assert!(months.contains(&Some(2)));
    }

    #[test]
    fn dow_seven_collapses_to_sunday() {
        let s = ok("0 9 * * 7");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].day_of_week, Some(0));
    }

    #[test]
    fn rejects_too_few_fields() {
        assert!(expand("0 9 * *").is_err());
    }

    #[test]
    fn rejects_too_many_fields() {
        assert!(expand("0 9 * * * *").is_err());
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(expand("60 9 * * *").is_err());
        assert!(expand("0 24 * * *").is_err());
        assert!(expand("0 9 32 * *").is_err());
        assert!(expand("0 9 * 13 *").is_err());
        assert!(expand("0 9 * * 8").is_err());
    }

    #[test]
    fn rejects_inverted_range() {
        assert!(expand("0 9 * * 5-1").is_err());
    }

    #[test]
    fn rejects_zero_step() {
        assert!(expand("*/0 9 * * *").is_err());
    }

    #[test]
    fn rejects_unknown_alpha() {
        assert!(expand("0 9 * sun *").is_err());        // sun in mon column
        assert!(expand("0 9 * * jan").is_err());        // jan in dow column
        assert!(expand("0 9 * xyz *").is_err());
    }

    #[test]
    fn rejects_too_dense() {
        // every minute every day → 1440 slots, way over MAX_SLOTS
        assert!(matches!(
            expand("* * * * *"),
            Err(AutomationError::CronTooDense(..))
        ));
    }

    #[test]
    fn dom_only_fires_each_listed_day() {
        let s = ok("0 9 1,15 * *");
        assert_eq!(s.len(), 2);
        let doms: Vec<Option<u8>> = s.iter().map(|x| x.day_of_month).collect();
        assert!(doms.contains(&Some(1)));
        assert!(doms.contains(&Some(15)));
    }

    #[test]
    fn restricted_months_with_star_dom_and_dow_keeps_all_months() {
        // Regression: previously collapsed to the first month only
        // because `pick_month` discarded the rest.
        let s = ok("0 9 * jan,feb *");
        assert_eq!(s.len(), 2);
        let months: Vec<Option<u8>> = s.iter().map(|x| x.month).collect();
        assert!(months.contains(&Some(1)));
        assert!(months.contains(&Some(2)));
        for slot in &s {
            assert_eq!(slot.day_of_month, None);
            assert_eq!(slot.day_of_week, None);
        }
    }

    #[test]
    fn dom_and_dow_both_set_emits_union() {
        // Vixie-cron OR semantics: 1st of month OR Mondays
        let s = ok("0 9 1 * 1");
        assert_eq!(s.len(), 2);
        // One slot has dom=Some(1), dow=None; another dom=None, dow=Some(1)
        assert!(s.iter().any(|x| x.day_of_month == Some(1) && x.day_of_week.is_none()));
        assert!(s.iter().any(|x| x.day_of_month.is_none() && x.day_of_week == Some(1)));
    }
}

//! Bounding the device list.
//!
//! `remote-devices.json` only ever grew. Every pairing appends a
//! [`Device`], and so does every password login — a session and a paired
//! device are deliberately the same thing (see [`super::login`]), so a
//! row lands on each sign-in. Revocation *sets a mark* rather than
//! removing the row, because this file is the revocation list. Nothing
//! anywhere removed anything.
//!
//! # Why removing a spent row is safe, and what it costs
//!
//! [`super::authenticate`] filters to `is_usable_at` and only then
//! matches the hash, so an unknown token and a revoked one are the same
//! answer: `None`. Dropping a row therefore cannot let a token back in.
//!
//! What it costs is *history* — the ability to answer "was this token
//! ever issued, and when did I turn it off". That is the whole reason
//! the rows are kept, so the policy below spends that history as slowly
//! as it can while still being bounded.
//!
//! # The policy
//!
//! - **A live device is never pruned.** Unrevoked and unexpired means
//!   someone is using it; no cap justifies removing it, so the caps
//!   below apply only to rows that are already refused.
//! - **An expired *session* is dropped [`EXPIRED_GRACE_DAYS`] after it
//!   expired.** A session is machine-issued and self-expiring, so its
//!   audit value decays fast — nobody decided anything by letting one
//!   lapse.
//! - **A revoked device is kept, up to [`MAX_REVOKED`], newest first.**
//!   Revoking is a decision a human made about a specific device, which
//!   is worth far more than a lapsed session and is kept far longer.
//!
//! That asymmetry between the two is the load-bearing judgement here:
//! both rows are equally refused by `authenticate`, and they are *not*
//! equally interesting to a person reading the list later.

use chrono::{DateTime, Duration, Utc};

use super::Device;

/// How long a lapsed session stays on the list after its expiry.
///
/// Sessions live [`super::login::SESSION_TTL_DAYS`] (30) and then sit
/// here for this long, so a login is visible for ~4 months total.
pub const EXPIRED_GRACE_DAYS: i64 = 90;

/// How many revoked devices to keep, newest first.
///
/// Matches the order of magnitude `remote::panel::read_state` already
/// uses for its own caps (200 sessions / 32 devices) rather than
/// inventing a new scale.
pub const MAX_REVOKED: usize = 200;

/// What a prune removed. Returned rather than logged inside, so the
/// caller decides where it is reported — and so it is assertable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pruned {
    /// Sessions dropped for being expired longer than the grace period.
    pub expired_sessions: usize,
    /// Revoked devices dropped for falling past [`MAX_REVOKED`].
    pub revoked_over_cap: usize,
}

impl Pruned {
    pub fn total(self) -> usize {
        self.expired_sessions + self.revoked_over_cap
    }

    pub fn is_empty(self) -> bool {
        self.total() == 0
    }
}

/// Drop spent rows past their keep-window. Order of the survivors is
/// preserved — the list is rendered in it.
pub fn prune(devices: &mut Vec<Device>, now: DateTime<Utc>) -> Pruned {
    let mut out = Pruned::default();

    // 1. Lapsed sessions, past the grace period.
    //
    // `revoked_at` is checked first so a revoked session is judged as a
    // revocation, not as a lapse — otherwise a device someone
    // deliberately turned off could leave on the session timetable,
    // which is the shorter one.
    let cutoff = now - Duration::days(EXPIRED_GRACE_DAYS);
    devices.retain(|d| {
        if d.revoked_at.is_some() {
            return true;
        }
        match d.expires_at {
            Some(e) if e <= cutoff => {
                out.expired_sessions += 1;
                false
            }
            _ => true,
        }
    });

    // 2. Revoked devices past the cap, oldest first.
    //
    // Ranked by `revoked_at` — when it was turned off — rather than by
    // `created_at`. A long-lived device revoked this morning is the most
    // interesting row in the file, and ranking by creation would evict
    // it before a throwaway paired and revoked last year.
    let revoked = devices.iter().filter(|d| d.revoked_at.is_some()).count();
    if revoked > MAX_REVOKED {
        let mut marks: Vec<DateTime<Utc>> = devices.iter().filter_map(|d| d.revoked_at).collect();
        marks.sort_unstable();
        // Everything revoked strictly before this goes. Ties are kept,
        // so the survivor count can exceed the cap by the width of a
        // tie rather than dropping an arbitrary one of two rows that
        // carry the same instant.
        let threshold = marks[revoked - MAX_REVOKED];
        devices.retain(|d| match d.revoked_at {
            Some(r) if r < threshold => {
                out.revoked_over_cap += 1;
                false
            }
            _ => true,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::Device;
    use chrono::TimeZone;
    use uuid::Uuid;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 25, 12, 0, 0).unwrap()
    }

    fn dev(name: &str) -> Device {
        Device {
            id: Uuid::new_v4(),
            name: name.into(),
            token_hash: format!("hash-{name}"),
            created_at: now() - Duration::days(400),
            last_seen: None,
            revoked_at: None,
            expires_at: None,
        }
    }

    fn revoked_days_ago(name: &str, d: i64) -> Device {
        Device {
            revoked_at: Some(now() - Duration::days(d)),
            ..dev(name)
        }
    }

    fn session_expiring_in(name: &str, d: i64) -> Device {
        Device {
            expires_at: Some(now() + Duration::days(d)),
            ..dev(name)
        }
    }

    #[test]
    fn a_live_device_is_never_pruned() {
        // Not by the cap, not by the grace period, not ever. Someone is
        // using it.
        let mut v = vec![dev("paired"), session_expiring_in("session", 5)];
        let out = prune(&mut v, now());
        assert!(out.is_empty());
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn a_lapsed_session_survives_the_grace_period_then_goes() {
        let mut v = vec![
            session_expiring_in("just-lapsed", -1),
            session_expiring_in("inside-grace", -(EXPIRED_GRACE_DAYS - 1)),
            session_expiring_in("on-the-boundary", -EXPIRED_GRACE_DAYS),
            session_expiring_in("long-gone", -(EXPIRED_GRACE_DAYS + 1)),
        ];
        let out = prune(&mut v, now());
        assert_eq!(out.expired_sessions, 2, "boundary is inclusive");
        let names: Vec<&str> = v.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["just-lapsed", "inside-grace"]);
    }

    #[test]
    fn a_revoked_session_is_judged_as_a_revocation_not_a_lapse() {
        // Both marks set, and the session timetable is much shorter. A
        // device someone deliberately turned off must not leave early
        // just because its token also happened to lapse.
        let mut v = vec![Device {
            revoked_at: Some(now() - Duration::days(1)),
            expires_at: Some(now() - Duration::days(EXPIRED_GRACE_DAYS + 10)),
            ..dev("revoked-and-lapsed")
        }];
        let out = prune(&mut v, now());
        assert!(out.is_empty(), "a revocation is not a lapse");
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn revoked_devices_are_kept_up_to_the_cap() {
        let mut v: Vec<Device> = (0..MAX_REVOKED as i64)
            .map(|i| revoked_days_ago(&format!("r{i}"), i))
            .collect();
        let out = prune(&mut v, now());
        assert!(out.is_empty(), "exactly at the cap keeps everything");
        assert_eq!(v.len(), MAX_REVOKED);
    }

    #[test]
    fn past_the_cap_the_oldest_revocations_go_first() {
        // Newest-first is the point: a device revoked this morning is the
        // most interesting row in the file.
        let mut v: Vec<Device> = (0..(MAX_REVOKED as i64 + 5))
            .map(|i| revoked_days_ago(&format!("r{i}"), i))
            .collect();
        let out = prune(&mut v, now());
        assert_eq!(out.revoked_over_cap, 5);
        assert_eq!(v.len(), MAX_REVOKED);
        // r0 was revoked today and must survive; the five oldest are gone.
        let names: Vec<&str> = v.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"r0"));
        for gone in [
            MAX_REVOKED as i64,
            MAX_REVOKED as i64 + 1,
            MAX_REVOKED as i64 + 4,
        ] {
            assert!(!names.contains(&format!("r{gone}").as_str()), "r{gone}");
        }
    }

    #[test]
    fn the_cap_never_evicts_a_live_device_to_make_room() {
        // The live rows go FIRST, and that placement is the whole test.
        // An earlier version appended `my-phone` last and passed against
        // a cap that dropped rows by position rather than by liveness —
        // the live device survived by luck of ordering, not by the rule.
        // Watched, against exactly that mutation.
        let mut v: Vec<Device> = vec![dev("my-phone"), session_expiring_in("my-laptop", 5)];
        v.extend((0..(MAX_REVOKED as i64 + 3)).map(|i| revoked_days_ago(&format!("r{i}"), i)));

        let out = prune(&mut v, now());
        assert_eq!(out.revoked_over_cap, 3);
        assert_eq!(out.expired_sessions, 0);
        for keep in ["my-phone", "my-laptop"] {
            assert!(
                v.iter().any(|d| d.name == keep),
                "{keep} is usable and must survive any cap"
            );
        }
        // And nothing usable was dropped, counted rather than spot-checked.
        assert_eq!(v.iter().filter(|d| d.is_usable_at(now())).count(), 2);
    }

    #[test]
    fn survivor_order_is_preserved() {
        // The list is rendered in this order; a prune must not shuffle it.
        let mut v = vec![
            dev("a"),
            session_expiring_in("gone", -(EXPIRED_GRACE_DAYS + 1)),
            dev("b"),
            dev("c"),
        ];
        prune(&mut v, now());
        let names: Vec<&str> = v.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn admit_bounds_the_list_but_never_evicts_the_new_device() {
        use crate::remote::DevicesFile;
        // A full list plus a lapsed session, then one more login. The
        // prune runs BEFORE the push precisely so the arriving device
        // cannot be what the cap drops.
        let mut file = DevicesFile {
            devices: (0..(MAX_REVOKED as i64 + 2))
                .map(|i| revoked_days_ago(&format!("r{i}"), i))
                .chain(std::iter::once(session_expiring_in(
                    "lapsed",
                    -(EXPIRED_GRACE_DAYS + 1),
                )))
                .collect(),
            ..Default::default()
        };
        let pruned = file.admit(dev("brand-new"), now());

        assert_eq!(pruned.expired_sessions, 1);
        assert_eq!(pruned.revoked_over_cap, 2);
        assert!(
            file.devices.iter().any(|d| d.name == "brand-new"),
            "the device being admitted is never the one evicted"
        );
        // And the file it produces is still one the store will accept.
        file.validate().expect("pruned file stays valid");
    }

    #[test]
    fn admit_on_a_fresh_install_prunes_nothing() {
        use crate::remote::DevicesFile;
        let mut file = DevicesFile::default();
        assert!(file.admit(dev("first"), now()).is_empty());
        assert_eq!(file.devices.len(), 1);
    }

    #[test]
    fn an_empty_list_is_a_no_op() {
        let mut v: Vec<Device> = vec![];
        assert!(prune(&mut v, now()).is_empty());
        assert!(v.is_empty());
    }
}

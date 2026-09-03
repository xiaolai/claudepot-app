//! Pure expiration logic over a set of grants. No I/O — the clock is
//! injected so the orchestrator and tests share one code path. The
//! per-prompt question ("does a grant cover this cwd?") lives in
//! `permission::hook`; this module answers the orchestrator's ("which
//! grants have lapsed?").

use chrono::{DateTime, Utc};

use crate::permission::grants::{Grant, GrantsFile};

/// Grants split by whether they have reached their deadline at `now`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantPartition<'a> {
    /// Not yet expired — the UI shows these with a live countdown.
    pub active: Vec<&'a Grant>,
    /// Reached or passed `expires_at` — the orchestrator drops these.
    pub expired: Vec<&'a Grant>,
}

/// Split `file`'s grants into active vs expired at `now`. Order within
/// each bucket follows the file's order (stable, hand-edit-friendly).
pub fn partition(file: &GrantsFile, now: DateTime<Utc>) -> GrantPartition<'_> {
    let mut active = Vec::new();
    let mut expired = Vec::new();
    for g in &file.grants {
        if g.is_expired(now) {
            expired.push(g);
        } else {
            active.push(g);
        }
    }
    GrantPartition { active, expired }
}

/// Grants that have reached their deadline at `now` — the set the
/// orchestrator must drop.
pub fn expired_grants(file: &GrantsFile, now: DateTime<Utc>) -> Vec<&Grant> {
    partition(file, now).expired
}

/// The *active* (not-yet-expired) grant for `project_path`, if any.
/// An expired grant returns `None` — from the UI's perspective an
/// expired-but-not-yet-pruned grant is no longer in effect, and the
/// hook agrees (`hook::covering_grant` skips it too).
pub fn active_grant<'a>(
    file: &'a GrantsFile,
    project_path: &str,
    now: DateTime<Utc>,
) -> Option<&'a Grant> {
    file.find(project_path).filter(|g| !g.is_expired(now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
    }

    fn grant(path: &str, granted: i64, expires: i64) -> Grant {
        Grant {
            project_path: path.to_string(),
            granted_at: ts(granted),
            expires_at: Some(ts(expires)),
        }
    }

    fn file(grants: Vec<Grant>) -> GrantsFile {
        GrantsFile {
            grants,
            ..GrantsFile::default()
        }
    }

    #[test]
    fn partition_splits_active_and_expired() {
        let f = file(vec![
            grant("/p/active", 0, 7200),
            grant("/p/expired", 0, 100),
            grant("/p/also-active", 0, 9999),
        ]);
        let p = partition(&f, ts(200));
        assert_eq!(
            p.active
                .iter()
                .map(|g| g.project_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/p/active", "/p/also-active"]
        );
        assert_eq!(
            p.expired
                .iter()
                .map(|g| g.project_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/p/expired"]
        );
    }

    #[test]
    fn expired_grants_is_inclusive_of_the_deadline() {
        let f = file(vec![grant("/p/a", 0, 100)]);
        assert!(expired_grants(&f, ts(99)).is_empty());
        assert_eq!(expired_grants(&f, ts(100)).len(), 1);
        assert_eq!(expired_grants(&f, ts(101)).len(), 1);
    }

    #[test]
    fn active_grant_returns_none_for_expired() {
        let f = file(vec![grant("/p/a", 0, 100)]);
        assert!(active_grant(&f, "/p/a", ts(50)).is_some());
        assert!(active_grant(&f, "/p/a", ts(100)).is_none());
    }

    #[test]
    fn active_grant_returns_none_for_unknown_path() {
        let f = file(vec![grant("/p/a", 0, 7200)]);
        assert!(active_grant(&f, "/p/missing", ts(0)).is_none());
    }

    #[test]
    fn empty_file_partitions_to_empty() {
        let f = file(vec![]);
        let p = partition(&f, ts(0));
        assert!(p.active.is_empty() && p.expired.is_empty());
    }
}

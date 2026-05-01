//! Persistent notification history.
//!
//! Toasts and OS-dispatched notifications used to be fire-and-forget —
//! a toast left only `lastDismissed` in the status bar; an OS notif
//! was the OS Notification Center's problem after dispatch. Users had
//! no in-app way to ask "what did Claudepot just tell me?".
//!
//! This module backs a small ring buffer (cap [`MAX_ENTRIES`], default
//! 500) on disk at `~/.claudepot/notifications.json`. Both surfaces
//! ([`crate::commands_notification`] from the renderer's `pushToast`
//! and `dispatchOsNotification`) append into the same log, and the
//! shell's bell-icon popover reads from it.
//!
//! Design tradeoffs:
//!
//! - **JSON not SQLite.** Max payload is ~100 KB (500 × ~200 B). A new
//!   SQLite file would be infrastructure tax for a fixed schema with no
//!   query complexity. JSON is also easier to inspect / delete by hand.
//! - **Single mutex around the entire `Inner`.** Append rate is
//!   user-paced (one notification per UX event), so the lock is never
//!   contended. Read-after-write needs the same lock anyway because
//!   the renderer might list immediately after appending.
//! - **Write-through, no dirty flag.** Every mutation writes the full
//!   JSON. Cost at 500 entries is single-digit milliseconds; the
//!   simplicity beats a debounce path that could lose entries on
//!   crash.
//! - **Atomic write via [`crate::fs_utils::atomic_write`].** Same
//!   temp-then-rename pattern used by every other persisted file in
//!   the project; never partial state on disk.
//! - **Corrupt file → empty log.** If the JSON fails to parse on
//!   open, we treat the on-disk store as wiped and start fresh.
//!   Better than wedging the bell forever; the file gets overwritten
//!   on the next append. The previous content is moved aside to
//!   `notifications.json.corrupt` for forensics.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::fs_utils::atomic_write;

/// Hard ring-buffer cap. 500 × ~200 B = ~100 KB on disk in the worst
/// case. Bump if traffic grows past one notification per minute on a
/// machine that stays open for a week (the practical breaking point
/// for "look back over the weekend"); the JSON serialization stays
/// linear-time in entry count.
pub const MAX_ENTRIES: usize = 500;

/// Where the notification surfaced. Both surfaces are dispatched
/// independently — a single user-facing event can be both a toast
/// (when the window is focused) AND an OS notification (when it's
/// not). The capture sites in `notify.ts` and `useToasts.ts` log
/// each surface separately so the log accurately reflects what the
/// user was actually shown, not what was logically intended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSource {
    /// In-app toast via the `useToasts` hook.
    Toast,
    /// OS desktop notification via `dispatchOsNotification`.
    Os,
}

/// User-facing severity. Matches the shape `pushToast` already uses
/// in the frontend (`"info" | "error"`) plus an explicit `notice`
/// for OS-only signals that aren't errors but aren't routine info
/// either (auth-rejected banners, long-running op completion). The
/// frontend coerces toast `kind` directly; OS dispatchers pick
/// explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationKind {
    Info,
    Notice,
    Error,
}

/// One entry in the log. `id` is monotonic per-process — it's only
/// used to discriminate "newer than the last seen" without timestamp
/// ambiguity (two entries inside the same millisecond), and to power
/// the React `key`. `target` mirrors the renderer's
/// `NotificationTarget` discriminator so a click on a logged entry
/// can route the user the same way a fresh notification would.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationEntry {
    /// Monotonic per-process id. Reset to (max + 1) on load so newly
    /// appended entries always sort after persisted ones.
    pub id: u64,
    /// Wall-clock millis since epoch. The renderer formats this for
    /// display; the back end never compares two entries by ts (use
    /// `id` instead).
    pub ts_ms: i64,
    pub source: NotificationSource,
    pub kind: NotificationKind,
    pub title: String,
    /// Empty string when the surface didn't carry a body.
    pub body: String,
    /// Opaque JSON value — the renderer round-trips its
    /// `NotificationTarget` through here without us needing to lift
    /// every variant into Rust types. Stored as a JSON value rather
    /// than a string so the renderer can deserialize it directly
    /// without an extra parse step. `null` when the surface had no
    /// click target.
    #[serde(default)]
    pub target: serde_json::Value,
}

/// Filter applied to [`NotificationLog::list`]. All fields are
/// optional; `None` means "don't filter on that axis." Mirrors the
/// frontend `NotificationLogFilter` shape.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationLogFilter {
    /// Empty Vec means "any kind." Wrapping in Option would make the
    /// camelCase transit from JS noisier without buying anything.
    #[serde(default)]
    pub kinds: Vec<NotificationKind>,
    /// `None` = both surfaces.
    #[serde(default)]
    pub source: Option<NotificationSource>,
    /// Lower bound on `ts_ms`; entries strictly older are excluded.
    /// `None` = no lower bound (the ring-buffer cap is the only
    /// floor).
    #[serde(default)]
    pub since_ms: Option<i64>,
    /// Substring match against `title` + `body`, case-insensitive.
    /// Empty string treated as `None`.
    #[serde(default)]
    pub query: Option<String>,
}

/// Sort order for [`NotificationLog::list`].
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortOrder {
    /// Newest first — default; matches the user expectation of "most
    /// recent at the top."
    #[default]
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedDoc {
    /// Highest id seen by any reader. Entries with `id > last_seen_id`
    /// are unread; the bell badge renders that count.
    #[serde(default)]
    last_seen_id: u64,
    #[serde(default)]
    entries: Vec<NotificationEntry>,
}

struct Inner {
    path: PathBuf,
    /// Front = oldest, back = newest. VecDeque so eviction is O(1).
    entries: VecDeque<NotificationEntry>,
    next_id: u64,
    last_seen_id: u64,
}

/// Persistent ring-buffer log of dispatched notifications. Cheap to
/// construct (one file read, one parse). Concurrent callers go
/// through a single mutex — appends are user-paced so contention is
/// never a real concern.
pub struct NotificationLog {
    inner: Mutex<Inner>,
}

impl NotificationLog {
    /// Open the log at `path`, creating the parent directory if
    /// necessary. A missing file is treated as an empty log; a
    /// corrupt file is moved aside to `<path>.corrupt` and the log
    /// starts empty. Both cases are non-fatal — better than wedging
    /// the bell on a parse glitch.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let doc = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<PersistedDoc>(&bytes) {
                Ok(d) => d,
                Err(_) => {
                    // Move the corrupt file aside for forensics.
                    // `rename` is best-effort; a failure here only
                    // means the next mutation overwrites it cleanly.
                    let corrupt = path.with_extension("json.corrupt");
                    let _ = std::fs::rename(&path, &corrupt);
                    PersistedDoc::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PersistedDoc::default(),
            Err(e) => return Err(e),
        };

        let mut entries: VecDeque<NotificationEntry> = doc.entries.into();
        // Defense against a hand-edited file that exceeds the cap —
        // truncate from the front so we keep the newest tail.
        while entries.len() > MAX_ENTRIES {
            entries.pop_front();
        }
        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        // Clamp `last_seen_id` to a sane range. A persisted value
        // higher than the current max can happen if the on-disk tail
        // got truncated by a corrupt-file fallback; clamp prevents the
        // bell from going permanently silent.
        let last_seen_id = doc.last_seen_id.min(next_id.saturating_sub(1));

        Ok(Self {
            inner: Mutex::new(Inner {
                path,
                entries,
                next_id,
                last_seen_id,
            }),
        })
    }

    /// Append a new entry. Assigns `id` and `ts_ms` server-side so the
    /// renderer can't lie about ordering. Returns the assigned `id`
    /// for the renderer to thread back into "did the dispatch I just
    /// fired actually land?" if it cares.
    pub fn append(
        &self,
        source: NotificationSource,
        kind: NotificationKind,
        title: String,
        body: String,
        target: serde_json::Value,
    ) -> std::io::Result<u64> {
        let mut g = self.inner.lock().expect("notification log mutex poisoned");
        let id = g.next_id;
        g.next_id = g.next_id.saturating_add(1);
        let ts_ms = chrono::Utc::now().timestamp_millis();
        g.entries.push_back(NotificationEntry {
            id,
            ts_ms,
            source,
            kind,
            title,
            body,
            target,
        });
        while g.entries.len() > MAX_ENTRIES {
            g.entries.pop_front();
        }
        Self::persist_locked(&g)?;
        Ok(id)
    }

    /// Return entries matching `filter`, in `order`. Cap the result
    /// at `limit` (default [`MAX_ENTRIES`]). Filtering is in-memory
    /// — at this scale (<= 500 entries) iterating the whole vec is
    /// cheaper than maintaining secondary indexes.
    pub fn list(
        &self,
        filter: &NotificationLogFilter,
        order: SortOrder,
        limit: Option<usize>,
    ) -> Vec<NotificationEntry> {
        let g = self.inner.lock().expect("notification log mutex poisoned");
        let q_lower = filter
            .query
            .as_deref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        let cap = limit.unwrap_or(MAX_ENTRIES);

        let matches = |e: &NotificationEntry| -> bool {
            if !filter.kinds.is_empty() && !filter.kinds.contains(&e.kind) {
                return false;
            }
            if let Some(s) = filter.source {
                if e.source != s {
                    return false;
                }
            }
            if let Some(since) = filter.since_ms {
                if e.ts_ms < since {
                    return false;
                }
            }
            if let Some(q) = q_lower.as_ref() {
                let in_title = e.title.to_lowercase().contains(q);
                let in_body = e.body.to_lowercase().contains(q);
                if !in_title && !in_body {
                    return false;
                }
            }
            true
        };

        let take = |it: Box<dyn Iterator<Item = &NotificationEntry> + '_>| {
            it.filter(|e| matches(e)).take(cap).cloned().collect()
        };

        match order {
            SortOrder::NewestFirst => take(Box::new(g.entries.iter().rev())),
            SortOrder::OldestFirst => take(Box::new(g.entries.iter())),
        }
    }

    /// Mark every current entry as seen. The next [`unread_count`]
    /// returns 0 until a fresh entry lands.
    pub fn mark_all_read(&self) -> std::io::Result<()> {
        let mut g = self.inner.lock().expect("notification log mutex poisoned");
        let highest = g.entries.iter().map(|e| e.id).max().unwrap_or(0);
        if g.last_seen_id == highest {
            return Ok(());
        }
        g.last_seen_id = highest;
        Self::persist_locked(&g)
    }

    /// Wipe every entry and reset the id counter to 1. The file is
    /// rewritten as an empty doc rather than deleted so subsequent
    /// reads don't hit the not-found branch in `open`.
    pub fn clear(&self) -> std::io::Result<()> {
        let mut g = self.inner.lock().expect("notification log mutex poisoned");
        g.entries.clear();
        g.next_id = 1;
        g.last_seen_id = 0;
        Self::persist_locked(&g)
    }

    /// Number of entries with `id > last_seen_id`. Drives the bell
    /// badge.
    pub fn unread_count(&self) -> u32 {
        let g = self.inner.lock().expect("notification log mutex poisoned");
        // u32 is wide enough — MAX_ENTRIES is 500 and the badge
        // would render "99+" past two digits anyway.
        g.entries.iter().filter(|e| e.id > g.last_seen_id).count() as u32
    }

    /// Total entry count.
    pub fn len(&self) -> usize {
        let g = self.inner.lock().expect("notification log mutex poisoned");
        g.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Persist the current state to disk. Caller must hold the inner
    /// lock; we re-read from `g` so the on-disk file always matches
    /// the in-memory state at the lock-release point.
    fn persist_locked(g: &Inner) -> std::io::Result<()> {
        let doc = PersistedDoc {
            last_seen_id: g.last_seen_id,
            entries: g.entries.iter().cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("notification_log serialize: {e}"),
            )
        })?;
        atomic_write(&g.path, &bytes)
    }
}

/// Default on-disk path for the notification log.
pub fn default_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join("notifications.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp_log() -> (tempfile::TempDir, NotificationLog) {
        let dir = tempfile::tempdir().unwrap();
        let log = NotificationLog::open(dir.path().join("notifications.json")).unwrap();
        (dir, log)
    }

    fn append_info(log: &NotificationLog, title: &str) -> u64 {
        log.append(
            NotificationSource::Toast,
            NotificationKind::Info,
            title.to_string(),
            String::new(),
            serde_json::Value::Null,
        )
        .unwrap()
    }

    #[test]
    fn test_open_missing_file_is_empty() {
        let (_d, log) = tmp_log();
        assert_eq!(log.len(), 0);
        assert_eq!(log.unread_count(), 0);
    }

    #[test]
    fn test_append_assigns_monotonic_ids() {
        let (_d, log) = tmp_log();
        let a = append_info(&log, "first");
        let b = append_info(&log, "second");
        assert!(b > a, "ids must be monotonic: {a} → {b}");
    }

    #[test]
    fn test_append_evicts_at_cap() {
        let (_d, log) = tmp_log();
        for i in 0..(MAX_ENTRIES + 5) {
            append_info(&log, &format!("entry-{i}"));
        }
        assert_eq!(log.len(), MAX_ENTRIES);
        // The oldest five must be gone — verify by listing
        // oldest-first and inspecting the head title.
        let head = log.list(
            &NotificationLogFilter::default(),
            SortOrder::OldestFirst,
            Some(1),
        );
        assert_eq!(head.first().unwrap().title, "entry-5");
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        {
            let log = NotificationLog::open(path.clone()).unwrap();
            append_info(&log, "alpha");
            append_info(&log, "beta");
            log.mark_all_read().unwrap();
            append_info(&log, "gamma");
        }
        let log2 = NotificationLog::open(path).unwrap();
        assert_eq!(log2.len(), 3);
        // After reload: only `gamma` is unread because `mark_all_read`
        // ran between `beta` and `gamma`.
        assert_eq!(log2.unread_count(), 1);
        // ID continuity — the next append must not collide with a
        // persisted id.
        let new_id = append_info(&log2, "delta");
        let all = log2.list(
            &NotificationLogFilter::default(),
            SortOrder::OldestFirst,
            None,
        );
        let unique_ids: std::collections::HashSet<u64> = all.iter().map(|e| e.id).collect();
        assert_eq!(
            unique_ids.len(),
            all.len(),
            "ids must be unique post-reload"
        );
        assert!(new_id > all.iter().map(|e| e.id).max().unwrap() - 1);
    }

    #[test]
    fn test_corrupt_file_falls_back_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        std::fs::write(&path, b"{not valid json").unwrap();
        let log = NotificationLog::open(path.clone()).unwrap();
        assert_eq!(log.len(), 0);
        // The corrupt file should have been moved aside so we can
        // inspect it later.
        assert!(path.with_extension("json.corrupt").exists());
    }

    #[test]
    fn test_filter_by_kind() {
        let (_d, log) = tmp_log();
        log.append(
            NotificationSource::Toast,
            NotificationKind::Info,
            "ok".into(),
            String::new(),
            json!(null),
        )
        .unwrap();
        log.append(
            NotificationSource::Toast,
            NotificationKind::Error,
            "boom".into(),
            String::new(),
            json!(null),
        )
        .unwrap();
        let f = NotificationLogFilter {
            kinds: vec![NotificationKind::Error],
            ..Default::default()
        };
        let r = log.list(&f, SortOrder::NewestFirst, None);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "boom");
    }

    #[test]
    fn test_filter_by_source() {
        let (_d, log) = tmp_log();
        log.append(
            NotificationSource::Toast,
            NotificationKind::Info,
            "t".into(),
            String::new(),
            json!(null),
        )
        .unwrap();
        log.append(
            NotificationSource::Os,
            NotificationKind::Info,
            "o".into(),
            String::new(),
            json!(null),
        )
        .unwrap();
        let f = NotificationLogFilter {
            source: Some(NotificationSource::Os),
            ..Default::default()
        };
        let r = log.list(&f, SortOrder::NewestFirst, None);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "o");
    }

    #[test]
    fn test_filter_by_since_ms() {
        let (_d, log) = tmp_log();
        append_info(&log, "old");
        // Sleep a hair so ts_ms differs; in practice the renderer
        // sets the since to a past second so this is always true.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let cutoff = chrono::Utc::now().timestamp_millis();
        std::thread::sleep(std::time::Duration::from_millis(2));
        append_info(&log, "new");
        let f = NotificationLogFilter {
            since_ms: Some(cutoff),
            ..Default::default()
        };
        let r = log.list(&f, SortOrder::NewestFirst, None);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "new");
    }

    #[test]
    fn test_filter_by_query_case_insensitive() {
        let (_d, log) = tmp_log();
        log.append(
            NotificationSource::Toast,
            NotificationKind::Info,
            "Switched account".into(),
            "to bob@example.com".into(),
            json!(null),
        )
        .unwrap();
        log.append(
            NotificationSource::Toast,
            NotificationKind::Info,
            "Repair complete".into(),
            String::new(),
            json!(null),
        )
        .unwrap();
        let f = NotificationLogFilter {
            query: Some("BOB".into()),
            ..Default::default()
        };
        let r = log.list(&f, SortOrder::NewestFirst, None);
        assert_eq!(r.len(), 1);
        assert!(r[0].title.contains("Switched"));
    }

    #[test]
    fn test_sort_order_newest_first_is_default() {
        let (_d, log) = tmp_log();
        append_info(&log, "first");
        append_info(&log, "second");
        let r = log.list(
            &NotificationLogFilter::default(),
            SortOrder::NewestFirst,
            None,
        );
        assert_eq!(r[0].title, "second");
        let r = log.list(
            &NotificationLogFilter::default(),
            SortOrder::OldestFirst,
            None,
        );
        assert_eq!(r[0].title, "first");
    }

    #[test]
    fn test_mark_all_read_clears_unread_count() {
        let (_d, log) = tmp_log();
        append_info(&log, "x");
        append_info(&log, "y");
        assert_eq!(log.unread_count(), 2);
        log.mark_all_read().unwrap();
        assert_eq!(log.unread_count(), 0);
        // A fresh entry after mark-all-read goes back to unread.
        append_info(&log, "z");
        assert_eq!(log.unread_count(), 1);
    }

    #[test]
    fn test_clear_wipes_state() {
        let (_d, log) = tmp_log();
        append_info(&log, "x");
        append_info(&log, "y");
        log.clear().unwrap();
        assert_eq!(log.len(), 0);
        assert_eq!(log.unread_count(), 0);
        // Ids reset to 1 — the next append starts the counter over so
        // a totally cleared log is indistinguishable from a fresh
        // install on the next reload.
        let next = append_info(&log, "z");
        assert_eq!(next, 1);
    }

    #[test]
    fn test_clamp_last_seen_id_on_overflow() {
        // Hand-craft a corrupt-but-parseable doc with last_seen_id
        // higher than any entry — open() must clamp so the bell
        // doesn't go permanently silent.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let bad = serde_json::json!({
            "last_seen_id": 9_999_999u64,
            "entries": [{
                "id": 3u64,
                "ts_ms": 1_700_000_000_000i64,
                "source": "toast",
                "kind": "info",
                "title": "x",
                "body": "",
                "target": null,
            }]
        });
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        let log = NotificationLog::open(path).unwrap();
        // Three was the highest; new appends start at 4.
        let id = append_info(&log, "y");
        assert_eq!(id, 4);
        // Existing entry must be considered seen, the new one unread.
        assert_eq!(log.unread_count(), 1);
    }

    #[test]
    fn test_target_roundtrips() {
        let (_d, log) = tmp_log();
        let target = serde_json::json!({
            "kind": "app",
            "route": { "section": "accounts", "email": "a@b.c" }
        });
        log.append(
            NotificationSource::Os,
            NotificationKind::Notice,
            "click me".into(),
            "to switch".into(),
            target.clone(),
        )
        .unwrap();
        let r = log.list(
            &NotificationLogFilter::default(),
            SortOrder::NewestFirst,
            None,
        );
        assert_eq!(r[0].target, target);
    }
}

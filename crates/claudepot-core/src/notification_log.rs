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
use crate::notifications::{Category, Priority, Surface};
use crate::session_export::redact_secrets;

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
///
/// **Schema evolution (Phase 0 of the refactor).** Three fields were
/// added for the routing redesign — each with serde `default` so
/// pre-migration entries round-trip cleanly:
///
/// - `category` / `priority` — the routing axes recorded by the new
///   `emit()` facade. Pre-migration entries have `None`; the bell
///   popover treats those as "Other / Legacy" at filter time.
/// - `surfaces_requested` — the surface set the dispatcher asked for,
///   computed after user-pref filtering but before delivery gates.
/// - `surfaces_delivered` — the surfaces that actually rendered.
///   Toasts always deliver if requested; OS banners can be dropped
///   by focus/permission/rate gate.
///
/// The legacy `source` field stays for read-only interpretation of
/// pre-migration entries. New code MUST NOT set it; new entries
/// carry their surface in `surfaces_*` instead. See the bell
/// filter's source-compat shim for how legacy rows are matched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationEntry {
    /// Monotonic per-process id. Reset to (max + 1) on load so newly
    /// appended entries always sort after persisted ones.
    pub id: u64,
    /// Wall-clock millis since epoch. The renderer formats this for
    /// display; the back end never compares two entries by ts (use
    /// `id` instead).
    pub ts_ms: i64,
    /// **Deprecated** in favor of `surfaces_requested` /
    /// `surfaces_delivered`. Set to `Some(_)` on pre-Phase-0
    /// entries; new code writes `None` and uses the explicit
    /// surface vectors instead. Kept as `Option<_>` so the on-disk
    /// JSON of pre-migration installs round-trips.
    #[serde(default)]
    pub source: Option<NotificationSource>,
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
    /// Routing category. `None` on pre-Phase-0 entries.
    #[serde(default)]
    pub category: Option<Category>,
    /// Routing priority. `None` on pre-Phase-0 entries.
    #[serde(default)]
    pub priority: Option<Priority>,
    /// Surfaces routing asked for AFTER pref filtering, BEFORE
    /// delivery gates. Empty vec when the category was muted or
    /// when the entry predates Phase 0.
    #[serde(default)]
    pub surfaces_requested: Vec<Surface>,
    /// Surfaces that actually rendered. Filled after dispatch.
    /// Empty vec for pre-Phase-0 entries.
    #[serde(default)]
    pub surfaces_delivered: Vec<Surface>,
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
    /// When true, `persist_locked` returns Ok without touching disk.
    /// Set by `in_memory_only()` for the boot-fallback path so a
    /// degraded log doesn't spam attempted writes against a path
    /// that's already known to be unreachable.
    volatile: bool,
}

/// Mutex-poison recovery — `unwrap_or_else(PoisonError::into_inner)`
/// in one place so every method handles a poisoned lock the same way:
/// recover via `into_inner()` after a `tracing::warn!`. The
/// notification log is advisory state (no correctness guarantees
/// ride on it), so panicking the process — or worse, panicking every
/// subsequent caller — is the wrong tradeoff. The previous
/// `.expect()` per call site violated `.claude/rules/rust-conventions.md`'s
/// "no unwrap/expect in core" gate.
///
/// Note: a poisoned mutex stays poisoned for the process lifetime,
/// so every subsequent acquire under poison logs another warn. The
/// noise is informative — it tells operators that an earlier panic
/// is still unresolved rather than silently masking it. If the
/// volume becomes a problem we can hoist a `OnceCell` to log once;
/// for now the explicit per-call signal is correct.
fn lock_inner(m: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::warn!(
                "notification_log: mutex was poisoned by an earlier panic; \
                 recovering with under-lock data (advisory state, no \
                 correctness guarantees ride on it)"
            );
            poisoned.into_inner()
        }
    }
}

/// Persistent ring-buffer log of dispatched notifications. Cheap to
/// construct (one file read, one parse). Concurrent callers go
/// through a single mutex — appends are user-paced so contention is
/// never a real concern.
pub struct NotificationLog {
    inner: Mutex<Inner>,
}

impl NotificationLog {
    /// Build a volatile in-memory-only log — appends and reads work,
    /// but persistence is skipped entirely (the `volatile` flag
    /// short-circuits `persist_locked`). Used as the last-resort
    /// boot fallback when both the real and temp-dir file paths
    /// refused to open. The bell still works for the current
    /// process; nothing survives a restart.
    ///
    /// The `path` field is kept (set to a unique per-process value)
    /// so the Inner shape is uniform across both code paths, but
    /// nothing ever writes to it.
    pub fn in_memory_only() -> Self {
        let path =
            std::env::temp_dir().join(format!("claudepot-notif-volatile-{}", std::process::id()));
        Self {
            inner: Mutex::new(Inner {
                path,
                entries: VecDeque::new(),
                next_id: 1,
                last_seen_id: 0,
                volatile: true,
            }),
        }
    }

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
                volatile: false,
            }),
        })
    }

    /// Append a new entry via the legacy single-surface API. Assigns
    /// `id` and `ts_ms` server-side so the renderer can't lie about
    /// ordering. Returns the assigned `id` for the renderer to thread
    /// back into "did the dispatch I just fired actually land?" if it
    /// cares.
    ///
    /// **Migration note (Phase 0):** New code should prefer
    /// [`NotificationLog::append_routed`] which records the routing
    /// metadata (category, priority, surfaces requested/delivered).
    /// This shape exists for back-compat — every call from the
    /// renderer's old `notificationLogAppend` IPC and the
    /// `service_status_watcher` direct path still works unchanged.
    /// Phase 1 introduces the new path; Phase 3 retires this one.
    pub fn append(
        &self,
        source: NotificationSource,
        kind: NotificationKind,
        title: String,
        body: String,
        target: serde_json::Value,
    ) -> std::io::Result<u64> {
        // Defense-in-depth: redact `sk-ant-*` tokens before they
        // touch the persistent log. The renderer also redacts at
        // emit time, but a panic backtrace or third-party crate's
        // error string can route around the renderer (Rust-side
        // direct writes from service_status_watcher /
        // usage_watcher). `.claude/rules/design.md` non-negotiable.
        let title = redact_secrets(&title);
        let body = redact_secrets(&body);
        let mut g = lock_inner(&self.inner);
        let id = g.next_id;
        g.next_id = g.next_id.saturating_add(1);
        let ts_ms = chrono::Utc::now().timestamp_millis();
        g.entries.push_back(NotificationEntry {
            id,
            ts_ms,
            source: Some(source),
            kind,
            title,
            body,
            target,
            category: None,
            priority: None,
            surfaces_requested: Vec::new(),
            surfaces_delivered: Vec::new(),
        });
        while g.entries.len() > MAX_ENTRIES {
            g.entries.pop_front();
        }
        Self::persist_locked(&g)?;
        Ok(id)
    }

    /// Append a new entry recording full routing metadata. The
    /// Phase 1 [`crate::notifications::route`] pipeline calls this
    /// after computing `surfaces_requested` and (optionally) after
    /// the OS dispatcher has reported `surfaces_delivered`.
    ///
    /// Legacy `source` is set to `None` — new entries are
    /// authoritatively described by `surfaces_*`, and the bell
    /// filter treats legacy rows (`source: Some`) and new rows
    /// (`source: None`, `surfaces_*` populated) symmetrically.
    #[allow(clippy::too_many_arguments)]
    pub fn append_routed(
        &self,
        category: Category,
        priority: Priority,
        kind: NotificationKind,
        title: String,
        body: String,
        target: serde_json::Value,
        surfaces_requested: Vec<Surface>,
        surfaces_delivered: Vec<Surface>,
    ) -> std::io::Result<u64> {
        // Defense-in-depth redaction at the persistence boundary —
        // same rationale as `append()`. See that fn for the policy.
        let title = redact_secrets(&title);
        let body = redact_secrets(&body);
        // Reject `surfaces_delivered` entries that weren't in
        // `surfaces_requested` — drops the bug class where a renderer
        // marks a surface as delivered when it was never requested.
        // We trim defensively rather than erroring so a borderline
        // race (e.g. renderer asks for OS, dispatcher reports
        // delivery, pref toggle flipped mid-flight) still records the
        // best truth we have.
        let surfaces_delivered: Vec<Surface> = surfaces_delivered
            .into_iter()
            .filter(|s| surfaces_requested.contains(s))
            .collect();
        let mut g = lock_inner(&self.inner);
        let id = g.next_id;
        g.next_id = g.next_id.saturating_add(1);
        let ts_ms = chrono::Utc::now().timestamp_millis();
        g.entries.push_back(NotificationEntry {
            id,
            ts_ms,
            source: None,
            kind,
            title,
            body,
            target,
            category: Some(category),
            priority: Some(priority),
            surfaces_requested,
            surfaces_delivered,
        });
        while g.entries.len() > MAX_ENTRIES {
            g.entries.pop_front();
        }
        Self::persist_locked(&g)?;
        Ok(id)
    }

    /// Mark a previously-appended entry as delivered on `surface`.
    /// Used by the OS dispatcher to report back after a focus / rate
    /// gate decision. Best-effort: if `id` is no longer in the ring
    /// buffer (evicted by a burst), this returns `Ok(false)` and the
    /// caller doesn't care — the dispatch already happened. Returns
    /// `Ok(true)` when the entry was updated.
    pub fn mark_delivered(&self, id: u64, surface: Surface) -> std::io::Result<bool> {
        let mut g = lock_inner(&self.inner);
        let entry = g.entries.iter_mut().find(|e| e.id == id);
        let Some(entry) = entry else {
            return Ok(false);
        };
        // Reject delivery marks for surfaces that weren't in
        // `surfaces_requested` — a renderer bug or malicious IPC
        // shouldn't be able to claim "we delivered on this surface"
        // when routing never asked for it. Caller treats `false` as
        // "entry not updated"; the dispatcher already swallows that
        // outcome (best-effort post-confirmation).
        if !entry.surfaces_requested.contains(&surface) {
            return Ok(false);
        }
        if entry.surfaces_delivered.contains(&surface) {
            // Idempotent; no persist needed.
            return Ok(true);
        }
        entry.surfaces_delivered.push(surface);
        Self::persist_locked(&g)?;
        Ok(true)
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
        let g = lock_inner(&self.inner);
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
                // Legacy entries carry the surface in `e.source`;
                // post-Phase-0 entries carry it in
                // `e.surfaces_requested` / `e.surfaces_delivered`.
                // Match a filter against EITHER place so legacy and
                // routed rows participate symmetrically. The
                // mapping below treats `surfaces_*` Toast/OsBanner
                // as evidence the row was a Toast/Os entry.
                let legacy_match = e.source == Some(s);
                let routed_match = match s {
                    NotificationSource::Toast => {
                        e.surfaces_requested.contains(&Surface::Toast)
                            || e.surfaces_delivered.contains(&Surface::Toast)
                    }
                    NotificationSource::Os => {
                        e.surfaces_requested.contains(&Surface::OsBanner)
                            || e.surfaces_delivered.contains(&Surface::OsBanner)
                    }
                };
                if !legacy_match && !routed_match {
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
        let mut g = lock_inner(&self.inner);
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
        let mut g = lock_inner(&self.inner);
        g.entries.clear();
        g.next_id = 1;
        g.last_seen_id = 0;
        Self::persist_locked(&g)
    }

    /// Number of entries with `id > last_seen_id`. Drives the bell
    /// badge.
    pub fn unread_count(&self) -> u32 {
        let g = lock_inner(&self.inner);
        // u32 is wide enough — MAX_ENTRIES is 500 and the badge
        // would render "99+" past two digits anyway.
        g.entries.iter().filter(|e| e.id > g.last_seen_id).count() as u32
    }

    /// Phase 5: count of unread entries at-or-above the given
    /// priority. Used by the tray badge so a window-closed user
    /// sees the icon light up when a P0 / P1 / P2 entry lands —
    /// without P3 ambient writes from CC overwriting CLAUDE.md
    /// flooding the badge.
    ///
    /// Legacy entries (pre-Phase-0) have `priority: None`. They're
    /// treated as P2-equivalent (the historical "matters to the
    /// user" tier) so the badge doesn't go silent for installs
    /// that haven't yet generated any routed entries.
    pub fn unread_count_at_or_above(&self, min: Priority) -> u32 {
        let g = lock_inner(&self.inner);
        let rank = priority_rank;
        let threshold = rank(min);
        g.entries
            .iter()
            .filter(|e| e.id > g.last_seen_id)
            .filter(|e| {
                let p = e.priority.unwrap_or(Priority::P2Acknowledge);
                rank(p) <= threshold
            })
            .count() as u32
    }

    /// Total entry count.
    pub fn len(&self) -> usize {
        let g = lock_inner(&self.inner);
        g.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Persist the current state to disk. Caller must hold the inner
    /// lock; we re-read from `g` so the on-disk file always matches
    /// the in-memory state at the lock-release point.
    ///
    /// No-op when `g.volatile` is set (the boot-fallback degraded
    /// log) — see [`NotificationLog::in_memory_only`] for the
    /// rationale.
    fn persist_locked(g: &Inner) -> std::io::Result<()> {
        if g.volatile {
            return Ok(());
        }
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

/// Internal: rank priorities so "at-or-above" comparisons work.
/// Lower rank = higher urgency. P0 = 0, P3 = 3.
fn priority_rank(p: Priority) -> u32 {
    match p {
        Priority::P0Blocking => 0,
        Priority::P1Stalled => 1,
        Priority::P2Acknowledge => 2,
        Priority::P3Ambient => 3,
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

    // ── Phase 0 schema-evolution tests ─────────────────────────────

    #[test]
    fn test_legacy_entry_format_roundtrips_without_new_fields() {
        // A pre-Phase-0 entry on disk has `source: "toast"` and lacks
        // category/priority/surfaces_*. The serde defaults must make
        // it readable; the new fields must be empty/None.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        let legacy = serde_json::json!({
            "last_seen_id": 0u64,
            "entries": [{
                "id": 1u64,
                "ts_ms": 1_700_000_000_000i64,
                "source": "os",
                "kind": "notice",
                "title": "legacy banner",
                "body": "",
                "target": null,
            }]
        });
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let log = NotificationLog::open(path).unwrap();
        let r = log.list(
            &NotificationLogFilter::default(),
            SortOrder::NewestFirst,
            None,
        );
        assert_eq!(r.len(), 1);
        let e = &r[0];
        assert_eq!(e.title, "legacy banner");
        assert_eq!(e.source, Some(NotificationSource::Os));
        assert_eq!(e.category, None);
        assert_eq!(e.priority, None);
        assert!(e.surfaces_requested.is_empty());
        assert!(e.surfaces_delivered.is_empty());
    }

    #[test]
    fn test_append_routed_populates_new_fields() {
        use crate::notifications::{Category, Priority, Surface};
        let (_d, log) = tmp_log();
        let id = log
            .append_routed(
                Category::UsageThreshold,
                Priority::P1Stalled,
                NotificationKind::Notice,
                "Near cap".into(),
                "90% of weekly cap".into(),
                serde_json::Value::Null,
                vec![Surface::OsBanner],
                vec![Surface::OsBanner],
            )
            .unwrap();
        let r = log.list(
            &NotificationLogFilter::default(),
            SortOrder::NewestFirst,
            None,
        );
        let e = r.iter().find(|e| e.id == id).unwrap();
        assert_eq!(e.source, None);
        assert_eq!(e.category, Some(Category::UsageThreshold));
        assert_eq!(e.priority, Some(Priority::P1Stalled));
        assert_eq!(e.surfaces_requested, vec![Surface::OsBanner]);
        assert_eq!(e.surfaces_delivered, vec![Surface::OsBanner]);
    }

    #[test]
    fn test_append_routed_persists_and_reloads_cleanly() {
        use crate::notifications::{Category, Priority, Surface};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notifications.json");
        {
            let log = NotificationLog::open(path.clone()).unwrap();
            log.append_routed(
                Category::ProjectRenamed,
                Priority::P2Acknowledge,
                NotificationKind::Info,
                "Renamed".into(),
                "old → new".into(),
                serde_json::Value::Null,
                vec![Surface::Toast],
                vec![Surface::Toast],
            )
            .unwrap();
        }
        let log = NotificationLog::open(path).unwrap();
        let r = log.list(
            &NotificationLogFilter::default(),
            SortOrder::NewestFirst,
            None,
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].category, Some(Category::ProjectRenamed));
        assert_eq!(r[0].surfaces_requested, vec![Surface::Toast]);
    }

    #[test]
    fn test_mark_delivered_appends_surface_once() {
        use crate::notifications::{Category, Priority, Surface};
        let (_d, log) = tmp_log();
        let id = log
            .append_routed(
                Category::UsageThreshold,
                Priority::P1Stalled,
                NotificationKind::Notice,
                "x".into(),
                String::new(),
                serde_json::Value::Null,
                vec![Surface::OsBanner],
                Vec::new(), // start undelivered
            )
            .unwrap();
        assert!(log.mark_delivered(id, Surface::OsBanner).unwrap());
        // Idempotent on second call.
        assert!(log.mark_delivered(id, Surface::OsBanner).unwrap());
        let r = log.list(
            &NotificationLogFilter::default(),
            SortOrder::NewestFirst,
            None,
        );
        let e = r.iter().find(|e| e.id == id).unwrap();
        assert_eq!(e.surfaces_delivered, vec![Surface::OsBanner]);
    }

    #[test]
    fn test_mark_delivered_unknown_id_returns_false() {
        use crate::notifications::Surface;
        let (_d, log) = tmp_log();
        // No entries yet — id=42 cannot exist.
        assert!(!log.mark_delivered(42, Surface::Toast).unwrap());
    }

    #[test]
    fn test_unread_count_at_or_above_filters_by_priority() {
        // Tray badge driver: P3 ambient entries (memory writes,
        // config patches) must NOT inflate the badge. Only P0/P1/P2
        // should count when min=P2.
        use crate::notifications::{Category, Priority, Surface};
        let (_d, log) = tmp_log();
        log.append_routed(
            Category::MemoryChanged,
            Priority::P3Ambient,
            NotificationKind::Info,
            "memory write".into(),
            String::new(),
            serde_json::Value::Null,
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        log.append_routed(
            Category::UsageThreshold,
            Priority::P1Stalled,
            NotificationKind::Notice,
            "90%".into(),
            String::new(),
            serde_json::Value::Null,
            vec![Surface::OsBanner],
            vec![Surface::OsBanner],
        )
        .unwrap();
        log.append_routed(
            Category::ProjectRenamed,
            Priority::P2Acknowledge,
            NotificationKind::Info,
            "renamed".into(),
            String::new(),
            serde_json::Value::Null,
            vec![Surface::Toast],
            vec![Surface::Toast],
        )
        .unwrap();
        // All three are unread (no mark_all_read).
        assert_eq!(log.unread_count(), 3);
        // P2-and-above excludes the P3 memory entry.
        assert_eq!(
            log.unread_count_at_or_above(Priority::P2Acknowledge),
            2
        );
        // P1-and-above keeps just the usage threshold.
        assert_eq!(log.unread_count_at_or_above(Priority::P1Stalled), 1);
        // P0-and-above: none of these are blocking.
        assert_eq!(log.unread_count_at_or_above(Priority::P0Blocking), 0);
    }

    #[test]
    fn test_unread_count_at_or_above_treats_legacy_entries_as_p2() {
        // Legacy entries (no `priority` field) should count toward the
        // P2 threshold so the tray badge doesn't silently zero out
        // for installs that haven't generated routed entries yet.
        use crate::notifications::Priority;
        let (_d, log) = tmp_log();
        log.append(
            NotificationSource::Toast,
            NotificationKind::Info,
            "legacy".into(),
            String::new(),
            serde_json::Value::Null,
        )
        .unwrap();
        assert_eq!(log.unread_count_at_or_above(Priority::P2Acknowledge), 1);
        // Legacy entries do NOT count toward P1+ — they could be
        // anything, and rolling them up to "urgent" would defeat
        // the priority filter.
        assert_eq!(log.unread_count_at_or_above(Priority::P1Stalled), 0);
    }

    #[test]
    fn test_legacy_and_routed_coexist_in_same_log() {
        // Mixed-mode log: a pre-Phase-0 entry plus a Phase-0 routed
        // entry. Both must list, both round-trip; the bell-popover
        // source filter is the next layer and stays out of scope
        // here.
        use crate::notifications::{Category, Priority, Surface};
        let (_d, log) = tmp_log();
        // Legacy-style append.
        log.append(
            NotificationSource::Toast,
            NotificationKind::Info,
            "legacy".into(),
            String::new(),
            serde_json::Value::Null,
        )
        .unwrap();
        // Routed append.
        log.append_routed(
            Category::ProjectRenamed,
            Priority::P2Acknowledge,
            NotificationKind::Info,
            "routed".into(),
            String::new(),
            serde_json::Value::Null,
            vec![Surface::Toast],
            vec![Surface::Toast],
        )
        .unwrap();
        let r = log.list(
            &NotificationLogFilter::default(),
            SortOrder::OldestFirst,
            None,
        );
        assert_eq!(r.len(), 2);
        // First row is legacy: source set, category absent.
        assert_eq!(r[0].title, "legacy");
        assert_eq!(r[0].source, Some(NotificationSource::Toast));
        assert_eq!(r[0].category, None);
        // Second row is routed: source absent, category set.
        assert_eq!(r[1].title, "routed");
        assert_eq!(r[1].source, None);
        assert_eq!(r[1].category, Some(Category::ProjectRenamed));
    }
}

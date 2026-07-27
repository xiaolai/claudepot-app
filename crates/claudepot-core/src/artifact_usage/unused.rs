//! "Installed but never observed firing" — the Unused view's business
//! logic.
//!
//! Pure: no I/O, no clock. The caller supplies the installed inventory,
//! the ever-fired key set, the enabled-plugin set, and `now_ms`. That
//! keeps every rule here testable and keeps the renderer to presentation
//! only, per `.claude/rules/architecture.md`.
//!
//! # The four rules, and why each exists
//!
//! 1. **Identity + dedup.** `config_view::discover::collect_plugins`
//!    emits every cached *version* directory as its own plugin root, so
//!    one logical skill can appear a dozen times. Counting those as a
//!    dozen unused artifacts would wildly overstate the problem.
//! 2. **Ever-fired subtraction**, sourced from the durable
//!    `artifact_first_last` ledger — never from `usage_daily`, whose
//!    counters are decremented when a transcript is pruned.
//! 3. **Grace window.** Something installed or edited yesterday has not
//!    had a fair chance to fire.
//! 4. **Disabled plugins are excluded.** A disabled plugin cannot fire,
//!    so listing its artifacts as unused is a category error — the same
//!    class of mistake as counting an MCP-only plugin.
//!
//! # What this deliberately cannot tell you
//!
//! The ledger records "ever observed **by Claudepot**". Usage predating
//! it, or living only in transcripts deleted before its backfill, is
//! invisible. Callers must render "no invocation on record", never
//! "never used".
//!
//! Bundled **MCP servers are not covered at all** — `ArtifactKind` spans
//! skill/hook/agent/command, while MCP calls land in `tool_calls`. A
//! plugin whose value flows through an MCP server will show zero
//! artifact fires. That is why this module answers at *artifact*
//! granularity and must not be rolled up into "this plugin is unused".

use std::collections::{HashMap, HashSet};

use super::identity::artifact_key_for_path;

/// Milliseconds in a day.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Artifacts modified within this window are suppressed.
pub const RECENTLY_MODIFIED_GRACE_DAYS: i64 = 7;

/// One installed artifact file, as handed in by the caller.
#[derive(Debug, Clone)]
pub struct InstalledFile {
    /// `config_view` node kind (`"skill"`, `"agent"`, `"command"`, …).
    pub kind: String,
    /// The owning `FileNode.id`, so the UI can deep-link into the
    /// Config tree (`subRoute = "node:<id>"`). Carried through rather
    /// than recomputed — the hash lives in `config_view`.
    pub node_id: String,
    pub abs_path: String,
    /// Filesystem mtime in ms. **Modification** time, not install time.
    pub modified_ms: i64,
}

/// One row of the Unused view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedArtifact {
    pub kind: String,
    /// Config-tree node id for "Reveal in Config".
    pub node_id: String,
    pub artifact_key: String,
    pub plugin_id: Option<String>,
    /// Trailing key segment — the human-meaningful part.
    pub label: String,
    pub abs_path: String,
    pub modified_ms: i64,
}

/// Result of [`compute_unused`], with the counts needed to make the
/// pane's summary line reconcile.
#[derive(Debug, Clone, Default)]
pub struct UnusedReport {
    pub rows: Vec<UnusedArtifact>,
    /// Distinct installed artifacts considered, after dedup.
    pub installed_count: usize,
    /// Held back by the grace window.
    pub suppressed_recent: usize,
    /// Excluded because their owning plugin is not enabled.
    pub suppressed_disabled: usize,
}

fn label_for(artifact_key: &str) -> String {
    artifact_key
        .rsplit(':')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(artifact_key)
        .to_string()
}

/// Compute the unused set.
///
/// - `ever_fired` — `(kind, artifact_key)` pairs from
///   [`super::store::list_ever_fired`].
/// - `enabled_plugins` — plugin **names** and/or `name@marketplace`
///   specs that are enabled. An artifact whose `plugin_id` is absent
///   from this set is excluded (rule 4). Standalone artifacts (no
///   plugin) are never excluded by this rule.
/// - `project_root` — `None` for a global-only scan. Passing a
///   home-anchored root mis-keys every user artifact; see
///   [`super::identity`].
pub fn compute_unused(
    files: &[InstalledFile],
    ever_fired: &HashSet<(String, String)>,
    enabled_plugins: &HashSet<String>,
    project_root: Option<&str>,
    now_ms: i64,
    grace_days: i64,
) -> UnusedReport {
    // Rule 1 — resolve identity, then dedup on (kind, key), keeping the
    // most recently modified instance so the displayed mtime is live.
    let mut by_identity: HashMap<(String, String), UnusedArtifact> = HashMap::new();
    for f in files {
        let Some(id) = artifact_key_for_path(&f.kind, &f.abs_path, project_root) else {
            continue;
        };
        let key = (id.kind.to_string(), id.artifact_key.clone());
        if let Some(prev) = by_identity.get(&key) {
            if prev.modified_ms >= f.modified_ms {
                continue;
            }
        }
        by_identity.insert(
            key,
            UnusedArtifact {
                kind: id.kind.to_string(),
                node_id: f.node_id.clone(),
                label: label_for(&id.artifact_key),
                artifact_key: id.artifact_key,
                plugin_id: id.plugin_id,
                abs_path: f.abs_path.clone(),
                modified_ms: f.modified_ms,
            },
        );
    }

    let installed_count = by_identity.len();
    let grace_cutoff = now_ms - grace_days * DAY_MS;
    let mut rows = Vec::new();
    let mut suppressed_recent = 0usize;
    let mut suppressed_disabled = 0usize;

    for (key, row) in by_identity {
        // Rule 2 — ever fired?
        if ever_fired.contains(&key) {
            continue;
        }
        // Rule 4 — owning plugin enabled? Checked before the grace
        // window so a disabled plugin's fresh artifact is attributed to
        // the right reason.
        if let Some(pid) = row.plugin_id.as_deref() {
            let enabled = enabled_plugins.contains(pid)
                || enabled_plugins
                    .iter()
                    .any(|s| s.split('@').next() == Some(pid));
            if !enabled {
                suppressed_disabled += 1;
                continue;
            }
        }
        // Rule 3 — grace window.
        if row.modified_ms >= grace_cutoff {
            suppressed_recent += 1;
            continue;
        }
        rows.push(row);
    }

    rows.sort_by(|a, b| {
        a.modified_ms
            .cmp(&b.modified_ms)
            .then_with(|| a.artifact_key.cmp(&b.artifact_key))
    });

    UnusedReport {
        rows,
        installed_count,
        suppressed_recent,
        suppressed_disabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const OLD: i64 = NOW - 400 * DAY_MS;

    fn f(kind: &str, path: &str, modified_ms: i64) -> InstalledFile {
        InstalledFile {
            kind: kind.into(),
            node_id: format!("id-{path}"),
            abs_path: path.into(),
            modified_ms,
        }
    }

    fn fired(pairs: &[(&str, &str)]) -> HashSet<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn enabled(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    fn run(
        files: &[InstalledFile],
        f2: &HashSet<(String, String)>,
        en: &HashSet<String>,
    ) -> UnusedReport {
        compute_unused(files, f2, en, None, NOW, RECENTLY_MODIFIED_GRACE_DAYS)
    }

    #[test]
    fn lists_an_installed_artifact_with_no_record() {
        let files = [f("skill", "/h/.claude/skills/lonely/SKILL.md", OLD)];
        let r = run(&files, &fired(&[]), &enabled(&[]));
        assert_eq!(r.rows.len(), 1);
        assert_eq!(r.rows[0].artifact_key, "userSettings:lonely");
        assert_eq!(r.rows[0].label, "lonely");
        assert_eq!(r.installed_count, 1);
    }

    #[test]
    fn excludes_an_artifact_in_the_ledger() {
        let files = [f("skill", "/h/.claude/skills/busy/SKILL.md", OLD)];
        let r = run(
            &files,
            &fired(&[("skill", "userSettings:busy")]),
            &enabled(&[]),
        );
        assert!(r.rows.is_empty());
    }

    #[test]
    fn a_fired_command_does_not_mark_a_same_named_skill_used() {
        let files = [f("skill", "/h/.claude/skills/deploy/SKILL.md", OLD)];
        let r = run(
            &files,
            &fired(&[("command", "userSettings:deploy")]),
            &enabled(&[]),
        );
        assert_eq!(r.rows.len(), 1, "identity is (kind, key), not key alone");
    }

    #[test]
    fn collapses_the_same_artifact_across_cached_plugin_versions() {
        let base = "/h/.claude/plugins/cache/mk/demo";
        let files = [
            f("skill", &format!("{base}/1.0.0/skills/thing/SKILL.md"), OLD),
            f(
                "skill",
                &format!("{base}/1.1.0/skills/thing/SKILL.md"),
                OLD + 10,
            ),
            f(
                "skill",
                &format!("{base}/2.0.0/skills/thing/SKILL.md"),
                OLD + 20,
            ),
        ];
        let r = run(&files, &fired(&[]), &enabled(&["demo"]));
        assert_eq!(r.installed_count, 1);
        assert_eq!(r.rows.len(), 1);
        assert_eq!(
            r.rows[0].modified_ms,
            OLD + 20,
            "keeps the most recently modified instance"
        );
    }

    #[test]
    fn suppresses_recently_modified() {
        let files = [f("skill", "/h/.claude/skills/fresh/SKILL.md", NOW - DAY_MS)];
        let r = run(&files, &fired(&[]), &enabled(&[]));
        assert!(r.rows.is_empty());
        assert_eq!(r.suppressed_recent, 1);
    }

    #[test]
    fn excludes_artifacts_of_disabled_plugins() {
        // A disabled plugin cannot fire; listing it as unused is a
        // category error.
        let files = [f(
            "skill",
            "/h/.claude/plugins/cache/mk/offplug/1.0.0/skills/x/SKILL.md",
            OLD,
        )];
        let r = run(&files, &fired(&[]), &enabled(&["otherplug"]));
        assert!(r.rows.is_empty());
        assert_eq!(r.suppressed_disabled, 1);
    }

    #[test]
    fn enabled_plugin_matches_by_name_or_by_spec() {
        let files = [f(
            "skill",
            "/h/.claude/plugins/cache/mk/onplug/1.0.0/skills/x/SKILL.md",
            OLD,
        )];
        for set in [enabled(&["onplug"]), enabled(&["onplug@mk"])] {
            let r = run(&files, &fired(&[]), &set);
            assert_eq!(r.rows.len(), 1, "both name and spec forms must match");
        }
    }

    #[test]
    fn standalone_artifacts_are_never_excluded_by_the_enabled_rule() {
        let files = [f("skill", "/h/.claude/skills/mine/SKILL.md", OLD)];
        let r = run(&files, &fired(&[]), &enabled(&[]));
        assert_eq!(r.rows.len(), 1, "no plugin_id means the rule doesn't apply");
        assert_eq!(r.suppressed_disabled, 0);
    }

    #[test]
    fn hooks_never_appear() {
        let files = [f("hook", "/h/.claude/hooks/thing.json", OLD)];
        let r = run(&files, &fired(&[]), &enabled(&[]));
        assert!(r.rows.is_empty());
        assert_eq!(r.installed_count, 0);
    }

    #[test]
    fn rows_are_sorted_oldest_first() {
        let files = [
            f("skill", "/h/.claude/skills/b/SKILL.md", OLD + 100),
            f("skill", "/h/.claude/skills/a/SKILL.md", OLD),
        ];
        let r = run(&files, &fired(&[]), &enabled(&[]));
        assert_eq!(r.rows[0].label, "a");
        assert_eq!(r.rows[1].label, "b");
    }
}

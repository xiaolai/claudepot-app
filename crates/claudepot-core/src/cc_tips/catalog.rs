//! Catalog cache + render-time interpolation.
//!
//! Caches the extracted catalog at `~/.claudepot/cc_tips_catalog.json`
//! keyed by `(binary path, file size, mtime_ns)`. Rendering joins
//! the cache with the current `tipsHistory`, the snapshot log, and
//! Claudepot's bundled categories/triggers, then resolves
//! `${...}` interpolations against the user's keybindings (default
//! shortcuts when no override).

use crate::cc_tips::categories::{category_for, spinner_override_prose, Category};
use crate::cc_tips::error::{TipsError, TipsResult};
use crate::cc_tips::extract::{extract_from_binary, resolve_cc_binary, RawTip};
use crate::cc_tips::history::{read_tips_history, LastSeen, SnapshotLog};
use crate::cc_tips::triggers::{default_shortcut, known_id_count, trigger_for};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub bin_path: String,
    pub bin_size: u64,
    pub bin_mtime_ns: i128,
    /// CC version string parsed from the binary path (best effort).
    pub cc_version: String,
    pub extracted_at: i64,
    pub tips: Vec<RawTip>,
}

pub fn catalog_cache_path() -> TipsResult<PathBuf> {
    let home = dirs::home_dir().ok_or(TipsError::NoHome)?;
    Ok(home.join(".claudepot").join("cc_tips_catalog.json"))
}

fn binary_cache_key(bin: &Path) -> TipsResult<(u64, i128)> {
    let m = std::fs::metadata(bin).map_err(|source| TipsError::BinaryRead {
        path: bin.to_string_lossy().into_owned(),
        source,
    })?;
    let size = m.len();
    let mtime_ns = file_mtime_ns(&m);
    Ok((size, mtime_ns))
}

fn file_mtime_ns(m: &std::fs::Metadata) -> i128 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

fn cc_version_from_path(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Load (and refresh on stale) the catalog. If the cache key matches,
/// we trust the cache; otherwise we re-extract and rewrite it.
pub fn ensure_catalog(force_refresh: bool) -> TipsResult<CatalogSnapshot> {
    let bin = resolve_cc_binary()?;
    let (size, mtime) = binary_cache_key(&bin)?;
    let cache_path = catalog_cache_path()?;
    let bin_path_string = bin.to_string_lossy().into_owned();

    if !force_refresh {
        if let Ok(raw) = std::fs::read(&cache_path) {
            if let Ok(snap) = serde_json::from_slice::<CatalogSnapshot>(&raw) {
                if snap.bin_path == bin_path_string
                    && snap.bin_size == size
                    && snap.bin_mtime_ns == mtime
                {
                    return Ok(snap);
                }
            }
        }
    }

    let tips = extract_from_binary(&bin)?;
    let snap = CatalogSnapshot {
        bin_path: bin_path_string,
        bin_size: size,
        bin_mtime_ns: mtime,
        cc_version: cc_version_from_path(&bin),
        extracted_at: now_ms(),
        tips,
    };
    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body =
        serde_json::to_vec_pretty(&snap).map_err(|source| TipsError::CatalogParse { source })?;
    std::fs::write(&cache_path, body).map_err(|source| TipsError::CatalogIo {
        path: cache_path.to_string_lossy().into_owned(),
        source,
    })?;
    Ok(snap)
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedTip {
    pub id: String,
    pub category: Category,
    pub category_label: String,
    pub prose: String,
    pub prose_b: Option<String>,
    pub experiment_flag: Option<String>,
    pub condition_label: Option<String>,
    pub condition_label_b: Option<String>,
    pub cooldown_sessions: Option<u32>,
    pub last_seen: Option<LastSeen>,
    pub trigger_summary: String,
    pub relevance_source: Option<String>,
    pub provider_agnostic: bool,
    pub seen_status: SeenStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SeenStatus {
    Seen,
    NeverSeen,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipsRender {
    pub catalog_version: String,
    pub extracted_at: i64,
    pub partial: bool,
    pub extracted_count: usize,
    pub known_count: usize,
    pub current_num_startups: u32,
    pub tips: Vec<RenderedTip>,
    pub counts: TipsCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipsCounts {
    pub all: usize,
    pub seen: usize,
    pub never_seen: usize,
    pub active_experiments: usize,
}

/// Build the render-ready tips list. Joins catalog with
/// `tipsHistory` and snapshot log; substitutes `${...}` interpolations
/// against the user's keybindings (or defaults).
pub fn render_tips(force_refresh: bool) -> TipsResult<TipsRender> {
    let snap = ensure_catalog(force_refresh)?;
    let history =
        read_tips_history(None).unwrap_or_else(|_| crate::cc_tips::history::TipsHistoryRead {
            num_startups: 0,
            tips_history: BTreeMap::new(),
        });
    let snap_log = SnapshotLog::at(SnapshotLog::default_path()?);
    let now = now_ms();

    // Build set of extracted ids for lookup.
    let mut extracted: BTreeMap<String, RawTip> = BTreeMap::new();
    for t in &snap.tips {
        extracted.insert(t.id.clone(), t.clone());
    }

    // Set of all ids to render: extracted ∪ history-seen (so users
    // see legacy / spinner-override / unknown tips that they have
    // history for).
    let mut all_ids: BTreeSet<String> = extracted.keys().cloned().collect();
    for k in history.tips_history.keys() {
        all_ids.insert(k.clone());
    }

    let mut tips: Vec<RenderedTip> = Vec::with_capacity(all_ids.len());
    for id in all_ids {
        let raw = extracted.get(&id);
        let trigger = trigger_for(&id);
        let category = category_for(&id);
        // Skip Internal-only tips unless the user has history for them.
        if matches!(category, Category::Internal) && !history.tips_history.contains_key(&id) {
            continue;
        }
        let (
            prose,
            prose_b,
            experiment_flag,
            condition_label,
            condition_label_b,
            cooldown,
            relevance,
            provider_agnostic,
        ) = match raw {
            Some(r) => (
                interpolate_prose(&r.prose),
                r.prose_b.as_ref().map(|s| interpolate_prose(s)),
                r.experiment_flag
                    .clone()
                    .or(trigger.experiment.map(String::from)),
                r.condition_label.clone(),
                r.condition_label_b.clone(),
                r.cooldown_sessions,
                r.is_relevant_source.clone(),
                r.provider_agnostic,
            ),
            None => {
                // Fallback: maybe a spinner-override tip.
                let prose = spinner_override_prose(&id)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "(prose unavailable)".to_string());
                (prose, None, None, None, None, None, None, false)
            }
        };

        let seen_status = if history.tips_history.contains_key(&id) {
            SeenStatus::Seen
        } else {
            SeenStatus::NeverSeen
        };

        let last_seen = if let Some(count) = history.tips_history.get(&id) {
            snap_log.resolve_last_seen(&id, *count, now).unwrap_or(None)
        } else {
            None
        };

        tips.push(RenderedTip {
            id,
            category,
            category_label: category.label().to_string(),
            prose,
            prose_b,
            experiment_flag,
            condition_label,
            condition_label_b,
            cooldown_sessions: cooldown,
            last_seen,
            trigger_summary: trigger.summary.to_string(),
            relevance_source: relevance,
            provider_agnostic,
            seen_status,
        });
    }

    // Sort: seen-recently first (by snapshot resolved desc), then
    // never-seen, then alphabetical within each.
    tips.sort_by(|a, b| {
        use std::cmp::Ordering;
        let ra = a.last_seen.as_ref().map(|s| s.startup_count_when_seen);
        let rb = b.last_seen.as_ref().map(|s| s.startup_count_when_seen);
        match (ra, rb) {
            (Some(x), Some(y)) => y.cmp(&x).then_with(|| a.id.cmp(&b.id)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.id.cmp(&b.id),
        }
    });

    let extracted_count = snap.tips.len();
    let known_count = known_id_count();
    let partial = extracted_count * 100 / known_count.max(1) < 80;
    let counts = TipsCounts {
        all: tips.len(),
        seen: tips
            .iter()
            .filter(|t| matches!(t.seen_status, SeenStatus::Seen))
            .count(),
        never_seen: tips
            .iter()
            .filter(|t| matches!(t.seen_status, SeenStatus::NeverSeen))
            .count(),
        active_experiments: tips.iter().filter(|t| t.experiment_flag.is_some()).count(),
    };

    Ok(TipsRender {
        catalog_version: snap.cc_version,
        extracted_at: snap.extracted_at,
        partial,
        extracted_count,
        known_count,
        current_num_startups: history.num_startups,
        tips,
        counts,
    })
}

/// Append a snapshot if due, then return whether one was written.
pub fn record_view() -> TipsResult<bool> {
    let history = read_tips_history(None)?;
    let snap = crate::cc_tips::history::Snapshot {
        ts: now_ms(),
        num_startups: history.num_startups,
        tips_history: history.tips_history,
    };
    let log = SnapshotLog::at(SnapshotLog::default_path()?);
    log.record_if_due(now_ms(), snap)
}

/// Shortcut helpers `Mf("key","scope","default")`. Matches generic
/// single-letter helper IDs that Bun produces.
static SHORTCUT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"\$\{[A-Za-z_$][A-Za-z0-9_$]*\("([a-z]+:[A-Za-z]+)","[^"]*","([^"]*)"\)\}"#)
        .expect("static regex")
});

/// Color helpers. Recognized shapes:
///   `${IDENT("text")}`
///   `${IDENT(`text`)}`
///   `${IDENT('text')}`
///   `${IDENT(arg, arg)("text")}`        — curried color call
///   `${IDENT(arg, arg)(`text`)}`        — curried color call w/ template
///   `${IDENT(IDENT2(args))}`            — wrapped call (e.g. `${_(WJH(q))}`)
static COLOR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"\$\{[A-Za-z_$][A-Za-z0-9_$]*(?:\([^()]*\))?\((?:"([^"]*)"|`([^`]*)`|'([^']*)')\)\}"#,
    )
    .expect("static regex")
});

/// Replace `${Mf("chat:cycleMode","Chat","shift+tab")}` with the
/// user's actual binding (or default), and unwrap `${HELPER("text")}`
/// or `${HELPER('text')}` color helpers down to `text`.
fn interpolate_prose(s: &str) -> String {
    // Phase 1: shortcut helpers.
    let phase1 = SHORTCUT_RE.replace_all(s, |caps: &regex::Captures| {
        let key = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let dflt = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let resolved = default_shortcut(key);
        if resolved.is_empty() {
            humanize_default_key(dflt)
        } else {
            resolved.to_string()
        }
    });

    // Phase 2: color helpers. For each, return the inner display
    // string. The user-facing payload is always the single
    // quoted/templated argument that comes last; theme args, indices,
    // and helper wrappers are noise.
    let phase2 = COLOR_RE.replace_all(&phase1, |caps: &regex::Captures| {
        caps.get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str().to_string())
            .unwrap_or_default()
    });

    // Phase 3: any leftover `${VAR}` placeholders pass through (we
    // can't resolve variables we don't track).
    phase2.into_owned()
}

fn humanize_default_key(s: &str) -> String {
    s.split('+')
        .map(|seg| {
            let mut c = seg.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_interpolation_uses_known_default() {
        let s = r#"Press ${Mf("chat:cycleMode","Chat","shift+tab")} twice"#;
        let out = interpolate_prose(s);
        assert_eq!(out, "Press Shift+Tab twice");
    }

    #[test]
    fn shortcut_fallback_to_default_arg() {
        let s = r#"Press ${Mf("chat:unknownAction","Chat","ctrl+x")} now"#;
        let out = interpolate_prose(s);
        assert_eq!(out, "Press Ctrl+X now");
    }

    #[test]
    fn color_helper_unwrapped() {
        let s = r#"Visit ${Kq("/passes")} for rewards"#;
        let out = interpolate_prose(s);
        assert_eq!(out, "Visit /passes for rewards");
    }

    #[test]
    fn test_prose_interpolate_shortcut_and_color_combined() {
        let s = r#"Press ${Mf("chat:cycleMode","Chat","shift+tab")} then run ${Kq("suggestion",H.theme)("/effort high")}"#;
        let out = interpolate_prose(s);
        assert_eq!(out, "Press Shift+Tab then run /effort high");
    }

    #[test]
    fn humanize_default_key_capitalizes_segments() {
        assert_eq!(humanize_default_key("shift+tab"), "Shift+Tab");
        assert_eq!(humanize_default_key("ctrl+v"), "Ctrl+V");
    }
}

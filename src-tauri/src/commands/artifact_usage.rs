//! Tauri commands for artifact-usage telemetry.
//!
//! Read-only surface backed by `claudepot_core::artifact_usage` over
//! `sessions.db`. Every handler:
//!
//! - opens the session index,
//! - calls `refresh()` so the data is current (the session-index
//!   refresh is idempotent and cheap when nothing changed),
//! - then queries via the public `SessionIndex::usage_*` API.
//!
//! All handlers run in `spawn_blocking` because the refresh path
//! parses JSONL and SQLite is sync.
//!
//! The handlers do not contain business logic — they are thin
//! adapters over the core API per `architecture.md`.
//!
//! Rejections cross as `ErrorDto`. The `open session index: ` /
//! `refresh session index: ` / `query: ` prefixes are gone —
//! `SessionIndexError` names what failed and the UI names what it was
//! attempting. See `crate::dto_error`.

use crate::dto_artifact_usage::{
    parse_kind, ArtifactEverFiredDto, ArtifactUsageBatchEntryDto, ArtifactUsageRowDto,
    ArtifactUsageStatsDto, UnusedReportDto,
};
use crate::dto_error::{codes, ErrorDto};
use chrono::Utc;
use claudepot_core::paths;
use claudepot_core::session_index::SessionIndex;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// `ArtifactKind::parse` rejected a wire kind. `parse_kind` returns a
/// pre-composed English string ("unknown artifact kind: skil"), carried
/// verbatim — this layer has nothing to add to it.
fn unknown_kind(m: String) -> ErrorDto {
    ErrorDto::detail(codes::ARTIFACT_USAGE_UNKNOWN_KIND, m)
}

/// Open the index at `<data>/sessions.db` and run a refresh against
/// `<config>/projects/`. Centralized here so every usage command
/// applies the same freshness contract.
fn open_and_refresh() -> Result<SessionIndex, ErrorDto> {
    let data_dir = paths::claudepot_data_dir();
    let db_path = data_dir.join("sessions.db");
    let idx = SessionIndex::open(&db_path)?;
    let cfg = paths::claude_config_dir();
    idx.refresh(&cfg)?;
    Ok(idx)
}

/// One artifact's stats. Empty stats are returned (not an error) for
/// artifacts that have never fired — the UI uses `count_30d == 0`
/// to render the "never used" state.
#[tauri::command]
pub async fn artifact_usage_for(
    kind: String,
    artifact_key: String,
) -> Result<ArtifactUsageStatsDto, ErrorDto> {
    tokio::task::spawn_blocking(move || {
        let kind = parse_kind(&kind).map_err(unknown_kind)?;
        let idx = open_and_refresh()?;
        let now_ms = Utc::now().timestamp_millis();
        let stats = idx.usage_for_artifact(kind, &artifact_key, now_ms)?;
        Ok::<_, ErrorDto>(stats.into())
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Batch fetch — used by the Config-tree renderer to populate badges
/// for every visible artifact in one round-trip.
///
/// Returns one entry per resolvable `(kind, key)` in input order.
/// Invalid kinds are silently skipped (UI shouldn't have produced
/// them; this keeps a malformed renderer call from killing the whole
/// batch).
#[tauri::command]
pub async fn artifact_usage_batch(
    keys: Vec<(String, String)>,
) -> Result<Vec<ArtifactUsageBatchEntryDto>, ErrorDto> {
    tokio::task::spawn_blocking(move || {
        // Resolve kinds up-front so the core batch sees only valid pairs.
        let parsed: Vec<(claudepot_core::artifact_usage::ArtifactKind, String)> = keys
            .into_iter()
            .filter_map(|(k, v)| parse_kind(&k).ok().map(|kind| (kind, v)))
            .collect();
        if parsed.is_empty() {
            return Ok::<_, ErrorDto>(Vec::new());
        }
        let idx = open_and_refresh()?;
        let now_ms = Utc::now().timestamp_millis();
        let rows = idx.usage_batch(&parsed, now_ms)?;
        Ok::<_, ErrorDto>(
            rows.into_iter()
                .map(|((kind, key), stats)| ArtifactUsageBatchEntryDto {
                    kind: kind.as_str().to_string(),
                    artifact_key: key,
                    stats: stats.into(),
                })
                .collect(),
        )
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// How long a built server→plugin map stays fresh.
///
/// The map only changes when a plugin is installed, updated, or
/// removed. 60 s bounds how long a just-installed plugin's MCP server
/// shows unattributed, while collapsing the 5 s poll from ~12 walks a
/// minute to one.
const MCP_MAP_TTL: Duration = Duration::from_secs(60);

type McpMapCache = Mutex<Option<(Instant, Arc<HashMap<String, String>>)>>;

fn mcp_map_cache() -> &'static McpMapCache {
    static CACHE: OnceLock<McpMapCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Server→plugin map, rebuilt at most once per [`MCP_MAP_TTL`].
///
/// A poisoned lock falls back to building the map directly rather than
/// propagating — a stale-cache failure must not break the Usage view.
fn mcp_plugin_map() -> Arc<HashMap<String, String>> {
    let build = || Arc::new(claudepot_core::config_view::discover::mcp_server_to_plugin());
    let Ok(mut guard) = mcp_map_cache().lock() else {
        return build();
    };
    if let Some((built_at, map)) = guard.as_ref() {
        if built_at.elapsed() < MCP_MAP_TTL {
            return Arc::clone(map);
        }
    }
    let map = build();
    *guard = Some((Instant::now(), Arc::clone(&map)));
    map
}

/// True when at least one row is an MCP call still missing its plugin.
///
/// Lets the common case — no MCP usage at all — skip the filesystem
/// walk entirely.
fn needs_mcp_attribution(rows: &[claudepot_core::artifact_usage::UsageListRow]) -> bool {
    use claudepot_core::artifact_usage::ArtifactKind;
    rows.iter()
        .any(|r| r.kind == ArtifactKind::Mcp && r.plugin_id.is_none())
}

/// Top N artifacts by 30-day fire count. Optional kind filter.
#[tauri::command]
pub async fn artifact_usage_top(
    kind: Option<String>,
    limit: u32,
) -> Result<Vec<ArtifactUsageRowDto>, ErrorDto> {
    tokio::task::spawn_blocking(move || {
        let kind = match kind.as_deref() {
            Some(s) => Some(parse_kind(s).map_err(unknown_kind)?),
            None => None,
        };
        let idx = open_and_refresh()?;
        let now_ms = Utc::now().timestamp_millis();
        let mut rows = idx.usage_top(kind, limit as usize, now_ms)?;
        // MCP events carry no plugin_id (the extractor is pure JSONL and
        // can't read a plugin's .mcp.json). Attribute them here so a
        // plugin whose value is a bundled MCP server groups under that
        // plugin in the Usage view's filter instead of reading unused.
        //
        // Both guards below matter: UsageView polls this command every
        // 5 s, and the map costs one directory listing plus a
        // `.mcp.json` open attempt per cached plugin version (86 on the
        // author's machine). Unconditionally rebuilding it here put a
        // filesystem walk on a UI poll.
        if needs_mcp_attribution(&rows) {
            claudepot_core::artifact_usage::attribute_mcp_plugins(&mut rows, &mcp_plugin_map());
        }
        Ok::<_, ErrorDto>(rows.into_iter().map(ArtifactUsageRowDto::from).collect())
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Every artifact Claudepot has ever observed fire. Backs the Usage
/// tab's "Unused" view: the renderer subtracts this set from the
/// installed inventory it already derives from the Config tree.
///
/// One SQL query over `artifact_first_last`. Deliberately NOT built
/// from `artifact_usage_batch` — that path issues six statements per
/// key (24h + 7d + 30d + last_seen + avg + p50), which is fine for a
/// single badge but is thousands of queries across a ~900-artifact
/// inventory on UsageView's refresh cadence.
///
/// Also deliberately NOT built from `usage_daily`: those counters are
/// decremented when a transcript is pruned, so an artifact the user
/// runs weekly would read as never-fired after a session cleanup.
#[tauri::command]
pub async fn artifact_usage_ever_fired() -> Result<Vec<ArtifactEverFiredDto>, ErrorDto> {
    tokio::task::spawn_blocking(move || {
        let idx = open_and_refresh()?;
        let rows = idx
            .usage_ever_fired()?
            .into_iter()
            .map(
                |(kind, artifact_key, first_seen_ms, last_seen_ms)| ArtifactEverFiredDto {
                    kind: kind.as_str().to_string(),
                    artifact_key,
                    first_seen_ms,
                    last_seen_ms,
                },
            )
            .collect();
        Ok::<_, ErrorDto>(rows)
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Flatten a `ConfigTree` node list into the shape `unused` consumes.
///
/// Recursive: artifacts live inside `DirNode`s (e.g. a `skills/` group),
/// so a top-level-only pass would find almost nothing.
fn collect_files(
    nodes: &[claudepot_core::config_view::model::Node],
    out: &mut Vec<claudepot_core::artifact_usage::unused::InstalledFile>,
) {
    use claudepot_core::artifact_usage::unused::InstalledFile;
    use claudepot_core::config_view::model::Node;
    for n in nodes {
        match n {
            Node::File(f) => out.push(InstalledFile {
                kind: f.kind.as_str().to_string(),
                node_id: f.id.clone(),
                abs_path: f.abs_path.display().to_string(),
                // ns → ms. Modification time, not install time.
                modified_ms: f.mtime_unix_ns / 1_000_000,
            }),
            Node::Dir(d) => collect_files(&d.children, out),
        }
    }
}

/// The Unused view's rows, computed entirely in core.
///
/// Wires three inputs together — the installed inventory
/// (`config_view`), the durable ever-fired ledger, and the enabled-plugin
/// set — and hands them to `artifact_usage::unused::compute_unused`,
/// which owns every rule (identity, dedup, ledger subtraction, grace
/// window, disabled-plugin exclusion). This command contains no business
/// logic, per `.claude/rules/architecture.md`.
///
/// **`project_root` is deliberately `None`.** We scan global scopes only,
/// and `config_view::scan_global()` anchors the tree at the home
/// directory — so the tree's own `project_root` is `~`. Passing it would
/// key `~/.claude/skills/x` as `projectSettings:x` while the extractor
/// writes `userSettings:x`, and every user-scope artifact would be
/// reported unused.
#[tauri::command]
pub async fn artifact_usage_unused() -> Result<UnusedReportDto, ErrorDto> {
    tokio::task::spawn_blocking(move || {
        use claudepot_core::artifact_usage::unused;
        use std::collections::HashSet;

        let idx = open_and_refresh()?;
        let ever_fired: HashSet<(String, String)> = idx
            .usage_ever_fired()?
            .into_iter()
            .map(|(kind, key, _, _)| (kind.as_str().to_string(), key))
            .collect();

        let tree = claudepot_core::config_view::scan_global();
        let mut files: Vec<unused::InstalledFile> = Vec::new();
        for scope in &tree.scopes {
            collect_files(&scope.children, &mut files);
        }

        let enabled = claudepot_core::config_view::discover::load_enabled_plugin_specs(
            &claudepot_core::paths::claude_config_dir(),
        );

        let now_ms = Utc::now().timestamp_millis();
        let report = unused::compute_unused(
            &files,
            &ever_fired,
            &enabled,
            None,
            now_ms,
            unused::RECENTLY_MODIFIED_GRACE_DAYS,
        );
        Ok::<_, ErrorDto>(UnusedReportDto::from(report))
    })
    .await
    .map_err(ErrorDto::task_join)?
}

// Historical note: `artifact_usage_known_keys` was a stub for an
// "Unused" filter that never shipped in its slice; the command was
// removed to keep the IPC surface honest while
// `SessionIndex::usage_known_keys` was kept for the follow-up. That
// follow-up is `artifact_usage_ever_fired` above — but it reads the
// durable `artifact_first_last` ledger rather than `usage_daily`,
// because the latter cannot answer "ever" (see schema v6).

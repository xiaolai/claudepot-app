//! Bucket C — global content (`--include-global`).
//!
//! See `dev-docs/project-migrate-spec.md` §3.2 (`global/` layout) and
//! §4 Bucket C, plus `dev-docs/project-migrate-cc-research.md` §1
//! Bucket C entries.
//!
//! What travels:
//!   - `CLAUDE.md` (top-level)
//!   - `agents/`, `skills/`, `commands/`, `memory/` (top-level)
//!   - `plugins/installed_plugins.json` — registry only
//!   - `settings.json` — scrubbed (hooks split out)
//!   - `mcpServers.json` — scrubbed (absolute-path commands flagged)
//!   - `proposed-hooks.json` — written when hooks exist
//!
//! What does not travel:
//!   - `plugins/cache/<marketplace>/<plugin>/<version>/` — re-installs
//!   - `state.db`, caches, telemetry — Bucket D
//!   - `settings.local.json` — local by name
//!   - `managed-settings.json` — org policy
//!
//! Layout inside the bundle: `global/<rel>` mirrors the on-disk shape
//! under `~/.claude/`, with two exceptions:
//!   - `settings.json` becomes `settings.json.scrubbed`.
//!   - hooks travel under `proposed-hooks.json`.

use crate::migrate::bundle::BundleWriter;
use crate::migrate::error::MigrateError;
use crate::migrate::manifest::FileInventoryEntry;
use crate::migrate::trust;
use std::fs;
use std::path::{Path, PathBuf};

/// Top-level files in `~/.claude/` that Bucket C copies verbatim
/// (after the per-file scrubbing rules below).
pub const GLOBAL_TOP_LEVEL_FILES: &[&str] = &["CLAUDE.md"];

/// Directories under `~/.claude/` that travel verbatim under Bucket C.
pub const GLOBAL_DIRS: &[&str] = &["agents", "skills", "commands", "memory"];

/// Walk the source CC config dir's Bucket-C surfaces and append them
/// to the bundle. Returns the inventory entries written (so the
/// caller can fold them into the per-bundle integrity file).
///
/// Symlinks at every layer — including the top-level entries the
/// recursive walker normally follows once before its own symlink
/// guard kicks in — are rejected. `is_file`/`is_dir` follow symlinks,
/// so a `~/.claude/agents` symlink to `/etc` would export `/etc`'s
/// contents under `global/agents/...`. Use `symlink_metadata` for
/// every top-level probe to keep that path closed.
pub fn append_global(
    config_dir: &Path,
    writer: &mut BundleWriter,
) -> Result<Vec<FileInventoryEntry>, MigrateError> {
    let mut inv = Vec::new();
    let before = writer.inventory().len();

    // Top-level files — verbatim. Reject symlinks here, not just in
    // the recursive walker (which only sees children, not the named
    // entry the export targets).
    for name in GLOBAL_TOP_LEVEL_FILES {
        let src = config_dir.join(name);
        match fs::symlink_metadata(&src) {
            Ok(md) if md.file_type().is_symlink() => {
                return Err(MigrateError::IntegrityViolation(format!(
                    "global path is a symlink, refusing to export: {}",
                    src.display()
                )));
            }
            Ok(md) if md.is_file() => {
                writer.append_file(&format!("global/{name}"), &src, None)?;
            }
            _ => {}
        }
    }

    // Top-level directories — recursive verbatim.
    for d in GLOBAL_DIRS {
        let src = config_dir.join(d);
        match fs::symlink_metadata(&src) {
            Ok(md) if md.file_type().is_symlink() => {
                return Err(MigrateError::IntegrityViolation(format!(
                    "global directory is a symlink, refusing to export: {}",
                    src.display()
                )));
            }
            Ok(md) if md.is_dir() => {
                walk_append(&src, &src, &format!("global/{d}"), writer)?;
            }
            _ => {}
        }
    }

    // Plugins registry — file only, not the cache payload.
    let plugins_registry = config_dir.join("plugins/installed_plugins.json");
    match fs::symlink_metadata(&plugins_registry) {
        Ok(md) if md.file_type().is_symlink() => {
            return Err(MigrateError::IntegrityViolation(format!(
                "plugins registry is a symlink, refusing to export: {}",
                plugins_registry.display()
            )));
        }
        Ok(md) if md.is_file() => {
            writer.append_file(
                "global/plugins/installed_plugins.json",
                &plugins_registry,
                None,
            )?;
        }
        _ => {}
    }

    // settings.json — scrub + split.
    let settings_path = config_dir.join("settings.json");
    match fs::symlink_metadata(&settings_path) {
        Ok(md) if md.file_type().is_symlink() => {
            return Err(MigrateError::IntegrityViolation(format!(
                "settings.json is a symlink, refusing to export: {}",
                settings_path.display()
            )));
        }
        Ok(md) if md.is_file() => {
            let raw = fs::read(&settings_path).map_err(MigrateError::from)?;
            let parsed: serde_json::Value = serde_json::from_slice(&raw)
                .map_err(|e| MigrateError::Serialize(format!("settings.json parse: {e}")))?;
            let split = trust::scrub_user_settings(parsed);
            let scrubbed = serde_json::to_vec_pretty(&split.scrubbed)
                .map_err(|e| MigrateError::Serialize(e.to_string()))?;
            writer.append_bytes("global/settings.json.scrubbed", &scrubbed, 0o644)?;
            if let Some(hooks) = split.hooks {
                let hooks_bytes = serde_json::to_vec_pretty(&hooks)
                    .map_err(|e| MigrateError::Serialize(e.to_string()))?;
                writer.append_bytes("global/proposed-hooks.json", &hooks_bytes, 0o644)?;
            }
        }
        _ => {}
    }

    // mcpServers.json — scrub absolute-path commands.
    let mcp_path = config_dir.join("mcpServers.json");
    match fs::symlink_metadata(&mcp_path) {
        Ok(md) if md.file_type().is_symlink() => {
            return Err(MigrateError::IntegrityViolation(format!(
                "mcpServers.json is a symlink, refusing to export: {}",
                mcp_path.display()
            )));
        }
        Ok(md) if md.is_file() => {
            let raw = fs::read(&mcp_path).map_err(MigrateError::from)?;
            let mut parsed: serde_json::Value = serde_json::from_slice(&raw)
                .map_err(|e| MigrateError::Serialize(format!("mcpServers.json parse: {e}")))?;
            trust::scrub_mcp_block(&mut parsed);
            let scrubbed = serde_json::to_vec_pretty(&parsed)
                .map_err(|e| MigrateError::Serialize(e.to_string()))?;
            writer.append_bytes("global/mcp-servers.scrubbed.json", &scrubbed, 0o644)?;
        }
        _ => {}
    }

    // Pull all just-added entries into the local inventory snapshot
    // so the caller can roll them into the bundle's integrity record.
    for entry in &writer.inventory()[before..] {
        inv.push(entry.clone());
    }
    Ok(inv)
}

/// Recursively walk a Bucket-C directory and append each regular file
/// to the bundle under `bundle_prefix`.
fn walk_append(
    root: &Path,
    base: &Path,
    bundle_prefix: &str,
    writer: &mut BundleWriter,
) -> Result<(), MigrateError> {
    for entry in fs::read_dir(root).map_err(MigrateError::from)? {
        let entry = entry.map_err(MigrateError::from)?;
        let ft = entry.file_type().map_err(MigrateError::from)?;
        if ft.is_symlink() {
            return Err(MigrateError::IntegrityViolation(format!(
                "symlink in global content: {}",
                entry.path().display()
            )));
        }
        let path = entry.path();
        if ft.is_dir() {
            walk_append(&path, base, bundle_prefix, writer)?;
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(base)
                .map_err(|e| MigrateError::Io(std::io::Error::other(format!("strip_prefix: {e}"))))?
                .to_string_lossy()
                .replace('\\', "/");
            let bp = format!("{bundle_prefix}/{rel}");
            writer.append_file(&bp, &path, None)?;
        }
    }
    Ok(())
}

/// Apply extracted global content to the target config dir. Walks the
/// `global/` subtree under `staging` and copies each file into
/// `target_config_dir`, with the following rules:
///
/// - `CLAUDE.md` and `agents/`, `skills/`, `commands/`, `memory/`,
///   `plugins/installed_plugins.json`: copy verbatim. If the target
///   already has the file with the same content (sha256 equal),
///   skip; if different, write `<name>.imported` next to it (per
///   spec §6 "merge global content: never overwrite silently").
/// - `settings.json.scrubbed`: rename to `settings.json` at apply
///   time. If target has an existing `settings.json`, write
///   `.imported` next to it.
/// - `proposed-hooks.json`: only applied when `accept_hooks` is true.
///   Merged into the target's `settings.json:hooks` block.
/// - `mcp-servers.scrubbed.json`: written as
///   `mcpServers.imported.json`. The user re-points or re-enables
///   manually (or via `--accept-mcp` — deferred until the apply
///   pipeline supports it).
///
/// Returns a list of journal-step-friendly tuples
/// `(after_path, snapshot_path | None)` so the caller can record
/// rollback metadata.
pub fn apply_global(
    staging: &Path,
    target_config_dir: &Path,
    accept_hooks: bool,
    bundle_id: &str,
) -> Result<Vec<GlobalApplyStep>, MigrateError> {
    let global_root = staging.join("global");
    if !global_root.exists() {
        return Ok(Vec::new());
    }
    let mut steps = Vec::new();

    // Plain files: walk subtree and copy.
    let mut stack = vec![global_root.clone()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).map_err(MigrateError::from)? {
            let entry = entry.map_err(MigrateError::from)?;
            let ft = entry.file_type().map_err(MigrateError::from)?;
            let p = entry.path();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let rel = p
                .strip_prefix(&global_root)
                .map_err(|e| MigrateError::Io(std::io::Error::other(format!("strip: {e}"))))?
                .to_string_lossy()
                .replace('\\', "/");
            let step = apply_one(&rel, &p, target_config_dir, accept_hooks, bundle_id)?;
            if let Some(s) = step {
                steps.push(s);
            }
        }
    }
    Ok(steps)
}

#[derive(Debug, Clone)]
pub struct GlobalApplyStep {
    pub after: String,
    pub snapshot: Option<String>,
    pub kind: GlobalApplyKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalApplyKind {
    /// Wrote a fresh file to the target tree.
    Created,
    // (`Replaced` removed — the apply path always writes
    //  side-by-side instead of overwriting silently. Settings hooks
    //  are the one exception, and they go through `HooksAccepted`.)
    /// Wrote `<name>.imported` next to a differing target file.
    SideBySide,
    /// Skipped because the target file matches byte-for-byte.
    SkippedIdentical,
    /// Wrote `proposed-hooks.json` to settings (only when
    /// `accept_hooks=true`).
    HooksAccepted,
    /// Wrote `proposed-hooks.json` next to settings.json as
    /// `proposed-hooks.json` (when accept_hooks=false).
    HooksProposed,
    /// MCP scrubbed file written as `.imported`.
    McpProposed,
}

fn apply_one(
    rel: &str,
    src: &Path,
    target_config_dir: &Path,
    accept_hooks: bool,
    bundle_id: &str,
) -> Result<Option<GlobalApplyStep>, MigrateError> {
    let (target_rel, kind_hint) = match rel {
        "settings.json.scrubbed" => ("settings.json".to_string(), Hint::SettingsScrubbed),
        "proposed-hooks.json" => ("proposed-hooks.json".to_string(), Hint::Hooks),
        "mcp-servers.scrubbed.json" => ("mcpServers.imported.json".to_string(), Hint::Mcp),
        other => (other.to_string(), Hint::Plain),
    };
    let target_path = target_config_dir.join(&target_rel);
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(MigrateError::from)?;
    }

    match kind_hint {
        Hint::Plain | Hint::SettingsScrubbed => {
            apply_plain_or_settings(src, &target_path, &target_rel, bundle_id, kind_hint)
        }
        Hint::Hooks => {
            if accept_hooks {
                apply_hooks(src, target_config_dir, bundle_id)
            } else {
                // Place proposed-hooks.json next to settings.json so
                // the user can review. If a previous import already
                // wrote one, append the bundle_id suffix so we don't
                // clobber the prior review artifact (matches the
                // settings/CLAUDE.md side-by-side pattern in
                // `apply_plain_or_settings`).
                let proposed =
                    unique_proposed_path(target_config_dir, "proposed-hooks.json", bundle_id);
                fs::copy(src, &proposed).map_err(MigrateError::from)?;
                Ok(Some(GlobalApplyStep {
                    after: proposed.to_string_lossy().to_string(),
                    snapshot: None,
                    kind: GlobalApplyKind::HooksProposed,
                }))
            }
        }
        Hint::Mcp => {
            // Always proposed-only in v0; --accept-mcp is the next
            // layer. Same collision pattern as proposed-hooks: if a
            // prior import already wrote `mcpServers.imported.json`,
            // suffix this one so we don't overwrite the earlier
            // review artifact.
            let final_path =
                unique_proposed_path(target_config_dir, "mcpServers.imported.json", bundle_id);
            fs::copy(src, &final_path).map_err(MigrateError::from)?;
            Ok(Some(GlobalApplyStep {
                after: final_path.to_string_lossy().to_string(),
                snapshot: None,
                kind: GlobalApplyKind::McpProposed,
            }))
        }
    }
}

/// Pick a non-colliding path for a "proposed" review artifact
/// (`proposed-hooks.json`, `mcpServers.imported.json`). If the bare
/// name doesn't exist, return that. Otherwise insert the short
/// bundle_id before the file extension so repeated imports each get
/// their own review artifact.
fn unique_proposed_path(target_config_dir: &Path, base_name: &str, bundle_id: &str) -> PathBuf {
    let bare = target_config_dir.join(base_name);
    if !bare.exists() {
        return bare;
    }
    let suffix = bundle_id.split('-').next().unwrap_or(bundle_id);
    // Split base_name into stem + ext so we land at e.g.
    // `proposed-hooks.<id>.json`, not `proposed-hooks.json.<id>`.
    let (stem, ext) = match base_name.rsplit_once('.') {
        Some((s, e)) => (s, format!(".{e}")),
        None => (base_name, String::new()),
    };
    target_config_dir.join(format!("{stem}.{suffix}{ext}"))
}

#[derive(Clone, Copy)]
enum Hint {
    Plain,
    SettingsScrubbed,
    Hooks,
    Mcp,
}

fn apply_plain_or_settings(
    src: &Path,
    target: &Path,
    target_rel: &str,
    bundle_id: &str,
    hint: Hint,
) -> Result<Option<GlobalApplyStep>, MigrateError> {
    if !target.exists() {
        fs::copy(src, target).map_err(MigrateError::from)?;
        return Ok(Some(GlobalApplyStep {
            after: target.to_string_lossy().to_string(),
            snapshot: None,
            kind: GlobalApplyKind::Created,
        }));
    }
    // Compare contents. Identical → skip.
    let cur = fs::read(target).map_err(MigrateError::from)?;
    let new = fs::read(src).map_err(MigrateError::from)?;
    if cur == new {
        return Ok(Some(GlobalApplyStep {
            after: target.to_string_lossy().to_string(),
            snapshot: None,
            kind: GlobalApplyKind::SkippedIdentical,
        }));
    }
    // Differing — write side-by-side. Settings and CLAUDE.md must
    // never be silently overwritten. Append a unique suffix
    // (`<bundle_id_short>`) so repeated imports don't clobber prior
    // review artifacts (audit Robustness finding).
    let suffix = bundle_id.split('-').next().unwrap_or(bundle_id);
    let imported_name = match hint {
        Hint::SettingsScrubbed => format!("settings.imported.{suffix}.json"),
        _ => format!("{target_rel}.imported.{suffix}"),
    };
    let imported_path = target
        .parent()
        .map(|p| p.join(&imported_name))
        .ok_or_else(|| MigrateError::Io(std::io::Error::other("target has no parent")))?;
    fs::copy(src, &imported_path).map_err(MigrateError::from)?;
    Ok(Some(GlobalApplyStep {
        after: imported_path.to_string_lossy().to_string(),
        snapshot: None,
        kind: GlobalApplyKind::SideBySide,
    }))
}

fn apply_hooks(
    src: &Path,
    target_config_dir: &Path,
    bundle_id: &str,
) -> Result<Option<GlobalApplyStep>, MigrateError> {
    let hooks_bytes = fs::read(src).map_err(MigrateError::from)?;
    let hooks: serde_json::Value = serde_json::from_slice(&hooks_bytes)
        .map_err(|e| MigrateError::Serialize(format!("proposed-hooks.json: {e}")))?;

    let settings_path = target_config_dir.join("settings.json");
    let mut settings = if settings_path.exists() {
        let bytes = fs::read(&settings_path).map_err(MigrateError::from)?;
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| MigrateError::Serialize(format!("settings.json parse: {e}")))?
    } else {
        serde_json::Value::Object(Default::default())
    };

    let snapshot = if settings_path.exists() {
        crate::migrate::apply::snapshot_file(bundle_id, &settings_path)?
            .map(|p| p.to_string_lossy().to_string())
    } else {
        None
    };

    if let serde_json::Value::Object(map) = &mut settings {
        map.insert("hooks".to_string(), hooks);
    }

    let new_bytes =
        serde_json::to_vec_pretty(&settings).map_err(|e| MigrateError::Serialize(e.to_string()))?;
    fs::write(&settings_path, new_bytes).map_err(MigrateError::from)?;
    Ok(Some(GlobalApplyStep {
        after: settings_path.to_string_lossy().to_string(),
        snapshot,
        kind: GlobalApplyKind::HooksAccepted,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::bundle::{BundleReader, BundleWriter};
    use crate::migrate::manifest::{BundleManifest, ExportFlags, SCHEMA_VERSION};

    fn fixture_manifest() -> BundleManifest {
        BundleManifest {
            schema_version: SCHEMA_VERSION,
            claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
            cc_version: None,
            created_at: "2026-04-27T00:00:00Z".to_string(),
            source_os: "macos".to_string(),
            source_arch: "aarch64".to_string(),
            host_identity: "ab".repeat(32),
            source_home: "/Users/joker".to_string(),
            source_claude_config_dir: "/Users/joker/.claude".to_string(),
            projects: vec![],
            flags: ExportFlags {
                include_global: true,
                ..Default::default()
            },
            file_inventory: vec![],
        }
    }

    fn seed_global(cfg: &Path) {
        fs::create_dir_all(cfg.join("agents")).unwrap();
        fs::create_dir_all(cfg.join("skills")).unwrap();
        fs::create_dir_all(cfg.join("plugins")).unwrap();
        fs::write(cfg.join("CLAUDE.md"), "# user prefs\n").unwrap();
        fs::write(cfg.join("agents/foo.md"), "# foo agent\n").unwrap();
        fs::write(cfg.join("skills/bar.md"), "# bar skill\n").unwrap();
        fs::write(
            cfg.join("plugins/installed_plugins.json"),
            r#"{"version":1,"plugins":[]}"#,
        )
        .unwrap();
        fs::write(
            cfg.join("settings.json"),
            r#"{"theme":"dark","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[]}]}}"#,
        )
        .unwrap();
        fs::write(
            cfg.join("mcpServers.json"),
            r#"{"x":{"command":"/opt/bin/x"},"y":{"command":"node","args":["a.js"]}}"#,
        )
        .unwrap();
    }

    #[test]
    fn append_global_writes_expected_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join(".claude");
        seed_global(&cfg);
        let bundle_path = tmp.path().join("g.tar.zst");
        let mut w = BundleWriter::create(&bundle_path).unwrap();
        let inv = append_global(&cfg, &mut w).unwrap();
        w.finalize(&fixture_manifest()).unwrap();

        let r = BundleReader::open(&bundle_path).unwrap();
        // Verbatim files.
        assert_eq!(r.read_entry("global/CLAUDE.md").unwrap(), b"# user prefs\n");
        assert_eq!(
            r.read_entry("global/agents/foo.md").unwrap(),
            b"# foo agent\n"
        );
        assert_eq!(
            r.read_entry("global/skills/bar.md").unwrap(),
            b"# bar skill\n"
        );
        // Plugin registry, but NOT cache.
        assert!(r
            .read_entry("global/plugins/installed_plugins.json")
            .is_ok());
        // Scrubbed settings has no hooks.
        let settings = r.read_entry("global/settings.json.scrubbed").unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&settings).unwrap();
        assert!(parsed.get("hooks").is_none());
        assert_eq!(parsed["theme"], "dark");
        // proposed-hooks.json carries the hooks.
        let hooks = r.read_entry("global/proposed-hooks.json").unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&hooks)
            .unwrap()
            .get("PreToolUse")
            .is_some());
        // mcp-servers.scrubbed marks /opt path needs_resolution.
        let mcp = r.read_entry("global/mcp-servers.scrubbed.json").unwrap();
        let mcp_v: serde_json::Value = serde_json::from_slice(&mcp).unwrap();
        assert_eq!(mcp_v["x"]["_claudepot"]["needs_resolution"], true);
        assert!(mcp_v["y"].get("_claudepot").is_none());

        assert!(!inv.is_empty());
    }

    #[test]
    fn apply_global_creates_files_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        let global = staging.join("global");
        fs::create_dir_all(&global).unwrap();
        fs::write(global.join("CLAUDE.md"), "from-bundle").unwrap();
        fs::write(
            global.join("settings.json.scrubbed"),
            r#"{"theme":"light"}"#,
        )
        .unwrap();

        let target_cfg = tmp.path().join("target/.claude");
        fs::create_dir_all(&target_cfg).unwrap();

        let steps = apply_global(&staging, &target_cfg, false, "bid").unwrap();
        assert!(steps.iter().any(|s| s.kind == GlobalApplyKind::Created));
        assert_eq!(
            fs::read_to_string(target_cfg.join("CLAUDE.md")).unwrap(),
            "from-bundle"
        );
        assert!(target_cfg.join("settings.json").exists());
    }

    #[test]
    fn apply_global_writes_side_by_side_for_differing_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        let global = staging.join("global");
        fs::create_dir_all(&global).unwrap();
        fs::write(
            global.join("settings.json.scrubbed"),
            r#"{"theme":"light"}"#,
        )
        .unwrap();

        let target_cfg = tmp.path().join("target/.claude");
        fs::create_dir_all(&target_cfg).unwrap();
        fs::write(target_cfg.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();

        let steps = apply_global(&staging, &target_cfg, false, "bid").unwrap();
        let side = steps
            .iter()
            .find(|s| s.kind == GlobalApplyKind::SideBySide)
            .unwrap();
        // Bundle id is `bid` per the test fixture; suffix uses the
        // first hyphen-segment, so `settings.imported.bid.json`.
        assert!(
            side.after.ends_with("settings.imported.bid.json"),
            "expected per-bundle suffix; got: {}",
            side.after
        );
        // Original target untouched.
        assert_eq!(
            fs::read_to_string(target_cfg.join("settings.json")).unwrap(),
            r#"{"theme":"dark"}"#
        );
    }

    #[test]
    fn apply_global_hooks_proposed_when_not_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        let global = staging.join("global");
        fs::create_dir_all(&global).unwrap();
        fs::write(global.join("proposed-hooks.json"), r#"{"PreToolUse":[]}"#).unwrap();
        let target_cfg = tmp.path().join("target/.claude");
        fs::create_dir_all(&target_cfg).unwrap();
        let steps = apply_global(&staging, &target_cfg, false, "bid").unwrap();
        let h = steps
            .iter()
            .find(|s| s.kind == GlobalApplyKind::HooksProposed)
            .unwrap();
        assert!(h.after.ends_with("proposed-hooks.json"));
        // settings.json was NOT created with hooks merged in.
        assert!(!target_cfg.join("settings.json").exists());
    }

    #[test]
    fn apply_global_hooks_accepted_merges_into_settings() {
        use crate::testing::lock_data_dir;
        let _lock = lock_data_dir();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", tmp.path().join("d"));
        let staging = tmp.path().join("staging");
        let global = staging.join("global");
        fs::create_dir_all(&global).unwrap();
        fs::write(
            global.join("proposed-hooks.json"),
            r#"{"PreToolUse":[{"matcher":"Bash"}]}"#,
        )
        .unwrap();
        let target_cfg = tmp.path().join("target/.claude");
        fs::create_dir_all(&target_cfg).unwrap();
        fs::write(target_cfg.join("settings.json"), r#"{"theme":"light"}"#).unwrap();

        let steps = apply_global(&staging, &target_cfg, true, "bid").unwrap();
        let h = steps
            .iter()
            .find(|s| s.kind == GlobalApplyKind::HooksAccepted)
            .unwrap();
        assert!(h.after.ends_with("settings.json"));
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(target_cfg.join("settings.json")).unwrap()).unwrap();
        assert!(v.get("hooks").is_some());
        std::env::remove_var("CLAUDEPOT_DATA_DIR");
    }

    #[test]
    fn apply_global_skips_identical_files() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        let global = staging.join("global");
        fs::create_dir_all(&global).unwrap();
        fs::write(global.join("CLAUDE.md"), "same").unwrap();
        let target_cfg = tmp.path().join("target/.claude");
        fs::create_dir_all(&target_cfg).unwrap();
        fs::write(target_cfg.join("CLAUDE.md"), "same").unwrap();
        let steps = apply_global(&staging, &target_cfg, false, "bid").unwrap();
        assert!(steps
            .iter()
            .any(|s| s.kind == GlobalApplyKind::SkippedIdentical));
    }
}

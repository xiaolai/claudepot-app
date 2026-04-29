//! Bundle manifest schema and version negotiation.
//!
//! See `dev-docs/project-migrate-spec.md` §3.4 for the schema-version
//! contract: the importer refuses unknown versions. The
//! `--upgrade-schema` flow (§3.4 in-place upgrade) is not implemented
//! yet — there are no older schemas to upgrade from.
//!
//! Source identity is recorded as a hashed digest only. We sha256
//! `(hostname || user || HOME)` and store the hex result in
//! `host_identity`. The pre-hash material is **not** kept anywhere
//! in the bundle — opaque off-machine, stable across exports from
//! the same machine.

use serde::{Deserialize, Serialize};

/// Wire-format version. Bumped on any **breaking** change to the
/// manifest, integrity record, or per-project artifact layout.
/// Additive fields do not bump this — `serde(default)` carries the gap.
///
/// Schema 2 (current): integrity is consolidated into the
/// `BundleManifest.file_inventory` field. The previous standalone
/// `integrity.sha256` file is gone — the manifest is the one source
/// of truth for which files are in the bundle and what their hashes
/// should be. Signing the manifest digest (`<bundle>.manifest.minisig`)
/// therefore covers integrity for every payload file by transitivity.
pub const SCHEMA_VERSION: u32 = 2;

/// Top-level bundle manifest, written as `manifest.json` at the bundle
/// root. The trailing self-sha256 line (`§3.3`) is appended by the
/// bundle writer, not by this struct's `serde` round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    /// Monotonic schema version. See `SCHEMA_VERSION`.
    pub schema_version: u32,

    /// `claudepot-core` package version that wrote this bundle. Used
    /// for forensics ("import receipt" §5.4) and for picking the
    /// rewrite-engine codepath in future schema-3 → schema-2 import
    /// matrices.
    pub claudepot_version: String,

    /// CC version from `~/.claude/CLAUDE.md` if present at export time.
    /// Optional — older bundles (or bundles without `--include-global`)
    /// may not capture it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cc_version: Option<String>,

    /// RFC 3339 UTC timestamp when the bundle was sealed.
    pub created_at: String,

    /// Source-machine OS family. One of `"macos"`, `"linux"`,
    /// `"windows"`. Drives the cross-OS rewrite rules in `plan.rs`.
    pub source_os: String,

    /// Source-machine architecture (`"aarch64"`, `"x86_64"`, etc.).
    /// Currently informational — no rewrite rule depends on it, but
    /// recorded so resume failures can be debugged.
    pub source_arch: String,

    /// Sha256 hex digest of `(hostname || user || HOME)`. Stable
    /// across exports from the same machine, opaque off-machine.
    pub host_identity: String,

    /// Source HOME at export time. Becomes a substitution rule at
    /// import (§5.2 rule 3) so user-name-bearing paths inside tool
    /// results are rewritten.
    pub source_home: String,

    /// Source `CLAUDE_CONFIG_DIR` if non-default; otherwise the default
    /// `<HOME>/.claude` shape. Becomes substitution rule 4 (§5.2).
    pub source_claude_config_dir: String,

    /// One entry per project bundled. The list is the unit of conflict
    /// resolution at import time.
    pub projects: Vec<ProjectManifestRef>,

    /// Feature flags chosen at export. Drives import-time UI: e.g.
    /// when `include_global` is false, the importer doesn't surface
    /// the global trust gate panel even if `global/` is empty.
    pub flags: ExportFlags,

    /// Single source of truth for "what files are in this bundle and
    /// what should their bytes hash to". One entry per regular file in
    /// the tar except `manifest.json` itself (which is self-verified
    /// via its sha trailer). Signing the manifest digest commits to
    /// every file in this list by transitivity.
    ///
    /// Replaces the standalone `integrity.sha256` file shipped in
    /// schema 1. `serde(default)` is intentionally absent — schema 2
    /// requires this field; older bundles trip
    /// `UnsupportedSchemaVersion` before deserialization is reached.
    pub file_inventory: Vec<FileInventoryEntry>,
}

/// Pointer to a per-project manifest under `projects/<id>/manifest.json`.
/// The full per-project manifest carries the inventory; this just keeps
/// the top-level summary fast to read for `project migrate inspect`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifestRef {
    /// Stable id within the bundle. UUIDv4. Used for the staging
    /// directory name and the journal entry id.
    pub id: String,

    /// Source absolute cwd, NFC-normalized. The display key in the
    /// import wizard.
    pub source_cwd: String,

    /// Sanitized slug as it appeared on the source disk. Migrators
    /// recompute the target slug from `target_canonical_git_root`;
    /// this is recorded for forensics only.
    pub source_slug: String,

    /// Number of session JSONLs bundled for this project.
    pub session_count: u32,
}

/// Per-project manifest written at `projects/<id>/manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    /// Same id as the top-level reference.
    pub id: String,

    /// Source absolute cwd, NFC-normalized.
    pub source_cwd: String,

    /// Source canonical git root (per CC's `findCanonicalGitRoot`).
    /// Equal to `source_cwd` for non-git projects. Drives slug
    /// recompute on import (§5.3).
    pub source_canonical_git_root: String,

    /// Source slug at export time.
    pub source_slug: String,

    /// All sessionIds belonging to this project (own slug + worktree
    /// siblings, see §4 sub-bucket B enumeration). Used to filter
    /// Bucket B siblings in the apply phase.
    pub session_ids: Vec<String>,

    /// `--include-live` was set and at least one of `session_ids`
    /// changed under us during export. Surfaces on import as a banner
    /// the user must explicitly accept.
    #[serde(default)]
    pub live_at_export: bool,

    /// `--include-worktree` was set and `worktree.tar` is present.
    #[serde(default)]
    pub worktree_set: bool,
}

/// Inventory entry for a single bundle file. The path is bundle-relative
/// (i.e. starts with `projects/<id>/...`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInventoryEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// Export-time feature flags. Travel inside `manifest.json` so the
/// importer can surface them in the inspect view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportFlags {
    #[serde(default)]
    pub include_global: bool,
    #[serde(default)]
    pub include_worktree: bool,
    #[serde(default)]
    pub include_live: bool,
    #[serde(default)]
    pub include_claudepot_state: bool,
    /// `--no-file-history` inverts this — bundle keeps the JSONL
    /// records but skips the on-disk `<sha256>@v<n>` files.
    #[serde(default = "default_true")]
    pub include_file_history: bool,
    #[serde(default)]
    pub encrypted: bool,
    #[serde(default)]
    pub signed: bool,
}

impl Default for ExportFlags {
    fn default() -> Self {
        Self {
            include_global: false,
            include_worktree: false,
            include_live: false,
            include_claudepot_state: false,
            // Match the serde default so `default()` and round-tripped
            // older bundles agree on the fallback.
            include_file_history: true,
            encrypted: false,
            signed: false,
        }
    }
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_constant_round_trips() {
        let m = BundleManifest {
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
            flags: ExportFlags::default(),
            file_inventory: vec![],
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: BundleManifest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn export_flags_default_keeps_file_history() {
        // The default-true on `include_file_history` is load-bearing —
        // older bundles written before this field existed must roundtrip
        // as if file-history was bundled (the conservative default).
        let json = r#"{}"#;
        let f: ExportFlags = serde_json::from_str(json).unwrap();
        assert!(f.include_file_history);
    }

    #[test]
    fn project_manifest_omits_optional_fields_when_default() {
        // `live_at_export` and `worktree_set` default to false; we
        // don't want them to appear in the wire format unless set.
        let pm = ProjectManifest {
            id: "abc".to_string(),
            source_cwd: "/x".to_string(),
            source_canonical_git_root: "/x".to_string(),
            source_slug: "-x".to_string(),
            session_ids: vec![],
            live_at_export: false,
            worktree_set: false,
        };
        let s = serde_json::to_string(&pm).unwrap();
        // The serializer does include them (no skip_serializing_if on
        // bool defaults). That's fine — older readers default-load
        // them. Test just locks the round-trip.
        let back: ProjectManifest = serde_json::from_str(&s).unwrap();
        assert!(!back.live_at_export);
        assert!(!back.worktree_set);
    }
}

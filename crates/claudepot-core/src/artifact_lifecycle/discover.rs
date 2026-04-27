//! `list_disabled` — walk every active scope_root's `.disabled/`
//! tree and emit one record per artifact found there.
//!
//! Bypasses the active-discovery deny-list intentionally: this is the
//! one caller that wants to see inside `.disabled/`.

use crate::artifact_lifecycle::disable::DisabledRecord;
use crate::artifact_lifecycle::error::{LifecycleError, Result};
use crate::artifact_lifecycle::paths::{
    enabled_target_for, ActiveRoots, ArtifactKind, PayloadKind, Scope, Trackable, DISABLED_DIR,
};
use std::path::{Path, PathBuf};

/// Walk each scope_root's `.disabled/{skills,agents,commands}/` tree
/// and return one `DisabledRecord` per artifact. Order: by scope
/// (User first), then kind, then name.
pub fn list_disabled(roots: &ActiveRoots) -> Result<Vec<DisabledRecord>> {
    let mut out = Vec::new();
    for (scope, scope_root) in roots.iter_scoped() {
        let disabled_root = scope_root.join(DISABLED_DIR);
        if !disabled_root.exists() {
            continue;
        }
        for kind in [ArtifactKind::Skill, ArtifactKind::Agent, ArtifactKind::Command] {
            let kind_root = disabled_root.join(kind.subdir());
            if !kind_root.exists() {
                continue;
            }
            for record in walk_kind(scope, scope_root, kind, &kind_root)? {
                out.push(record);
            }
        }
    }
    out.sort_by(|a, b| {
        // User scope first, then kind, then name.
        a.scope
            .cmp_key()
            .cmp(&b.scope.cmp_key())
            .then_with(|| a.kind.subdir().cmp(b.kind.subdir()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

fn walk_kind(
    scope: Scope,
    scope_root: &Path,
    kind: ArtifactKind,
    kind_root: &Path,
) -> Result<Vec<DisabledRecord>> {
    let mut out = Vec::new();
    match kind {
        ArtifactKind::Skill => {
            // One level: each entry is a directory containing SKILL.md
            // (or the bare-file form `<name>.md`).
            for entry in
                std::fs::read_dir(kind_root).map_err(LifecycleError::io("read disabled skills"))?
            {
                let entry = entry.map_err(LifecycleError::io("read disabled skill entry"))?;
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let payload_kind = if path.is_dir() {
                    PayloadKind::Directory
                } else {
                    PayloadKind::File
                };
                out.push(record_for(scope, scope_root, kind, name, path, payload_kind));
            }
        }
        ArtifactKind::Agent | ArtifactKind::Command => {
            // Recursive walk for `.md` files; preserve nested rel-path.
            walk_md_files(kind_root, &mut |abs| {
                if let Ok(rel) = abs.strip_prefix(kind_root) {
                    let name = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    out.push(record_for(
                        scope,
                        scope_root,
                        kind,
                        name,
                        abs.to_path_buf(),
                        PayloadKind::File,
                    ));
                }
            })?;
        }
    }
    Ok(out)
}

fn walk_md_files(root: &Path, f: &mut dyn FnMut(&Path)) -> Result<()> {
    for entry in std::fs::read_dir(root).map_err(LifecycleError::io("read disabled walk"))? {
        let entry = entry.map_err(LifecycleError::io("read disabled entry"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if name_s.starts_with('.') {
            continue;
        }
        let ft = entry
            .file_type()
            .map_err(LifecycleError::io("file type"))?;
        if ft.is_dir() {
            walk_md_files(&path, f)?;
        } else if ft.is_file() && name_s.ends_with(".md") {
            f(&path);
        }
    }
    Ok(())
}

fn record_for(
    scope: Scope,
    scope_root: &Path,
    kind: ArtifactKind,
    name: String,
    current_path: PathBuf,
    payload_kind: PayloadKind,
) -> DisabledRecord {
    let trackable = Trackable {
        scope,
        scope_root: scope_root.to_path_buf(),
        kind,
        relative_path: name.clone(),
        payload_kind,
        already_disabled: true,
    };
    DisabledRecord {
        scope,
        scope_root: scope_root.to_path_buf(),
        kind,
        name,
        original_path: enabled_target_for(&trackable),
        current_path,
        payload_kind,
    }
}

impl Scope {
    fn cmp_key(self) -> u8 {
        match self {
            Self::User => 0,
            Self::Project => 1,
        }
    }
}

//! Read/write the `pending-changes.json` side-car next to a
//! template-driven run's report.

use std::path::{Path, PathBuf};

use crate::fs_utils;

use super::ops::PendingChanges;

/// Resolve the `pending-changes.json` path for a run, given the
/// blueprint's `pending_changes_path` template (e.g.
/// `"{output_dir}/.pending-changes.json"`) and the actual
/// resolved output path of this run.
pub fn pending_path_for(
    pending_changes_path_template: &str,
    output_path: &Path,
) -> PathBuf {
    let dir = output_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let resolved = pending_changes_path_template.replace("{output_dir}", &dir.display().to_string());
    PathBuf::from(resolved)
}

pub fn read(path: &Path) -> Result<PendingChanges, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

pub fn write(path: &Path, value: &PendingChanges) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| format!("serialize: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    fs_utils::atomic_write(path, &bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::apply::ops::{Operation, PendingGroup, PendingItem};

    #[test]
    fn pending_path_uses_template_and_output_dir() {
        let p = pending_path_for(
            "{output_dir}/.pending-changes.json",
            Path::new("/tmp/foo/report.md"),
        );
        assert_eq!(p, PathBuf::from("/tmp/foo/.pending-changes.json"));
    }

    #[test]
    fn pending_changes_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".pending-changes.json");
        let value = PendingChanges {
            schema_version: 1,
            automation_id: "a".into(),
            run_id: "r".into(),
            generated_at: "2026-05-02T08:00:00Z".into(),
            summary: "1 change".into(),
            groups: vec![PendingGroup {
                id: "g".into(),
                title: "moves".into(),
                items: vec![PendingItem {
                    id: "i".into(),
                    description: "move x → y".into(),
                    operation: Operation::Move {
                        from: "/tmp/x".into(),
                        to: "/tmp/y".into(),
                    },
                }],
            }],
        };
        write(&p, &value).unwrap();
        let back = read(&p).unwrap();
        assert_eq!(back, value);
    }

    #[test]
    fn read_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.json");
        let err = read(&p).unwrap_err();
        assert!(err.contains("read"));
    }
}

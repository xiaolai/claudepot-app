//! Read/write the `pending-changes.json` side-car next to a
//! template-driven run's report.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::fs_utils;

use super::ops::PendingChanges;

/// Side-car read/write failures. Folded into
/// [`crate::templates::TemplateError`] via `#[from]` so callers at the
/// templates boundary can propagate with `?`; kept as its own enum so
/// the io-vs-parse distinction survives (per rust-conventions, no
/// stringly `Result<_, String>` at public core boundaries).
#[derive(Debug, Error)]
pub enum SidecarError {
    #[error("read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("serialize: {0}")]
    Serialize(#[source] serde_json::Error),

    #[error("write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Resolve the `pending-changes.json` path for a run, given the
/// blueprint's `pending_changes_path` template (e.g.
/// `"{output_dir}/.pending-changes.json"`) and the actual
/// resolved output path of this run.
pub fn pending_path_for(pending_changes_path_template: &str, output_path: &Path) -> PathBuf {
    let dir = output_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let resolved =
        pending_changes_path_template.replace("{output_dir}", &dir.display().to_string());
    PathBuf::from(resolved)
}

pub fn read(path: &Path) -> Result<PendingChanges, SidecarError> {
    let bytes = std::fs::read(path).map_err(|e| SidecarError::Read {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_json::from_slice(&bytes).map_err(|e| SidecarError::Parse {
        path: path.to_path_buf(),
        source: e,
    })
}

pub fn write(path: &Path, value: &PendingChanges) -> Result<(), SidecarError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(SidecarError::Serialize)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| SidecarError::Write {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    fs_utils::atomic_write(path, &bytes).map_err(|e| SidecarError::Write {
        path: path.to_path_buf(),
        source: e,
    })?;
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
            agent_id: "a".into(),
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
        assert!(matches!(err, SidecarError::Read { .. }));
        assert!(err.to_string().contains("read"));
    }

    #[test]
    fn read_malformed_json_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.json");
        std::fs::write(&p, b"{not json").unwrap();
        let err = read(&p).unwrap_err();
        assert!(matches!(err, SidecarError::Parse { .. }));
    }
}

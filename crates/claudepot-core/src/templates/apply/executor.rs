//! Async executor for selected pending operations.
//!
//! Operates only on typed `Operation` values. No shell ever
//! reaches the filesystem. Each operation is re-validated
//! against the blueprint's apply config immediately before
//! execution — even if the validator was bypassed in an earlier
//! step, the executor will refuse.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::templates::blueprint::ApplyConfig;

use super::ops::{Operation, PendingChanges, PendingItem};
use super::validator::validate_item;

/// Per-item outcome of an apply attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemOutcome {
    Applied,
    Rejected { reason: String },
    Failed { error: String },
}

/// Aggregate result of an apply call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub item_id: String,
    pub outcome: ItemOutcome,
}

/// Persisted record of a completed apply step. Written next to
/// the run's report; the Reports panel shows it as a separate
/// row classified by `ArtifactKind::ApplyReceipt`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyReceipt {
    pub schema_version: u32,
    pub automation_id: String,
    pub run_id: String,
    pub applied_at: String,
    pub outcomes: Vec<ApplyOutcome>,
}

/// Apply selected items from a `PendingChanges` document.
///
/// `selected_ids` is the user's checked-rows set. Items not in
/// the set are quietly omitted from the receipt. Items in the
/// set that fail validation are logged as `Rejected` (not
/// `Failed`) and never executed.
pub async fn apply_selected(
    pending: &PendingChanges,
    apply: &ApplyConfig,
    selected_ids: &[String],
) -> ApplyReceipt {
    let mut outcomes = Vec::new();
    for group in &pending.groups {
        for item in &group.items {
            if !selected_ids.iter().any(|s| s == &item.id) {
                continue;
            }
            let outcome = apply_one(item, apply).await;
            outcomes.push(ApplyOutcome {
                item_id: item.id.clone(),
                outcome,
            });
        }
    }
    ApplyReceipt {
        schema_version: 1,
        automation_id: pending.automation_id.clone(),
        run_id: pending.run_id.clone(),
        applied_at: chrono::Utc::now().to_rfc3339(),
        outcomes,
    }
}

async fn apply_one(item: &PendingItem, apply: &ApplyConfig) -> ItemOutcome {
    if let Err(e) = validate_item(&item.operation, apply) {
        return ItemOutcome::Rejected {
            reason: e.to_string(),
        };
    }
    match execute(&item.operation).await {
        Ok(()) => ItemOutcome::Applied,
        Err(e) => ItemOutcome::Failed { error: e },
    }
}

async fn execute(op: &Operation) -> Result<(), String> {
    match op {
        Operation::Move { from, to } => {
            let from = expand_user(from);
            let to = expand_user(to);
            if let Some(parent) = to.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            tokio::fs::rename(&from, &to)
                .await
                .map_err(|e| format!("move {} → {}: {e}", from.display(), to.display()))
        }
        Operation::Rename { path, new_name } => {
            let path = expand_user(path);
            let parent = path
                .parent()
                .ok_or_else(|| format!("no parent for {}", path.display()))?
                .to_path_buf();
            let new_path = parent.join(new_name);
            tokio::fs::rename(&path, &new_path)
                .await
                .map_err(|e| format!("rename {} → {}: {e}", path.display(), new_path.display()))
        }
        Operation::Mkdir { path } => {
            let path = expand_user(path);
            tokio::fs::create_dir_all(&path)
                .await
                .map_err(|e| format!("mkdir {}: {e}", path.display()))
        }
        Operation::Write { path, content_b64 } => {
            let path = expand_user(path);
            let bytes = base64_decode(content_b64)?;
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
            }
            tokio::fs::write(&path, &bytes)
                .await
                .map_err(|e| format!("write {}: {e}", path.display()))
        }
        Operation::Delete { path, .. } => {
            let path = expand_user(path);
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| format!("delete {}: {e}", path.display()))
        }
    }
}

fn expand_user(path: &Path) -> PathBuf {
    let s = match path.to_str() {
        Some(s) => s,
        None => return path.to_path_buf(),
    };
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if s == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    path.to_path_buf()
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut table = [255u8; 256];
    for (i, &b) in ALPHABET.iter().enumerate() {
        table[b as usize] = i as u8;
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| *b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for b in bytes {
        let v = table[b as usize];
        if v == 255 {
            return Err(format!("non-base64 byte: 0x{b:02x}"));
        }
        buf = (buf << 6) | (v as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::apply::ops::{PendingGroup, PendingItem};
    use crate::templates::blueprint::{ApplyOperation, ApplyScope, ItemIdStrategy};

    fn cfg(allowed_paths: Vec<String>, allow_ops: Vec<ApplyOperation>) -> ApplyConfig {
        ApplyConfig {
            scope: ApplyScope {
                allowed_paths,
                deny_outside: true,
            },
            allowed_operations: allow_ops,
            pending_changes_path: "{output_dir}/.pending-changes.json".into(),
            schema_version: 1,
            item_id_strategy: ItemIdStrategy::ContentHash,
        }
    }

    fn pending(items: Vec<PendingItem>) -> PendingChanges {
        PendingChanges {
            schema_version: 1,
            automation_id: "auto".into(),
            run_id: "run".into(),
            generated_at: "now".into(),
            summary: format!("{} changes", items.len()),
            groups: vec![PendingGroup {
                id: "g".into(),
                title: "g".into(),
                items,
            }],
        }
    }

    #[tokio::test]
    async fn applies_move_inside_scope() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("a.txt");
        let to = dir.path().join("b.txt");
        std::fs::write(&from, b"hello").unwrap();

        let item = PendingItem {
            id: "1".into(),
            description: "move".into(),
            operation: Operation::Move {
                from: from.clone(),
                to: to.clone(),
            },
        };
        let p = pending(vec![item]);
        let c = cfg(
            vec![format!("{}/**", dir.path().display())],
            vec![ApplyOperation::Move],
        );
        let receipt = apply_selected(&p, &c, &["1".to_string()]).await;
        assert_eq!(receipt.outcomes.len(), 1);
        assert!(matches!(receipt.outcomes[0].outcome, ItemOutcome::Applied));
        assert!(!from.exists());
        assert!(to.exists());
    }

    #[tokio::test]
    async fn rejects_unselected_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"x").unwrap();

        let item = PendingItem {
            id: "skip-me".into(),
            description: "delete".into(),
            operation: Operation::Delete {
                path: path.clone(),
                must_be_empty: false,
            },
        };
        let p = pending(vec![item]);
        let c = cfg(
            vec![format!("{}/**", dir.path().display())],
            vec![ApplyOperation::Delete],
        );
        let receipt = apply_selected(&p, &c, &[]).await;
        assert_eq!(receipt.outcomes.len(), 0);
        assert!(path.exists(), "unselected items must not execute");
    }

    #[tokio::test]
    async fn rejects_out_of_scope_at_apply_time() {
        // Deliberately bypass the install-time check by handing
        // the executor a config that doesn't cover the path.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"x").unwrap();
        let item = PendingItem {
            id: "1".into(),
            description: "del".into(),
            operation: Operation::Delete {
                path: path.clone(),
                must_be_empty: false,
            },
        };
        let p = pending(vec![item]);
        let c = cfg(
            vec!["/elsewhere/**".to_string()],
            vec![ApplyOperation::Delete],
        );
        let receipt = apply_selected(&p, &c, &["1".to_string()]).await;
        assert!(matches!(
            receipt.outcomes[0].outcome,
            ItemOutcome::Rejected { .. }
        ));
        assert!(path.exists(), "rejected items must not execute");
    }

    #[tokio::test]
    async fn rejects_disallowed_operation_kind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"x").unwrap();
        let item = PendingItem {
            id: "1".into(),
            description: "del".into(),
            operation: Operation::Delete {
                path: path.clone(),
                must_be_empty: false,
            },
        };
        let p = pending(vec![item]);
        let c = cfg(
            vec![format!("{}/**", dir.path().display())],
            vec![ApplyOperation::Move], // Delete not in the whitelist
        );
        let receipt = apply_selected(&p, &c, &["1".to_string()]).await;
        assert!(matches!(
            receipt.outcomes[0].outcome,
            ItemOutcome::Rejected { ref reason } if reason.contains("delete")
        ));
        assert!(path.exists());
    }

    #[tokio::test]
    async fn write_creates_missing_parent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deep/file.txt");
        let item = PendingItem {
            id: "1".into(),
            description: "write".into(),
            operation: Operation::Write {
                path: path.clone(),
                content_b64: "aGVsbG8=".into(), // "hello"
            },
        };
        let p = pending(vec![item]);
        let c = cfg(
            vec![format!("{}/**", dir.path().display())],
            vec![ApplyOperation::Write],
        );
        let receipt = apply_selected(&p, &c, &["1".to_string()]).await;
        assert!(matches!(receipt.outcomes[0].outcome, ItemOutcome::Applied));
        let body = std::fs::read(&path).unwrap();
        assert_eq!(body, b"hello");
    }
}

//! Structured operation types and the on-disk pending-changes
//! envelope. Every operation is typed; no raw shell.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level pending-changes side-car. The LLM produces this
/// next to its run output; the apply executor consumes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingChanges {
    pub schema_version: u32,
    pub automation_id: String,
    pub run_id: String,
    pub generated_at: String,
    pub summary: String,
    pub groups: Vec<PendingGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingGroup {
    pub id: String,
    pub title: String,
    pub items: Vec<PendingItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingItem {
    /// Stable across reruns when the blueprint uses
    /// `item_id_strategy = "content_hash"`. Lets the apply UI
    /// remember "user already rejected this" between runs.
    pub id: String,
    /// User-facing description; never executed.
    pub description: String,
    pub operation: Operation,
}

/// One typed filesystem op. The deserializer rejects unknown
/// `type` values, so a malicious or mistaken `type: "shell"`
/// payload is dropped at parse time, never reaching the
/// executor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Move a file or directory from `from` to `to`. Both paths
    /// are validated against the blueprint's `apply.scope`.
    Move { from: PathBuf, to: PathBuf },
    /// Rename within the same parent directory. `path` is the
    /// existing item; `new_name` is the new basename only — no
    /// path separators.
    Rename { path: PathBuf, new_name: String },
    /// Create a directory (and any missing parents). Idempotent.
    Mkdir { path: PathBuf },
    /// Write a file. Content is base64-encoded to keep the JSON
    /// safe for arbitrary bytes; size is bounded by
    /// `max_bytes` in the blueprint config.
    Write { path: PathBuf, content_b64: String },
    /// Delete a file. Refuses to operate on directories
    /// regardless of `must_be_empty` for v1 — directory deletes
    /// require their own opt-in down the road.
    Delete {
        path: PathBuf,
        #[serde(default)]
        must_be_empty: bool,
    },
}

impl Operation {
    /// Operation kind as a stable string; matches the blueprint's
    /// `apply.allowed_operations` enum strings.
    pub fn kind(&self) -> &'static str {
        match self {
            Operation::Move { .. } => "move",
            Operation::Rename { .. } => "rename",
            Operation::Mkdir { .. } => "mkdir",
            Operation::Write { .. } => "write",
            Operation::Delete { .. } => "delete",
        }
    }

    /// Every path the operation will touch. Used by the validator
    /// to check `apply.scope.allowed_paths` containment.
    pub fn paths(&self) -> Vec<&PathBuf> {
        match self {
            Operation::Move { from, to } => vec![from, to],
            Operation::Rename { path, .. } => vec![path],
            Operation::Mkdir { path } => vec![path],
            Operation::Write { path, .. } => vec![path],
            Operation::Delete { path, .. } => vec![path],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_operation_type() {
        // The classic foot-cannon test: a `type: "shell"` entry
        // in the wire format must fail deserialization, not
        // silently turn into a garbage variant.
        let bad = serde_json::json!({
            "type": "shell",
            "command": "rm -rf /",
        });
        let r: Result<Operation, _> = serde_json::from_value(bad);
        assert!(r.is_err(), "shell ops must be rejected at parse time");
    }

    #[test]
    fn rejects_unknown_field_inside_known_op() {
        let bad = serde_json::json!({
            "type": "move",
            "from": "/tmp/a",
            "to": "/tmp/b",
            "elevated": true,
        });
        let r: Result<Operation, _> = serde_json::from_value(bad);
        assert!(r.is_err(), "extraneous fields must be rejected");
    }

    #[test]
    fn parses_each_known_op() {
        let cases = [
            serde_json::json!({"type": "move", "from": "/a", "to": "/b"}),
            serde_json::json!({"type": "rename", "path": "/a", "new_name": "b"}),
            serde_json::json!({"type": "mkdir", "path": "/a"}),
            serde_json::json!({"type": "write", "path": "/a", "content_b64": ""}),
            serde_json::json!({"type": "delete", "path": "/a"}),
        ];
        for c in cases {
            let _: Operation = serde_json::from_value(c.clone())
                .unwrap_or_else(|e| panic!("failed to parse {c}: {e}"));
        }
    }

    #[test]
    fn kind_string_stable() {
        let m = Operation::Move {
            from: "/a".into(),
            to: "/b".into(),
        };
        assert_eq!(m.kind(), "move");
    }
}

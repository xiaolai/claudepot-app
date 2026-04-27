//! Substitution-table builder and the import plan.
//!
//! See `dev-docs/project-migrate-spec.md` §5.2.
//!
//! The substitution table is a `Vec<(from, to)>` sorted by `from.len()`
//! descending so longer prefixes win. It's the single source of truth
//! for every cross-OS path rewrite the importer performs.

use crate::migrate::nfc::nfc;
use crate::path_utils::simplify_windows_path;
use crate::project_sanitize::sanitize_path;
use serde::{Deserialize, Serialize};

/// One substitution rule. Both sides are NFC-normalized and run
/// through `simplify_windows_path` before being added to the table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubstitutionRule {
    pub from: String,
    pub to: String,
    /// Origin tag — useful for telling the user *why* a rule exists
    /// when they paste an unexpected `--remap`.
    pub origin: RuleOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOrigin {
    /// Project cwd → target cwd (one per project).
    ProjectCwd,
    /// Project canonical git root → target canonical git root.
    /// Drives the auto-memory dir relocation.
    CanonicalGitRoot,
    /// Source HOME → target HOME.
    Home,
    /// Source `CLAUDE_CONFIG_DIR` → target `CLAUDE_CONFIG_DIR`.
    ClaudeConfigDir,
    /// User-supplied `--remap source=target`.
    UserRemap,
}

/// Table of substitution rules, kept sorted by `from.len()` desc so
/// longer prefixes win at apply time. Construct via the builder; the
/// raw `Vec<SubstitutionRule>` is exposed so adapters can serialize it
/// for the import-plan UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubstitutionTable {
    pub rules: Vec<SubstitutionRule>,
}

impl SubstitutionTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a rule. Both sides are normalized before insertion. If
    /// `from == to` the rule is dropped (no-op rewrite).
    pub fn push(&mut self, from: &str, to: &str, origin: RuleOrigin) {
        let from = nfc(&simplify_windows_path(from));
        let to = nfc(&simplify_windows_path(to));
        if from == to {
            return;
        }
        self.rules.push(SubstitutionRule { from, to, origin });
    }

    /// Sort in-place by `from.len()` descending. Call once after all
    /// rules are pushed; `apply_path` relies on the order.
    pub fn finalize(&mut self) {
        self.rules.sort_by(|a, b| b.from.len().cmp(&a.from.len()));
    }

    /// Apply the table to a single path string. Returns the rewritten
    /// path on the first matching rule, or the original on miss.
    /// Assumes `finalize` has been called.
    ///
    /// Match semantics mirror `project_rewrite::rewrite_path_string`:
    /// exact match OR prefix-match with a `\` or `/` boundary. Both
    /// separators are accepted because cwd strings cross OS boundaries.
    pub fn apply_path(&self, s: &str) -> Option<String> {
        for rule in &self.rules {
            if s == rule.from {
                return Some(rule.to.clone());
            }
            for sep in ['\\', '/'] {
                let boundary = format!("{}{sep}", rule.from);
                if let Some(rest) = s.strip_prefix(&boundary) {
                    return Some(format!("{}{sep}{rest}", rule.to));
                }
            }
        }
        None
    }

    /// Total rule count (for reporting).
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// Compute the target slug given the target canonical git root, per
/// §5.3:
///   `target_slug = sanitize_path(NFC(simplify_windows_path(target_canonical_git_root)))`
pub fn target_slug(target_canonical_git_root: &str) -> String {
    sanitize_path(&nfc(&simplify_windows_path(target_canonical_git_root)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longer_prefix_wins() {
        let mut t = SubstitutionTable::new();
        t.push("/a", "/X", RuleOrigin::Home);
        t.push("/a/b", "/Y", RuleOrigin::ProjectCwd);
        t.finalize();
        assert_eq!(t.apply_path("/a/b/file"), Some("/Y/file".to_string()));
        assert_eq!(t.apply_path("/a/c"), Some("/X/c".to_string()));
    }

    #[test]
    fn boundary_separator_preserved() {
        let mut t = SubstitutionTable::new();
        t.push(r"C:\src\proj", r"D:\code\proj", RuleOrigin::ProjectCwd);
        t.finalize();
        // Backslash boundary
        assert_eq!(
            t.apply_path(r"C:\src\proj\a\b.rs"),
            Some(r"D:\code\proj\a\b.rs".to_string())
        );
    }

    #[test]
    fn unix_boundary_works() {
        let mut t = SubstitutionTable::new();
        t.push("/Users/joker", "/home/alice", RuleOrigin::Home);
        t.finalize();
        assert_eq!(
            t.apply_path("/Users/joker/proj/x.rs"),
            Some("/home/alice/proj/x.rs".to_string())
        );
    }

    #[test]
    fn no_match_returns_none() {
        let mut t = SubstitutionTable::new();
        t.push("/a", "/b", RuleOrigin::Home);
        t.finalize();
        assert_eq!(t.apply_path("/c/d"), None);
    }

    #[test]
    fn nfc_normalizes_inputs() {
        // NFD `é` (U+0065 U+0301) and NFC `é` (U+00E9) sanitize
        // differently — the table forces NFC on push so the rule
        // matches paths in either form once both have been normalized.
        let mut t = SubstitutionTable::new();
        // Push in NFD form.
        t.push(
            "/Users/caf\u{0065}\u{0301}",
            "/home/cafe",
            RuleOrigin::ProjectCwd,
        );
        t.finalize();
        // Lookup in NFC form should hit (after lookup-side NFC).
        let nfc_lookup = nfc("/Users/caf\u{00E9}/sub");
        assert_eq!(
            t.apply_path(&nfc_lookup),
            Some("/home/cafe/sub".to_string())
        );
    }

    #[test]
    fn drops_no_op_rules() {
        let mut t = SubstitutionTable::new();
        t.push("/a", "/a", RuleOrigin::Home);
        t.finalize();
        assert!(t.is_empty());
    }

    #[test]
    fn target_slug_unix() {
        assert_eq!(target_slug("/Users/joker/x"), "-Users-joker-x");
    }

    #[test]
    fn target_slug_windows_drive() {
        assert_eq!(
            target_slug(r"C:\Users\alice\x"),
            "C--Users-alice-x"
        );
    }

    #[test]
    fn target_slug_unc() {
        assert_eq!(
            target_slug(r"\\nas\share\proj"),
            "--nas-share-proj"
        );
    }

    #[test]
    fn target_slug_strips_verbatim_prefix() {
        // Verbatim `\\?\C:\Users\joker` must be stripped before sanitize,
        // otherwise we'd produce a slug with leading `--` from the
        // verbatim signature.
        assert_eq!(
            target_slug(r"\\?\C:\Users\joker"),
            "C--Users-joker"
        );
    }

    #[test]
    fn target_slug_nfc_normalizes() {
        // NFD path → recombined NFC slug. Both should produce the same
        // sanitized slug after normalization.
        let nfd = "/tmp/caf\u{0065}\u{0301}";
        let nfc_form = "/tmp/caf\u{00E9}";
        assert_eq!(target_slug(nfd), target_slug(nfc_form));
    }
}

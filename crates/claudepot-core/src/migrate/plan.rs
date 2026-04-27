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
    ///
    /// Lookup-side normalization: rules are NFC-normalized at insert
    /// time, so the lookup string must also be NFC-normalized before
    /// matching, otherwise an HFS+/APFS path that round-tripped as NFD
    /// would silently miss (audit Correctness finding). We also run
    /// `simplify_windows_path` so a verbatim `\\?\C:\…` lookup matches
    /// a non-verbatim rule.
    pub fn apply_path(&self, s: &str) -> Option<String> {
        let lookup = nfc(&simplify_windows_path(s));
        let lookup = lookup.as_str();
        for rule in &self.rules {
            if lookup == rule.from {
                return Some(target_with_native_sep(&rule.to, ""));
            }
            for sep in ['\\', '/'] {
                let boundary = format!("{}{sep}", rule.from);
                if let Some(rest) = lookup.strip_prefix(&boundary) {
                    // Cross-OS rewrite: when the target side is
                    // Windows-shape but the source separator was `/`,
                    // the legacy code emitted mixed paths like
                    // `C:\x/y`. Coerce the boundary + suffix into the
                    // target's native separator so the result stays
                    // consistent (audit Correctness finding row 2).
                    return Some(target_with_native_sep(&rule.to, rest));
                }
            }
        }
        None
    }

    /// Total rule count. Used by tests and by adapters that surface
    /// a "N substitution rules" line in the import-plan UI; not
    /// referenced from the orchestrator yet.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    #[allow(dead_code)]
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

/// Construct `<to><sep><suffix>`, picking the native separator for the
/// `to` side. If `to` is Windows-shape (drive letter or UNC), the
/// suffix's `/` separators are flipped to `\` and the boundary is
/// `\`. If `to` is Unix-shape, suffix `\` separators are flipped to
/// `/`. If suffix is empty, only `to` is returned.
fn target_with_native_sep(to: &str, suffix: &str) -> String {
    let to_is_windows = is_windows_shape(to);
    if suffix.is_empty() {
        return to.to_string();
    }
    let sep = if to_is_windows { '\\' } else { '/' };
    let canonical_suffix: String = if to_is_windows {
        suffix.replace('/', "\\")
    } else {
        suffix.replace('\\', "/")
    };
    format!("{to}{sep}{canonical_suffix}")
}

fn is_windows_shape(p: &str) -> bool {
    if p.starts_with(r"\\") || p.starts_with("//") {
        // UNC.
        return true;
    }
    let bytes = p.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        return true;
    }
    false
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
    fn apply_path_normalizes_raw_nfd_lookup() {
        // Audit Correctness fix: rules are NFC-normalized at insert,
        // but the lookup string also needs normalization. A path
        // round-tripped through HFS+ may come back as NFD; without
        // lookup-side NFC the rule would silently miss. Pass raw NFD
        // explicitly here — no caller-side NFC — to exercise the gate.
        let mut t = SubstitutionTable::new();
        // Insert rule in NFC form (precomposed `é`).
        t.push("/Users/caf\u{00E9}/x", "/home/cafe/x", RuleOrigin::ProjectCwd);
        t.finalize();
        // Lookup with raw NFD form (`e` + combining acute).
        let raw_nfd = "/Users/caf\u{0065}\u{0301}/x/sub.rs";
        let result = t.apply_path(raw_nfd);
        assert_eq!(result, Some("/home/cafe/x/sub.rs".to_string()));
    }

    #[test]
    fn apply_path_normalizes_verbatim_windows_lookup() {
        // Same shape but for verbatim `\\?\C:\…` lookups: rules use
        // the simplified form, lookup must too.
        let mut t = SubstitutionTable::new();
        t.push(
            r"C:\Users\joker\x",
            r"D:\code\x",
            RuleOrigin::ProjectCwd,
        );
        t.finalize();
        let verbatim_lookup = r"\\?\C:\Users\joker\x\foo.rs";
        let result = t.apply_path(verbatim_lookup);
        assert_eq!(result, Some(r"D:\code\x\foo.rs".to_string()));
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

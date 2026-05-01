//! Deny-by-default scope validator.
//!
//! Every operation is rejected unless every path it touches:
//!
//! 1. Lives under at least one allowed-path glob from the
//!    blueprint's `apply.scope.allowed_paths`, AFTER
//!    canonicalization (defeats `../` traversal).
//! 2. Resolves to a real path within the allowed scope (defeats
//!    a symlink that points outside).
//! 3. Has a kind that's listed in the blueprint's
//!    `apply.allowed_operations`.
//! 4. For `delete`: target is a regular file, not a directory.
//! 5. For `write`: bounded by `max_bytes` (default 10 MB).
//!
//! Paths that don't yet exist (a `move` target, a `mkdir`
//! target, a `write` target) are validated by canonicalizing
//! their parent and joining the basename.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::templates::blueprint::{ApplyConfig, ApplyOperation, ApplyScope};

use super::ops::Operation;

/// Per-item validation outcome the executor consumes.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValidationError {
    #[error("operation kind {0:?} is not allowed by this template")]
    OperationNotAllowed(String),

    #[error("path {0} resolves outside the template's allowed scope")]
    OutsideScope(String),

    #[error("path {0} traverses a symlink that points outside the allowed scope")]
    SymlinkEscape(String),

    #[error("rename target name {0:?} is not a plain basename (no separators allowed)")]
    InvalidRenameName(String),

    #[error("delete refused: {0} is a directory; only files may be deleted in v1")]
    DeleteOnDirectory(String),

    #[error("write content exceeds max_bytes ({0} > {1})")]
    WriteTooLarge(u64, u64),

    #[error("write content is not valid base64: {0}")]
    InvalidBase64(String),

    #[error("operation references a path that has no parent: {0}")]
    NoParent(String),
}

/// Default max bytes for a `write` operation's payload. Templates
/// that need more declare it in their blueprint runtime config.
pub const DEFAULT_WRITE_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Validate one pending operation against a blueprint's apply
/// configuration. Returns `Ok(())` only if every check passes.
pub fn validate_item(op: &Operation, apply: &ApplyConfig) -> Result<(), ValidationError> {
    // 1. Kind whitelist.
    if !kind_allowed(op, &apply.allowed_operations) {
        return Err(ValidationError::OperationNotAllowed(op.kind().to_string()));
    }

    // 2. Op-specific shape checks.
    match op {
        Operation::Rename { new_name, .. } => {
            if new_name.contains('/')
                || new_name.contains('\\')
                || new_name == ".."
                || new_name == "."
                || new_name.is_empty()
            {
                return Err(ValidationError::InvalidRenameName(new_name.clone()));
            }
        }
        Operation::Write { content_b64, .. } => {
            // Decode just to learn the length, but don't keep the bytes.
            // We could use base64::decoded_len but the crate variants
            // make this awkward; allocate a small Vec and drop.
            let bytes = base64_decode(content_b64).map_err(ValidationError::InvalidBase64)?;
            if bytes.len() as u64 > DEFAULT_WRITE_MAX_BYTES {
                return Err(ValidationError::WriteTooLarge(
                    bytes.len() as u64,
                    DEFAULT_WRITE_MAX_BYTES,
                ));
            }
        }
        Operation::Delete { path, .. } => {
            if path.exists() && path.is_dir() {
                return Err(ValidationError::DeleteOnDirectory(path.display().to_string()));
            }
        }
        Operation::Move { .. } | Operation::Mkdir { .. } => {}
    }

    // 3. Path containment. Every path the op touches must resolve
    //    inside the allowed scope.
    for path in op.paths() {
        check_in_scope(path, &apply.scope)?;
    }

    Ok(())
}

fn kind_allowed(op: &Operation, allow: &[ApplyOperation]) -> bool {
    let needed = match op {
        Operation::Move { .. } => ApplyOperation::Move,
        Operation::Rename { .. } => ApplyOperation::Rename,
        Operation::Mkdir { .. } => ApplyOperation::Mkdir,
        Operation::Write { .. } => ApplyOperation::Write,
        Operation::Delete { .. } => ApplyOperation::Delete,
    };
    allow.contains(&needed)
}

/// Canonicalize the path (resolving any existing prefix and
/// symlinks) and verify it sits under at least one
/// allowed_paths glob. Paths that don't exist yet are
/// canonicalized via their nearest existing parent.
fn check_in_scope(path: &Path, scope: &ApplyScope) -> Result<(), ValidationError> {
    if scope.allowed_paths.is_empty() {
        // Empty allow-list = no apply allowed. Reject.
        return Err(ValidationError::OutsideScope(path.display().to_string()));
    }

    let resolved = resolve_for_check(path)?;

    // Symlink-escape check: if the original path differs from
    // its canonical form via a symlink hop, ensure the canonical
    // form still lives under the scope.
    let mut allowed = false;
    for glob in &scope.allowed_paths {
        if path_matches_glob(&resolved, glob) {
            allowed = true;
            break;
        }
    }
    if !allowed {
        // Not in scope: distinguish "outside scope" from "symlink
        // escape" for the error message.
        if path.exists() && resolved != normalize(path) {
            return Err(ValidationError::SymlinkEscape(path.display().to_string()));
        }
        return Err(ValidationError::OutsideScope(path.display().to_string()));
    }
    Ok(())
}

/// Canonicalize-or-best-effort. For paths that don't exist (a
/// `move` target's destination, for instance), canonicalize
/// their nearest existing parent and re-join the unmatched
/// suffix. Then normalize `..` segments so a missing-prefix
/// traversal like `/tmp/foo/../../etc/passwd` resolves to
/// `/etc/passwd` rather than being left in raw form.
///
/// We also normalize the *input* upfront when canonicalize
/// fails. This is the path that handles `Path::file_name()`
/// returning `None` for `..` segments — without normalization,
/// the walk-up loop bottoms out at NoParent for any path that
/// traverses through a missing prefix.
fn resolve_for_check(path: &Path) -> Result<PathBuf, ValidationError> {
    let path = expand_user(path);
    if let Ok(canon) = std::fs::canonicalize(&path) {
        return Ok(canon);
    }
    // Pre-normalize the input so `..` segments after a missing
    // prefix collapse before we walk. Otherwise
    // `Path::file_name()` returns `None` for the `..` and the
    // loop bottoms out at NoParent.
    let pre_normalized = normalize(&path);

    // Walk up to the nearest existing parent.
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor: PathBuf = pre_normalized.clone();
    loop {
        if let Ok(canon) = std::fs::canonicalize(&cursor) {
            // Re-join the captured tail (in reverse-chronological
            // order, hence the .rev() below).
            let mut full = canon;
            for piece in tail.iter().rev() {
                full = full.join(piece);
            }
            return Ok(normalize(&full));
        }
        match cursor.file_name() {
            Some(name) => tail.push(name.to_os_string()),
            None => {
                // Reached a path whose final component is `..`
                // or empty. Treat as out-of-scope — neither in
                // nor near any allowed root.
                return Err(ValidationError::OutsideScope(
                    pre_normalized.display().to_string(),
                ));
            }
        }
        match cursor.parent() {
            Some(p) if p != cursor => {
                let parent = p.to_path_buf();
                cursor = parent;
            }
            _ => {
                return Err(ValidationError::OutsideScope(
                    pre_normalized.display().to_string(),
                ));
            }
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

/// Normalize without canonicalizing — strip `.` segments and
/// collapse trailing separators. Used only for the "did the
/// canonical form change due to a symlink?" comparison.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Tiny glob matcher. Supports `**` (any depth), `*` (any
/// segment but not `/`), and literal text. Adequate for
/// `apply.scope.allowed_paths` patterns; not a full glob crate.
///
/// Glob prefixes (the literal part before any wildcard) are
/// canonicalized so platforms with symlinked roots (macOS's
/// `/tmp` → `/private/tmp`) compare correctly against
/// canonicalized targets.
fn path_matches_glob(path: &Path, pattern: &str) -> bool {
    let pat_expanded = expand_user(Path::new(pattern));
    let pat_str = match pat_expanded.to_str() {
        Some(s) => s.to_string(),
        None => return false,
    };
    // Canonicalize the literal prefix of the glob (everything
    // up to the first wildcard segment).
    let canonical_pat = canonicalize_glob_prefix(&pat_str);
    let path_str = match path.to_str() {
        Some(s) => s.to_string(),
        None => return false,
    };
    glob_match(&path_str, &canonical_pat) || glob_match(&path_str, &pat_str)
}

/// Resolve the literal prefix of a glob (the part before any
/// segment containing `*` or `?`) and return the resulting
/// pattern. Used to make scope checks stable on platforms with
/// symlinked roots.
fn canonicalize_glob_prefix(pattern: &str) -> String {
    let segments: Vec<&str> = pattern.split('/').collect();
    let mut prefix = PathBuf::new();
    let mut wild_idx = segments.len();
    for (i, seg) in segments.iter().enumerate() {
        if seg.contains('*') || seg.contains('?') {
            wild_idx = i;
            break;
        }
        if i == 0 && seg.is_empty() {
            // Leading "/" — start absolute.
            prefix.push("/");
        } else {
            prefix.push(seg);
        }
    }
    let canonical = std::fs::canonicalize(&prefix).unwrap_or(prefix);
    let mut out = canonical.display().to_string();
    if wild_idx < segments.len() {
        out.push('/');
        out.push_str(&segments[wild_idx..].join("/"));
    }
    out
}

/// Compare strings with `*` and `**` semantics. `**` matches any
/// sequence including separators; `*` matches any sequence
/// excluding separators.
fn glob_match(haystack: &str, pattern: &str) -> bool {
    let h: Vec<char> = haystack.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    glob_match_inner(&h, 0, &p, 0)
}

fn glob_match_inner(h: &[char], hi: usize, p: &[char], pi: usize) -> bool {
    let mut hi = hi;
    let mut pi = pi;
    loop {
        if pi >= p.len() {
            return hi >= h.len();
        }
        if p[pi] == '*' {
            // `**` — any chars including '/'.
            let double = pi + 1 < p.len() && p[pi + 1] == '*';
            let next_pi = if double { pi + 2 } else { pi + 1 };
            // Skip a trailing separator after `**`.
            let next_pi = if double && next_pi < p.len() && p[next_pi] == '/' {
                next_pi + 1
            } else {
                next_pi
            };
            // Try matching zero or more chars from h.
            for skip in 0..=h.len().saturating_sub(hi) {
                if !double {
                    // `*` can't cross '/'.
                    let chunk: &[char] = &h[hi..hi + skip];
                    if chunk.contains(&'/') {
                        break;
                    }
                }
                if glob_match_inner(h, hi + skip, p, next_pi) {
                    return true;
                }
            }
            return false;
        }
        if hi >= h.len() {
            return false;
        }
        if p[pi] == '?' {
            if h[hi] == '/' {
                return false;
            }
        } else if p[pi] != h[hi] {
            return false;
        }
        hi += 1;
        pi += 1;
    }
}

/// Tiny base64 decoder so we don't pull a new dep just for one
/// decode call. Standard alphabet, no padding required.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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
    use crate::templates::blueprint::{ApplyConfig, ApplyOperation, ApplyScope, ItemIdStrategy};
    use std::path::PathBuf;

    fn config(allow_ops: &[ApplyOperation], allowed_paths: &[&str]) -> ApplyConfig {
        ApplyConfig {
            scope: ApplyScope {
                allowed_paths: allowed_paths.iter().map(|s| s.to_string()).collect(),
                deny_outside: true,
            },
            allowed_operations: allow_ops.to_vec(),
            pending_changes_path: "{output_dir}/.pending-changes.json".into(),
            schema_version: 1,
            item_id_strategy: ItemIdStrategy::ContentHash,
        }
    }

    #[test]
    fn empty_scope_blocks_everything() {
        let op = Operation::Mkdir {
            path: "/tmp/anywhere".into(),
        };
        let cfg = config(&[ApplyOperation::Mkdir], &[]);
        let err = validate_item(&op, &cfg).unwrap_err();
        assert!(matches!(err, ValidationError::OutsideScope(_)));
    }

    #[test]
    fn kind_not_in_allow_list_rejects() {
        let op = Operation::Delete {
            path: "/tmp/x".into(),
            must_be_empty: false,
        };
        let cfg = config(&[ApplyOperation::Move], &["/tmp/**"]);
        let err = validate_item(&op, &cfg).unwrap_err();
        assert!(matches!(err, ValidationError::OperationNotAllowed(ref s) if s == "delete"));
    }

    #[test]
    fn rename_with_separator_rejects() {
        let op = Operation::Rename {
            path: "/tmp/x".into(),
            new_name: "../y".into(),
        };
        let cfg = config(&[ApplyOperation::Rename], &["/tmp/**"]);
        let err = validate_item(&op, &cfg).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidRenameName(_)));
    }

    #[test]
    fn rename_dotdot_rejects() {
        let op = Operation::Rename {
            path: "/tmp/x".into(),
            new_name: "..".into(),
        };
        let cfg = config(&[ApplyOperation::Rename], &["/tmp/**"]);
        let err = validate_item(&op, &cfg).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidRenameName(_)));
    }

    #[test]
    fn write_too_large_rejects() {
        // 10MB+1 of zeros → encoded → still considered too large.
        let big = "A".repeat((DEFAULT_WRITE_MAX_BYTES as usize + 1) * 4 / 3 + 4);
        let op = Operation::Write {
            path: "/tmp/x".into(),
            content_b64: big,
        };
        let cfg = config(&[ApplyOperation::Write], &["/tmp/**"]);
        let err = validate_item(&op, &cfg).unwrap_err();
        assert!(matches!(err, ValidationError::WriteTooLarge(_, _)));
    }

    #[test]
    fn delete_on_directory_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let op = Operation::Delete {
            path: dir.path().to_path_buf(),
            must_be_empty: false,
        };
        let cfg = config(
            &[ApplyOperation::Delete],
            &[&format!("{}/**", dir.path().display())],
        );
        let err = validate_item(&op, &cfg).unwrap_err();
        assert!(matches!(err, ValidationError::DeleteOnDirectory(_)));
    }

    #[test]
    fn outside_scope_traversal_rejects() {
        let op = Operation::Mkdir {
            path: "/tmp/foo/../../etc/passwd".into(),
        };
        let cfg = config(&[ApplyOperation::Mkdir], &["/tmp/foo/**"]);
        let err = validate_item(&op, &cfg).unwrap_err();
        assert!(
            matches!(
                err,
                ValidationError::OutsideScope(_) | ValidationError::SymlinkEscape(_)
            ),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn glob_double_star_matches_subtree() {
        let mut p = PathBuf::from(std::env::temp_dir());
        p.push("claudepot-test-deep");
        p.push("a");
        p.push("b");
        p.push("c");
        std::fs::create_dir_all(&p).ok();
        let op = Operation::Mkdir { path: p.clone() };
        let cfg = config(
            &[ApplyOperation::Mkdir],
            &[&format!("{}/**", std::env::temp_dir().join("claudepot-test-deep").display())],
        );
        validate_item(&op, &cfg).unwrap();
        std::fs::remove_dir_all(std::env::temp_dir().join("claudepot-test-deep")).ok();
    }

    #[test]
    fn glob_single_star_does_not_cross_separator() {
        // Pattern: /tmp/foo/*  matches /tmp/foo/a but not /tmp/foo/a/b.
        assert!(glob_match("/tmp/foo/a", "/tmp/foo/*"));
        assert!(!glob_match("/tmp/foo/a/b", "/tmp/foo/*"));
        assert!(glob_match("/tmp/foo/a/b", "/tmp/foo/**"));
    }

    #[test]
    fn well_formed_move_inside_scope_passes() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("a.txt");
        let to = dir.path().join("b.txt");
        std::fs::write(&from, b"x").unwrap();
        let op = Operation::Move { from, to };
        let cfg = config(
            &[ApplyOperation::Move],
            &[&format!("{}/**", dir.path().display())],
        );
        validate_item(&op, &cfg).unwrap();
    }

    #[test]
    fn base64_decode_basic() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
        assert!(base64_decode("not-base64!").is_err());
    }
}

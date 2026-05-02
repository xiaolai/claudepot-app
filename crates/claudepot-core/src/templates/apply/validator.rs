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
                return Err(ValidationError::DeleteOnDirectory(
                    path.display().to_string(),
                ));
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
        // Strip Windows verbatim `\\?\` prefix so the resolved
        // form compares equal to the user-typed glob prefix.
        let s = canon.display().to_string();
        let simplified = crate::path_utils::simplify_windows_path(&s);
        return Ok(PathBuf::from(simplified));
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
            // Strip the Windows verbatim prefix so the joined form
            // matches user-friendly globs.
            let canon_str = canon.display().to_string();
            let simplified = crate::path_utils::simplify_windows_path(&canon_str);
            let mut full = PathBuf::from(simplified);
            // Re-join the captured tail (in reverse-chronological
            // order, hence the .rev() below).
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

/// Match a candidate path against one `apply.scope.allowed_paths`
/// glob, in a way that is correct on Windows as well as on
/// Unix.
///
/// Implementation notes:
///
/// - Patterns are normalized to forward-slash form before
///   compilation. `globset` does its own separator handling on
///   Windows, but we additionally normalize the haystack to `/`
///   so a pattern like `~/Downloads/**` matches a Windows path
///   `C:\Users\joker\Downloads\foo.txt`.
/// - The literal prefix (everything before the first wildcard) is
///   canonicalized so platforms with symlinked roots (macOS's
///   `/tmp` → `/private/tmp`) compare correctly against the
///   already-canonicalized haystack. After canonicalize on
///   Windows we strip the verbatim `\\?\` prefix via
///   `simplify_windows_path` so user-friendly paths still match.
/// - Matching is case-insensitive on Windows and macOS to track
///   filesystem semantics (NTFS and APFS are case-insensitive by
///   default; Linux ext4 is case-sensitive).
fn path_matches_glob(path: &Path, pattern: &str) -> bool {
    let pat_expanded = expand_user(Path::new(pattern));
    let pat_str = match pat_expanded.to_str() {
        Some(s) => s.to_string(),
        None => return false,
    };
    let canonical_pat = canonicalize_glob_prefix(&pat_str);
    let haystack = match path.to_str() {
        Some(s) => normalize_separators(s),
        None => return false,
    };
    glob_compile_and_match(&canonical_pat, &haystack) || glob_compile_and_match(&pat_str, &haystack)
}

fn glob_compile_and_match(pattern: &str, haystack: &str) -> bool {
    // Normalize both sides so a `\`-shaped pattern matches a
    // `/`-shaped haystack and vice versa. Callers that already
    // normalized are idempotent here.
    let normalized_pattern = normalize_separators(pattern);
    let normalized_haystack = normalize_separators(haystack);
    let mut builder = globset::GlobBuilder::new(&normalized_pattern);
    builder.literal_separator(true);
    // NTFS + APFS are case-insensitive by default; Linux ext4 is
    // case-sensitive. Match the host filesystem's behavior so the
    // validator doesn't reject legitimate operations whose paths
    // differ from the canonical form only in case.
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        builder.case_insensitive(true);
    }
    match builder.build() {
        Ok(g) => g.compile_matcher().is_match(&normalized_haystack),
        Err(_) => false,
    }
}

/// Replace `\` with `/` unconditionally so the same glob works on
/// Windows (`\`) and Unix (`/`). `globset` interprets `\` as a
/// regex-style escape on Unix; converting to `/` avoids that
/// pitfall and gives a consistent matching surface across hosts.
fn normalize_separators(s: &str) -> String {
    s.replace('\\', "/")
}

/// Resolve the literal prefix of a glob (the part before any
/// segment containing `*` or `?`) and return the resulting
/// pattern. Used to make scope checks stable on platforms with
/// symlinked roots.
fn canonicalize_glob_prefix(pattern: &str) -> String {
    // Split on either separator so Windows-shaped patterns also
    // work. The output is forward-slash-shaped so it pairs with
    // the haystack normalization below.
    let segments: Vec<&str> = pattern.split(['/', '\\']).collect();
    let mut prefix = PathBuf::new();
    let mut wild_idx = segments.len();
    for (i, seg) in segments.iter().enumerate() {
        if seg.contains('*') || seg.contains('?') {
            wild_idx = i;
            break;
        }
        if i == 0 && seg.is_empty() {
            // Leading "/" — start absolute (Unix only). On
            // Windows the first segment is the drive letter or
            // empty for UNC; either way Path::push handles it.
            prefix.push("/");
        } else {
            prefix.push(seg);
        }
    }
    let canonical = match std::fs::canonicalize(&prefix) {
        Ok(p) => {
            // On Windows canonicalize returns the verbatim
            // `\\?\C:\…` form. Strip it so user-friendly paths
            // still compare equal — this is the rule documented
            // in `.claude/rules/paths.md`.
            let s = p.display().to_string();
            let simplified = crate::path_utils::simplify_windows_path(&s);
            PathBuf::from(simplified)
        }
        Err(_) => prefix,
    };
    let mut out = normalize_separators(&canonical.display().to_string());
    if wild_idx < segments.len() {
        out.push('/');
        out.push_str(&segments[wild_idx..].join("/"));
    }
    out
}

/// Tiny base64 decoder so we don't pull a new dep just for one
/// decode call. Standard alphabet, no padding required.
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
            &[&format!(
                "{}/**",
                std::env::temp_dir().join("claudepot-test-deep").display()
            )],
        );
        validate_item(&op, &cfg).unwrap();
        std::fs::remove_dir_all(std::env::temp_dir().join("claudepot-test-deep")).ok();
    }

    #[test]
    fn glob_single_star_does_not_cross_separator() {
        // Pattern: /tmp/foo/*  matches /tmp/foo/a but not /tmp/foo/a/b.
        assert!(glob_compile_and_match("/tmp/foo/*", "/tmp/foo/a"));
        assert!(!glob_compile_and_match("/tmp/foo/*", "/tmp/foo/a/b"));
        assert!(glob_compile_and_match("/tmp/foo/**", "/tmp/foo/a/b"));
    }

    #[test]
    fn glob_normalizes_windows_separators() {
        // Even on Unix, the matcher accepts a `\\`-shaped haystack
        // via `normalize_separators`. This pins Windows behavior
        // on macOS/Linux CI.
        assert!(glob_compile_and_match(
            r"C:\Users\joker\Downloads\**",
            r"C:\Users\joker\Downloads\sub\file.txt",
        ));
    }

    #[test]
    fn glob_case_insensitive_on_windows_macos() {
        // NTFS + APFS are case-insensitive; the matcher matches.
        // Linux (case-sensitive) would reject the differing case
        // — that's the correct OS-tracking behavior.
        let matches = glob_compile_and_match("/foo/**/*.PDF", "/foo/sub/file.pdf");
        if cfg!(any(target_os = "windows", target_os = "macos")) {
            assert!(matches, "expected case-insensitive match on this OS");
        } else {
            assert!(!matches, "expected case-sensitive on Linux");
        }
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

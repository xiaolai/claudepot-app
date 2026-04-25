//! Cross-platform path string helpers.
//!
//! Every path-processing path (`resolve_path`, `find_canonical_git_root`,
//! sanitization, display) must flow through this module so Windows
//! peculiarities (verbatim `\\?\` prefix, UNC forms, drive letters, `\`
//! separator) are handled uniformly. See `.claude/rules/paths.md`.

/// Strip Windows verbatim / extended-length path prefix (`\\?\`).
///
/// `std::fs::canonicalize` on Windows returns `\\?\C:\Users\...` (or
/// `\\?\UNC\server\share\...` for UNC paths). CC writes the non-verbatim
/// form (`C:\Users\...` / `\\server\share\...`) into its project slugs
/// and session `cwd` fields, so feeding a verbatim path into
/// `sanitize_path` would produce a slug that does not match CC's.
///
/// Return value is always an owned `String` because the UNC-verbatim
/// form cannot be converted in-place (`\\?\UNC\srv\sh\p` → `\\srv\sh\p`
/// requires prepending two backslashes after removing seven chars).
///
/// On all platforms this is a pure string op — no filesystem access.
/// Safe to call on already-simplified paths (no-op) and on non-Windows
/// paths (no-op).
pub fn simplify_windows_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        if let Some(after_unc) = rest.strip_prefix(r"UNC\") {
            return format!(r"\\{}", after_unc);
        }
        return rest.to_string();
    }
    path.to_string()
}

/// Detect a Windows-shaped absolute path string regardless of host OS.
///
/// Recognizes:
///   - `C:\...` and `C:/...` (drive-letter absolute, any case letter)
///   - `\\server\share\...` (UNC) and `//server/share/...` (some tools
///     emit forward slashes)
///   - `\\?\C:\...` and `\\?\UNC\server\share\...` (verbatim;
///     `simplify_windows_path` is the preferred normalizer but we
///     classify them as absolute either way)
///
/// On Unix, `Path::is_absolute("C:\\foo")` returns `false`. CC writes
/// the OS-native form into JSON, so a Linux/macOS process processing a
/// Windows-sourced session must NOT prepend its own cwd. Use this
/// helper *before* `Path::is_absolute()` whenever the input may have
/// crossed an OS boundary (sessions, history, sanitized slugs).
///
/// Pure string op — no filesystem access.
pub fn is_windows_absolute(path: &str) -> bool {
    // Verbatim prefix wins outright.
    if path.starts_with(r"\\?\") {
        return true;
    }
    // UNC (both separators).
    if path.starts_with(r"\\") || path.starts_with("//") {
        // Reject `\\` alone or `\\?` (incomplete verbatim) — caller
        // already handled `\\?\`. We require host + share, which means
        // at least one more path-separator after the leading double.
        let rest = &path[2..];
        if rest.is_empty() {
            return false;
        }
        // At minimum we need a hostname character; treat any non-empty
        // payload after `\\` or `//` as UNC-shaped. The actual host /
        // share split is path-handler territory; for absolute-ness the
        // double-slash prefix is the contract.
        return true;
    }
    // Drive letter: `X:\...` or `X:/...`. Bare `X:` (no separator) is a
    // drive-relative path on Windows, not absolute — classify as
    // non-absolute so callers don't paste an OS cwd in front.
    let mut chars = path.chars();
    if let (Some(c), Some(b), Some(s)) = (chars.next(), chars.next(), chars.next()) {
        if c.is_ascii_alphabetic() && b == ':' && (s == '\\' || s == '/') {
            return true;
        }
    }
    false
}

/// Combined absolute-path predicate: host's `Path::is_absolute()` OR
/// any Windows-shape signature. Use this in code that processes paths
/// that may have crossed an OS boundary.
pub fn is_absolute_path_str(path: &str) -> bool {
    if is_windows_absolute(path) {
        return true;
    }
    std::path::Path::new(path).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_verbatim_drive_letter_prefix() {
        assert_eq!(
            simplify_windows_path(r"\\?\C:\Users\joker\project"),
            r"C:\Users\joker\project"
        );
    }

    #[test]
    fn strips_verbatim_unc_prefix() {
        assert_eq!(
            simplify_windows_path(r"\\?\UNC\server\share\project"),
            r"\\server\share\project"
        );
    }

    #[test]
    fn leaves_plain_drive_letter_path_unchanged() {
        assert_eq!(
            simplify_windows_path(r"C:\Users\joker\project"),
            r"C:\Users\joker\project"
        );
    }

    #[test]
    fn leaves_plain_unc_path_unchanged() {
        assert_eq!(
            simplify_windows_path(r"\\server\share\project"),
            r"\\server\share\project"
        );
    }

    #[test]
    fn leaves_unix_path_unchanged() {
        assert_eq!(
            simplify_windows_path("/Users/joker/project"),
            "/Users/joker/project"
        );
    }

    #[test]
    fn leaves_empty_string_unchanged() {
        assert_eq!(simplify_windows_path(""), "");
    }

    #[test]
    fn idempotent_on_already_simplified_verbatim() {
        let once = simplify_windows_path(r"\\?\C:\Users\joker");
        let twice = simplify_windows_path(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn verbatim_lowercase_unc_segment_is_not_matched() {
        // Windows verbatim UNC uses uppercase "UNC" — lowercase is not
        // a valid verbatim form, so we leave it alone.
        assert_eq!(
            simplify_windows_path(r"\\?\unc\server\share"),
            r"unc\server\share"
        );
    }

    // -------------------------------------------------------------------
    // is_windows_absolute / is_absolute_path_str — runs on every host OS
    // -------------------------------------------------------------------

    #[test]
    fn is_windows_absolute_drive_letter_backslash() {
        assert!(is_windows_absolute(r"C:\Users\joker"));
        assert!(is_windows_absolute(r"D:\"));
        assert!(is_windows_absolute(r"z:\path"));
    }

    #[test]
    fn is_windows_absolute_drive_letter_forward_slash() {
        // Some tools (msys, IntelliJ) emit `C:/Users/...`.
        assert!(is_windows_absolute("C:/Users/joker"));
    }

    #[test]
    fn is_windows_absolute_unc_backslash() {
        assert!(is_windows_absolute(r"\\server\share\path"));
    }

    #[test]
    fn is_windows_absolute_unc_forward_slash() {
        assert!(is_windows_absolute("//server/share/path"));
    }

    #[test]
    fn is_windows_absolute_verbatim_drive() {
        assert!(is_windows_absolute(r"\\?\C:\Users\joker"));
    }

    #[test]
    fn is_windows_absolute_verbatim_unc() {
        assert!(is_windows_absolute(r"\\?\UNC\server\share"));
    }

    #[test]
    fn is_windows_absolute_rejects_unix_path() {
        assert!(!is_windows_absolute("/Users/joker/project"));
    }

    #[test]
    fn is_windows_absolute_rejects_relative() {
        assert!(!is_windows_absolute("project"));
        assert!(!is_windows_absolute("./project"));
        assert!(!is_windows_absolute("../project"));
    }

    #[test]
    fn is_windows_absolute_rejects_drive_relative() {
        // `C:foo` is drive-relative, NOT absolute.
        assert!(!is_windows_absolute("C:foo"));
        assert!(!is_windows_absolute("C:"));
    }

    #[test]
    fn is_windows_absolute_rejects_empty() {
        assert!(!is_windows_absolute(""));
    }

    #[test]
    fn is_absolute_path_str_accepts_unix_absolute() {
        // Host-native check still covers Unix on every OS.
        // On Windows, `Path::is_absolute("/Users/...")` is false, but
        // `is_windows_absolute` rejects it too, so this test only
        // documents Unix-host behavior.
        #[cfg(unix)]
        assert!(is_absolute_path_str("/Users/joker/project"));
    }

    #[test]
    fn is_absolute_path_str_accepts_windows_shape_on_any_host() {
        // The whole point of the helper: a Windows-shaped string from
        // a foreign session must classify as absolute even on Unix.
        assert!(is_absolute_path_str(r"C:\Users\joker\project"));
        assert!(is_absolute_path_str(r"\\server\share\path"));
        assert!(is_absolute_path_str(r"\\?\C:\Users\joker"));
    }
}

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
}

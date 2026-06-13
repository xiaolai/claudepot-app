//! CC path sanitization — mirrors CC's sessionStoragePortable.ts:311-319.

pub(crate) const MAX_SANITIZED_LENGTH: usize = 200;

/// Replicate CC's `sanitizePath`. Non-alphanumeric ASCII chars become `-`.
/// Paths longer than 200 chars get a djb2 hash suffix.
pub fn sanitize_path(name: &str) -> String {
    // Iterate UTF-16 code units to match JS's `.replace(/[^a-zA-Z0-9]/g, '-')`.
    // JS strings are UTF-16, so surrogate pairs (emoji, etc.) produce 2 hyphens
    // where Rust's char iterator would produce 1.
    let sanitized: String = name
        .encode_utf16()
        .map(|u| {
            let c = u as u8;
            if u < 128 && (c as char).is_ascii_alphanumeric() {
                c as char
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.len() <= MAX_SANITIZED_LENGTH {
        sanitized
    } else {
        let hash = djb2_hash(name);
        format!("{}-{}", &sanitized[..MAX_SANITIZED_LENGTH], hash)
    }
}

/// Best-effort reverse of `sanitize_path`. Lossy by design: every `-` in
/// the slug could have been any non-alphanumeric char in the original.
/// Used for display fallback only — always prefer the authoritative
/// `cwd` field from a session.jsonl when one exists.
///
/// Platform handling:
/// * `<alpha>--...` is an unambiguous Windows drive-letter slug and is
///   always rendered as `X:\...` regardless of host OS (no Unix path
///   can produce this shape).
/// * Leading `--` is resolved as a Windows UNC path (`\\server\share`)
///   on Windows, and kept as a single `/` separator on Unix.
/// * Otherwise, `-` → host separator (`\` on Windows, `/` on Unix).
pub fn unsanitize_path(sanitized: &str) -> String {
    let bytes = sanitized.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b'-' && bytes[2] == b'-' {
        let drive = bytes[0] as char;
        let rest = sanitized[3..].replace('-', "\\");
        return format!("{}:\\{}", drive, rest);
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(rest) = sanitized.strip_prefix("--") {
            return format!(r"\\{}", rest.replace('-', "\\"));
        }
        sanitized.replace('-', "\\")
    }
    #[cfg(not(target_os = "windows"))]
    {
        sanitized.replace('-', "/")
    }
}

/// CC's djb2 hash — matches `djb2Hash()` in CC's `hash.ts`.
///
/// CC uses: seed=0, multiplier=31 (via `(h<<5)-h`), signed 32-bit,
/// then `Math.abs().toString(36)`. Input is UTF-16 code units
/// (JavaScript's `.charCodeAt()`), not UTF-8 bytes.
///
/// This is a 32-bit hash, so collisions are expected at ~65 536 entries
/// (birthday bound). We accept this because CC uses the same algorithm
/// and we must produce identical directory names for compatibility.
pub(crate) fn djb2_hash(s: &str) -> String {
    let mut hash: i32 = 0;
    // Iterate UTF-16 code units to match JavaScript's charCodeAt()
    for code_unit in s.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(code_unit as i32);
    }
    // CC does Math.abs(hash).toString(36)
    let abs = (hash as i64).unsigned_abs() as u32;
    format_radix(abs, 36)
}

pub(crate) fn format_radix(mut x: u32, radix: u32) -> String {
    if x == 0 {
        return "0".to_string();
    }
    let mut result = Vec::new();
    while x > 0 {
        let digit = (x % radix) as u8;
        let ch = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        result.push(ch);
        x /= radix;
    }
    result.reverse();
    String::from_utf8(result).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    //! Co-located golden tests for the four path shapes defined in
    //! `.claude/rules/paths.md`. Integration-style tests (sanitize used
    //! in context) live in `project::tests`; these lock down the pure
    //! string behavior of `sanitize_path` / `unsanitize_path` / `djb2_hash`
    //! against CC's on-disk slugger so any regression is caught here
    //! before touching the rest of the codebase.

    use super::*;

    // --- Group: all four path shapes (sanitize) ----------------------------

    #[test]
    fn sanitize_unix_absolute() {
        assert_eq!(
            sanitize_path("/Users/joker/project"),
            "-Users-joker-project"
        );
    }

    #[test]
    fn sanitize_windows_drive_letter() {
        assert_eq!(
            sanitize_path("C:\\Users\\joker\\project"),
            "C--Users-joker-project"
        );
    }

    #[test]
    fn sanitize_windows_unc() {
        assert_eq!(
            sanitize_path("\\\\server\\share\\project"),
            "--server-share-project"
        );
    }

    #[test]
    fn sanitize_windows_verbatim_not_accepted() {
        // The verbatim `\\?\C:\…` form must be stripped by
        // `path_utils::simplify_windows_path` BEFORE reaching
        // `sanitize_path` — feeding it verbatim here produces a slug
        // that won't match CC's on-disk directory. This test locks
        // that contract: callers must normalize first.
        let raw = sanitize_path("\\\\?\\C:\\Users\\joker");
        let simplified = sanitize_path(&crate::path_utils::simplify_windows_path(
            "\\\\?\\C:\\Users\\joker",
        ));
        assert_ne!(raw, simplified, "verbatim input must be rejected upstream");
        assert_eq!(simplified, "C--Users-joker");
    }

    // --- Group: edge cases -------------------------------------------------

    #[test]
    fn sanitize_empty_input() {
        assert_eq!(sanitize_path(""), "");
    }

    #[test]
    fn sanitize_all_alphanumeric_passthrough() {
        assert_eq!(sanitize_path("abc123XYZ"), "abc123XYZ");
    }

    #[test]
    fn sanitize_all_special_chars_become_hyphens() {
        assert_eq!(sanitize_path("!@#$%^&*()"), "----------");
    }

    #[test]
    fn sanitize_long_path_gets_djb2_suffix() {
        let input = "/Users/joker/".to_string() + &"a".repeat(250);
        let out = sanitize_path(&input);
        // Prefix: MAX_SANITIZED_LENGTH, separator '-', then djb2 hash.
        assert!(out.len() > MAX_SANITIZED_LENGTH);
        assert_eq!(
            out.len(),
            MAX_SANITIZED_LENGTH + 1 + djb2_hash(&input).len()
        );
    }

    #[test]
    fn sanitize_emoji_uses_utf16_code_units() {
        // 🎉 (U+1F389) is a surrogate pair in UTF-16, so it consumes
        // TWO hyphens — matching JS's `.charCodeAt()` iteration in CC.
        assert_eq!(sanitize_path("/tmp/🎉emoji"), "-tmp---emoji");
    }

    // --- Group: unsanitize roundtrip / lossy ------------------------------

    #[test]
    fn unsanitize_unix_absolute() {
        let original = "/Users/joker/project";
        let slug = sanitize_path(original);
        let back = unsanitize_path(&slug);
        // On non-Windows hosts we should get the original back exactly.
        #[cfg(not(target_os = "windows"))]
        assert_eq!(back, original);
        // On Windows, unsanitize reads slugs through the host lens, so
        // a Unix-shaped slug gets rendered with backslashes. The
        // authoritative cwd from session.jsonl is preferred anyway —
        // see recover_cwd_from_sessions.
        #[cfg(target_os = "windows")]
        assert_eq!(back, "\\Users\\joker\\project");
    }

    #[test]
    fn unsanitize_windows_drive_letter_signature() {
        // `<alpha>--...` is unambiguous — always rendered as `X:\...`
        // regardless of host OS.
        assert_eq!(
            unsanitize_path("C--Users-joker-project"),
            "C:\\Users\\joker\\project"
        );
    }

    #[test]
    fn unsanitize_is_lossy() {
        // `my-project` and `my_project` both sanitize to `my-project`,
        // so `unsanitize` can't recover the original punctuation.
        let dashed = sanitize_path("/tmp/my-project");
        let underscored = sanitize_path("/tmp/my_project");
        assert_eq!(dashed, underscored);
    }

    // --- Group: djb2 hash parity with CC ----------------------------------

    #[test]
    fn djb2_deterministic() {
        assert_eq!(djb2_hash("hello"), djb2_hash("hello"));
    }

    #[test]
    fn djb2_zero_seed() {
        assert_eq!(djb2_hash(""), "0");
    }

    #[test]
    fn djb2_cc_parity_long_input() {
        // Locked against CC's reference implementation — if this
        // drifts, slugs for long paths will not match on-disk dirs.
        let input = "/Users/joker/".to_string() + &"a".repeat(250);
        assert_eq!(djb2_hash(&input), "lwkvhu");
    }

    #[test]
    fn djb2_uses_utf16_code_units_not_utf8_bytes() {
        // "/tmp/café" — 'é' is one UTF-16 code unit (U+00E9). If we
        // accidentally iterated UTF-8 bytes we'd hash differently.
        assert_eq!(djb2_hash("/tmp/café"), "udmm60");
    }

    #[test]
    fn format_radix_zero() {
        assert_eq!(format_radix(0, 36), "0");
    }

    #[test]
    fn format_radix_base36_roundtrip_like_js() {
        // JS `(35).toString(36)` == "z"; `(36).toString(36)` == "10".
        assert_eq!(format_radix(35, 36), "z");
        assert_eq!(format_radix(36, 36), "10");
    }
}

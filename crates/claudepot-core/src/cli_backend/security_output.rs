//! The one place that decides what `/usr/bin/security` output may
//! become.
//!
//! # Why this is its own module
//!
//! Both keychain writers hand `security -i` a command line containing
//! the hex-encoded credential. When that line exceeds `security`'s
//! 4 KiB input buffer, `security` does not truncate it — it re-parses
//! the tail as further commands and echoes each unparsable fragment
//! back on stderr:
//!
//! ```text
//! security: unknown command "7b22636c617564654169..."
//! ```
//!
//! Those fragments **are** the credential. Any code that quotes that
//! stderr into an error or a log has written the user's Claude access
//! and refresh tokens, plus any third-party MCP OAuth secrets, into a
//! plaintext file on disk. That is what happened: reported as #45,
//! present in both `storage.rs` and `keychain.rs`, and in the latter
//! it was logged on *every* write rather than only on failure.
//!
//! Reading stderr to decide a category is safe. Reproducing it is not.
//! Keeping the rule in one module with one exported function means a
//! future writer has somewhere obvious to reach for, instead of
//! reaching for `String::from_utf8_lossy` again.

/// `security -i` reads one command per line into a fixed buffer.
/// Measured on macOS 26.5 (`security` from the 15.x tools): a blob of
/// ~2008 bytes writes cleanly, ~2016 bytes fails — consistent with a
/// 4096-byte line limit once hex doubles the payload.
pub(crate) const SECURITY_STDIN_LINE_LIMIT: usize = 4096;

/// Largest blob whose hex form still fits on one `security -i` line,
/// after the `add-generic-password` scaffolding and the account and
/// service names.
pub(crate) fn max_blob_len(account: &str, service: &str) -> usize {
    let scaffolding =
        "add-generic-password -U -a \"\" -s \"\" -X \"\"\n".len() + account.len() + service.len();
    // Every blob byte costs two hex characters.
    SECURITY_STDIN_LINE_LIMIT.saturating_sub(scaffolding) / 2
}

/// Describe a failed `security` invocation **without reproducing its
/// output**.
///
/// The returned string is safe to log and safe to show a user: it
/// carries the exit code and a category, and never a byte of `stderr`.
pub(crate) fn classify_security_failure(exit_code: i32, stderr: &[u8]) -> String {
    let s = String::from_utf8_lossy(stderr);
    let kind = if s.contains("unknown command") {
        // The oversize path. Named explicitly because it is the one
        // failure a user can act on, by shrinking the blob or moving
        // to the file backend.
        "input exceeded the `security -i` line buffer"
    } else if exit_code == 44 || s.contains("could not be found") {
        "item not found"
    } else if s.contains("User interaction is not allowed") {
        "user interaction not allowed (keychain locked, or denied by TCC)"
    } else if s.contains("The authorization was denied") {
        "authorization denied"
    } else {
        "unclassified"
    };
    format!("security failed (exit {exit_code}): {kind}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property that matters: whatever `security` said, none of it
    /// comes back out. Uses a realistic oversize stderr — the shape
    /// that leaked real tokens.
    #[test]
    fn classification_never_reproduces_stderr() {
        let secret_hex = "7b22636c61756465416941757468223a7b226163636573735f746f6b656e22";
        let stderr = format!(
            "security: unknown command \"{secret_hex}\"\n\
             security: unknown command \"{secret_hex}\"\n"
        );
        let out = classify_security_failure(1, stderr.as_bytes());
        assert!(
            !out.contains(secret_hex),
            "classification leaked payload bytes: {out}"
        );
        assert!(
            !out.contains("unknown command \""),
            "leaked the raw line: {out}"
        );
        assert!(
            out.contains("line buffer"),
            "the actionable cause must still be named: {out}"
        );
        assert!(out.contains("exit 1"), "exit code must survive: {out}");
    }

    #[test]
    fn classification_covers_the_conditions_a_user_can_act_on() {
        assert!(classify_security_failure(44, b"").contains("item not found"));
        assert!(classify_security_failure(36, b"could not be found").contains("item not found"));
        assert!(classify_security_failure(
            51,
            b"security: SecKeychainAddGenericPassword: User interaction is not allowed."
        )
        .contains("user interaction not allowed"));
        assert!(
            classify_security_failure(1, b"something new from a future macOS")
                .contains("unclassified")
        );
    }

    #[test]
    fn max_blob_len_leaves_room_for_the_command_scaffolding() {
        let limit = max_blob_len("me@example.com", "Claudepot-cli-credentials");
        // Two hex chars per byte, plus the prefix, must fit the buffer.
        let account = "me@example.com";
        let service = "Claudepot-cli-credentials";
        let line = format!(
            "add-generic-password -U -a \"{account}\" -s \"{service}\" -X \"{}\"\n",
            "a".repeat(limit * 2)
        );
        assert!(
            line.len() <= SECURITY_STDIN_LINE_LIMIT,
            "a blob at the limit must still fit: {} > {}",
            line.len(),
            SECURITY_STDIN_LINE_LIMIT
        );
        // And it should not be uselessly conservative.
        assert!(limit > 1900, "limit unexpectedly small: {limit}");
    }
}

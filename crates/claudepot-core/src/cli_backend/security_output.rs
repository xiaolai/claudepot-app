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
//!
//! # Why oversize writes go through argv
//!
//! `security` offers no size-safe stdin path: `-X` on a `security -i`
//! line hits the buffer above, and `-w` reading stdin silently
//! truncates to 128 bytes and still exits 0 — measured, not assumed.
//! The only transport that carries a large payload is argv.
//!
//! That is a real trade-off, and it is the one **Claude Code already
//! made** for this exact item (`macOsKeychainStorage.ts`): prefer stdin
//! so process monitors see only `security -i`, fall back to argv when
//! the payload cannot fit, because hex in argv "is recoverable by a
//! determined observer but defeats naive plaintext-grep rules, and the
//! alternative — silent credential corruption — is strictly worse."
//!
//! We match that policy deliberately. This crate writes the same
//! Keychain item Claude Code does; a blob CC can store and Claudepot
//! cannot is a broken account switch, and refusing the write does not
//! make the credential safer — it just moves it to a file.

/// `security -i` reads stdin with a 4096-byte `fgets()` buffer (BUFSIZ
/// on darwin). A longer command line is not truncated — the first 4096
/// bytes are consumed as one command with an unterminated quote, and
/// the overflow is re-parsed as further commands.
///
/// The 64-byte headroom and this whole threshold are taken from Claude
/// Code's own `macOsKeychainStorage.ts`, which hit the same wall and
/// documents the same derivation. Matching their constant is not
/// cosmetic: we write the *same Keychain item* they do, so a blob one
/// side considers writable and the other does not is a split brain.
///
/// Independently confirmed here: ~2008 bytes of blob writes, ~2016
/// fails, consistent with 4096 once hex doubles the payload.
pub(crate) const SECURITY_STDIN_LINE_LIMIT: usize = 4096 - 64;

/// Does this fully-formed `security -i` command line fit the buffer?
///
/// The caller builds the exact line it would send, so nothing has to
/// re-derive the scaffolding length and get it subtly wrong.
pub(crate) fn fits_stdin(command_line: &str) -> bool {
    command_line.len() <= SECURITY_STDIN_LINE_LIMIT
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

    /// The transport decision, at the sizes that actually occur.
    /// A ~1 KB blob is an ordinary credential; ~2 KB+ is one with a few
    /// `mcpOAuth` records, which is what #45 was about.
    #[test]
    fn transport_switches_to_argv_exactly_when_stdin_cannot_carry_it() {
        let line = |blob_len: usize| {
            format!(
                "add-generic-password -U -a \"me@example.com\" -s \"Claude Code-credentials\" -X \"{}\"\n",
                "a".repeat(blob_len * 2) // hex doubles it
            )
        };
        assert!(fits_stdin(&line(1024)), "an ordinary credential uses stdin");
        assert!(
            !fits_stdin(&line(4096)),
            "an mcpOAuth-heavy credential must take the argv path rather than \
             being silently mangled by the line buffer"
        );
        // The boundary is the buffer, not a guess.
        assert!(fits_stdin(&"x".repeat(SECURITY_STDIN_LINE_LIMIT)));
        assert!(!fits_stdin(&"x".repeat(SECURITY_STDIN_LINE_LIMIT + 1)));
    }

    /// Parity with Claude Code's own constant. We write the same
    /// Keychain item; disagreeing about what is writable is a split
    /// brain, so this is pinned rather than tuned.
    #[test]
    fn stdin_limit_matches_claude_codes_constant() {
        assert_eq!(SECURITY_STDIN_LINE_LIMIT, 4096 - 64);
    }
}

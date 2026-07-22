//! Detecting a `claude` running inside WSL, from a Windows-native binary.
//!
//! Windows and WSL keep separate process namespaces, so the `sysinfo`
//! scan in [`swap::is_cc_process_running`] cannot see a `claude` started
//! from a WSL shell (reference.md §I, "Process namespace isolation").
//! Left unhandled, a WSL user gets the pre-0.2.10 behaviour: Claudepot
//! rotates the single-use refresh token out from under a live session
//! and Claude Code demands a fresh login.
//!
//! ## Two rules this module will not break
//!
//! **Never boot a distro.** `wsl.exe --exec` against a *stopped* distro
//! starts it. Booting a VM as a side effect of a status probe would be
//! an appalling trade for a background tick, so we ask
//! `--list --running` first and only exec into distros already up. A
//! stopped distro cannot be running `claude` anyway.
//!
//! **Fail open, not closed.** Any failure — `wsl.exe` absent, a hung
//! call, unparseable output — reports "no WSL session". Failing closed
//! would be defensible in isolation (uncertain ⇒ assume live), but here
//! it would permanently block token refresh on every machine where
//! `wsl.exe` misbehaves, which is strictly worse than the exposure that
//! already exists today. The bias toward "assume running" in
//! `is_cc_process_running` applies to *ambiguity about a process we can
//! see*, not to a subsystem we cannot reach at all.
//!
//! The parsers below are compiled on every platform so their tests run
//! on macOS and Linux CI too — the Windows runner is not the only thing
//! standing between this code and a regression. Only the Windows-gated
//! `any_claude_in_wsl` calls them, so off Windows they are legitimately
//! unreferenced; that is the point, not an oversight.
#![cfg_attr(not(windows), allow(dead_code))]

/// Decode `wsl.exe`'s UTF-16LE output into a `String`.
///
/// `wsl.exe` writes its *own* output (as opposed to a Linux command's)
/// as UTF-16LE, which is the classic trap here: read it as UTF-8 and
/// every distro name looks like `U\0b\0u\0n\0t\0u\0`, so nothing ever
/// matches and the probe silently reports "no distros".
///
/// Tolerant by design: strips a BOM, ignores a trailing odd byte from a
/// truncated read, and lossily replaces unpaired surrogates. A mangled
/// name simply fails to match a real distro later.
pub(crate) fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let decoded = String::from_utf16_lossy(&units);
    decoded
        .strip_prefix('\u{feff}')
        .unwrap_or(&decoded)
        .to_string()
}

/// Parse `wsl.exe --list --running --quiet` into distro names.
///
/// `--quiet` gives one bare name per line with no header. Blank lines,
/// stray NULs, and CR are dropped; a distro name may legitimately
/// contain spaces, so lines are trimmed but never split.
pub(crate) fn parse_running_distros(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|line| line.trim_matches(|c: char| c.is_whitespace() || c == '\0'))
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// True when any *already-running* WSL distro has a `claude` process.
///
/// Cost when WSL is not installed: one failed spawn (~1 ms). When it is
/// installed with nothing running: one fast `wsl.exe` call. Only a
/// distro that is actually up costs a second call.
#[cfg(windows)]
pub(crate) async fn any_claude_in_wsl() -> bool {
    use crate::proc_utils::NoWindowExt;
    use std::time::Duration;
    use tokio::process::Command;

    /// Bound every `wsl.exe` call. A wedged WSL service must not hang a
    /// swap or an orchestrator tick.
    const CALL_TIMEOUT: Duration = Duration::from_secs(3);

    let listing = match tokio::time::timeout(
        CALL_TIMEOUT,
        Command::new("wsl.exe")
            .args(["--list", "--running", "--quiet"])
            .no_window()
            .output(),
    )
    .await
    {
        Ok(Ok(out)) if out.status.success() => out.stdout,
        // Absent / errored / timed out — fail open, per the module docs.
        _ => return false,
    };

    for distro in parse_running_distros(&decode_utf16le(&listing)) {
        // `--exec` runs the binary directly with no shell, so the distro
        // name and args need no quoting. A distro without `pgrep`
        // (uncommon; procps is near-universal) exits non-zero and is
        // treated as "not found" — fail open again.
        let probe = tokio::time::timeout(
            CALL_TIMEOUT,
            Command::new("wsl.exe")
                .args(["-d", &distro, "--exec", "pgrep", "-x", "claude"])
                .no_window()
                .output(),
        )
        .await;
        if let Ok(Ok(out)) = probe {
            if out.status.success() && !out.stdout.is_empty() {
                tracing::debug!(distro = %distro, "live claude detected inside WSL");
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode as `wsl.exe` would, so the fixtures can't drift from the
    /// decoder by both being written in the same (wrong) encoding.
    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[test]
    fn decodes_utf16le_output() {
        assert_eq!(decode_utf16le(&utf16le("Ubuntu\r\n")), "Ubuntu\r\n");
    }

    #[test]
    fn strips_the_byte_order_mark() {
        assert_eq!(decode_utf16le(&utf16le("\u{feff}Ubuntu")), "Ubuntu");
    }

    #[test]
    fn tolerates_a_truncated_trailing_byte() {
        let mut bytes = utf16le("Ubuntu");
        bytes.push(0x00); // odd length — a short read
        assert_eq!(decode_utf16le(&bytes), "Ubuntu");
    }

    #[test]
    fn empty_output_decodes_to_empty() {
        assert_eq!(decode_utf16le(&[]), "");
    }

    /// The regression this whole decoder exists for: UTF-16 bytes read
    /// as UTF-8 yield NUL-separated garbage that matches no distro.
    #[test]
    fn utf16_read_as_utf8_would_not_match_but_decoded_does() {
        let raw = utf16le("Ubuntu\r\n");
        let naive = String::from_utf8_lossy(&raw);
        assert!(
            !parse_running_distros(&naive).contains(&"Ubuntu".to_string()),
            "sanity: the naive read must NOT produce a usable name"
        );
        assert_eq!(
            parse_running_distros(&decode_utf16le(&raw)),
            vec!["Ubuntu".to_string()]
        );
    }

    #[test]
    fn parses_multiple_running_distros() {
        assert_eq!(
            parse_running_distros("Ubuntu\r\nDebian\r\n"),
            vec!["Ubuntu".to_string(), "Debian".to_string()]
        );
    }

    #[test]
    fn no_running_distros_yields_empty() {
        for raw in ["", "\r\n", "  \r\n\r\n", "\0\0"] {
            assert!(
                parse_running_distros(raw).is_empty(),
                "expected no distros from {raw:?}"
            );
        }
    }

    /// Distro names may contain spaces — trim the line, never split it.
    #[test]
    fn preserves_spaces_inside_a_distro_name() {
        assert_eq!(
            parse_running_distros("  Ubuntu 22.04 LTS  \r\n"),
            vec!["Ubuntu 22.04 LTS".to_string()]
        );
    }

    #[test]
    fn drops_embedded_nul_padding() {
        assert_eq!(
            parse_running_distros("Ubuntu\0\r\n\0\r\n"),
            vec!["Ubuntu".to_string()]
        );
    }
}

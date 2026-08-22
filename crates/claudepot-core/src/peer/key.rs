//! Locating and validating a session's peer token.
//!
//! CC publishes one key file per bound inbox at
//! `~/.claude/sessions/<pid>.<sha256(socket path)>.key`, mode 0600,
//! containing `{"peerToken": "<32 hex>", "procStart": "<ps lstart>"}`.
//! The socket itself is 0600 and its directory 0700, so possession of
//! the token is not the security boundary — being the same Unix user
//! is. The token exists so a peer cannot address a session it cannot
//! already see.
//!
//! Verified against the 2.1.239 binary.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;

use super::PeerError;

/// CC refuses to read a key file larger than this (`B7b`). A bigger
/// file is not a big key, it is the wrong file.
const MAX_KEY_BYTES: u64 = 4096;

/// CC's own schema for the token: exactly 16 random bytes, hex.
static PEER_TOKEN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[0-9a-f]{32}$").expect("static regex"));

/// Parsed key file. `proc_start` is CC's `ps -o lstart=` snapshot of
/// the owning process, taken when the inbox bound.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KeyFile {
    #[serde(rename = "peerToken")]
    pub peer_token: String,
    /// Unix shape. Windows writes `procStartFt` instead; this module
    /// is Unix-only, so that field is not modelled.
    #[serde(rename = "procStart", default)]
    pub proc_start: Option<String>,
}

/// Derive the key filename CC would have written for `(pid, socket)`.
///
/// Mirrors CC's `${pid}.${sha256(resolve(path))}.key`. We *derive* the
/// name rather than scanning the directory for a matching suffix, which
/// is what CC does when it has only a socket path. Deriving is stricter:
/// a scan has to rank candidates when several pids claim one socket, and
/// ranking means picking a winner where the honest answer is "ambiguous".
/// Here the pid is already known, so there is nothing to rank.
pub fn key_file_name(pid: u32, socket_path: &Path) -> Result<String, PeerError> {
    if !socket_path.is_absolute() {
        return Err(PeerError::RelativeSocketPath {
            path: socket_path.display().to_string(),
        });
    }
    let canonical = socket_path.to_string_lossy();
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(format!("{pid}.{}.key", hex::encode(digest)))
}

/// Read and validate the key file for `(pid, socket)`.
pub fn read_key(sessions_dir: &Path, pid: u32, socket_path: &Path) -> Result<KeyFile, PeerError> {
    let path = sessions_dir.join(key_file_name(pid, socket_path)?);

    let meta = std::fs::metadata(&path).map_err(|source| PeerError::KeyUnreadable {
        pid,
        path: path.display().to_string(),
        source,
    })?;
    if !meta.is_file() || meta.len() > MAX_KEY_BYTES {
        return Err(PeerError::KeyMalformed {
            pid,
            reason: format!("not a regular file under {MAX_KEY_BYTES} bytes"),
        });
    }

    let raw = std::fs::read_to_string(&path).map_err(|source| PeerError::KeyUnreadable {
        pid,
        path: path.display().to_string(),
        source,
    })?;
    let key: KeyFile = serde_json::from_str(&raw).map_err(|e| PeerError::KeyMalformed {
        pid,
        reason: e.to_string(),
    })?;

    if !PEER_TOKEN_RE.is_match(&key.peer_token) {
        // Deliberately does not echo the value: it is a live credential
        // for another process's input queue.
        return Err(PeerError::KeyMalformed {
            pid,
            reason: "peerToken is not 32 lowercase hex characters".into(),
        });
    }
    Ok(key)
}

/// Confirm the process at `pid` is the one that published the key.
///
/// Pids are recycled. `procStart` is CC's own defence: the exact
/// `ps -o lstart=` string captured when the inbox bound. If the running
/// process started at a different instant, the key belongs to a dead
/// session and the pid has been handed to someone else.
///
/// A key with no `procStart` cannot be checked here and is accepted —
/// that mirrors CC, and the `session_id` on every outgoing frame is the
/// backstop that still refuses misdelivery. The two checks sit at
/// different layers on purpose: this one says the *token* is current,
/// `session_id` says the *conversation* is the intended one.
#[cfg(unix)]
pub async fn verify_proc_start(pid: u32, expected: Option<&str>) -> Result<(), PeerError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let observed = read_proc_start(pid).await?;
    if observed.as_deref() == Some(expected) {
        Ok(())
    } else {
        Err(PeerError::PidRecycled {
            pid,
            expected: expected.to_string(),
            observed,
        })
    }
}

/// `ps -o lstart= -p <pid>` under the same locale and timezone CC uses,
/// so the strings are byte-comparable.
#[cfg(unix)]
async fn read_proc_start(pid: u32) -> Result<Option<String>, PeerError> {
    use crate::proc_utils::NoWindowExt;

    let out = tokio::process::Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .no_window()
        .output()
        .await
        .map_err(|source| PeerError::ProcProbeFailed { pid, source })?;

    if !out.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(if text.is_empty() { None } else { Some(text) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Golden: locks our derivation against CC's. Computed as
    /// `sha256("/tmp/cc-socks/11137.sock")`.
    #[test]
    fn key_file_name_matches_cc_derivation() {
        let name = key_file_name(11137, Path::new("/tmp/cc-socks/11137.sock")).unwrap();
        let (pid, rest) = name.split_once('.').unwrap();
        assert_eq!(pid, "11137");
        let digest = rest.strip_suffix(".key").unwrap();
        assert_eq!(digest.len(), 64, "sha256 hex is 64 chars");
        assert!(digest
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
        assert_eq!(
            digest,
            &hex::encode(Sha256::digest(b"/tmp/cc-socks/11137.sock")),
        );
    }

    #[test]
    fn key_file_name_matches_cc_filename_regex() {
        let re = Regex::new(r"^(\d+)\.[0-9a-f]{64}\.key$").unwrap();
        let name = key_file_name(42, Path::new("/tmp/cc-socks/42.sock")).unwrap();
        assert!(re.is_match(&name), "CC would not recognise {name}");
    }

    #[test]
    fn different_sockets_derive_different_names() {
        let a = key_file_name(7, Path::new("/tmp/cc-socks/7.sock")).unwrap();
        let b = key_file_name(7, Path::new("/run/user/501/cc-socks/7.sock")).unwrap();
        assert_ne!(
            a, b,
            "the digest must cover the socket path, not just the pid"
        );
    }

    #[test]
    fn relative_socket_path_is_refused() {
        let err = key_file_name(1, Path::new("cc-socks/1.sock")).unwrap_err();
        assert!(matches!(err, PeerError::RelativeSocketPath { .. }));
    }

    /// Fixture tokens must be obviously synthetic. A real `peerToken`
    /// read off this machine is a live credential for another process's
    /// input queue, and pasting one in here publishes it.
    fn write_key(dir: &Path, pid: u32, socket: &Path, body: &str) -> PathBuf {
        let path = dir.join(key_file_name(pid, socket).unwrap());
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn reads_a_well_formed_key() {
        let dir = tempfile::tempdir().unwrap();
        let socket = Path::new("/tmp/cc-socks/9.sock");
        write_key(
            dir.path(),
            9,
            socket,
            r#"{"peerToken":"feedfacefeedfacefeedfacefeedface","procStart":"Wed Aug 19 15:44:26 2026"}"#,
        );
        let key = read_key(dir.path(), 9, socket).unwrap();
        assert_eq!(key.peer_token, "feedfacefeedfacefeedfacefeedface");
        assert_eq!(key.proc_start.as_deref(), Some("Wed Aug 19 15:44:26 2026"));
    }

    #[test]
    fn key_without_proc_start_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let socket = Path::new("/tmp/cc-socks/9.sock");
        write_key(
            dir.path(),
            9,
            socket,
            r#"{"peerToken":"feedfacefeedfacefeedfacefeedface"}"#,
        );
        assert!(read_key(dir.path(), 9, socket)
            .unwrap()
            .proc_start
            .is_none());
    }

    #[test]
    fn rejects_a_token_that_is_not_32_hex() {
        let dir = tempfile::tempdir().unwrap();
        let socket = Path::new("/tmp/cc-socks/9.sock");
        for bad in ["", "nope", "2430C0CE56539B79A4B4C77716C599A5", "abc123"] {
            write_key(
                dir.path(),
                9,
                socket,
                &format!(r#"{{"peerToken":"{bad}"}}"#),
            );
            let err = read_key(dir.path(), 9, socket).unwrap_err();
            assert!(
                matches!(err, PeerError::KeyMalformed { .. }),
                "token {bad:?} should be refused"
            );
        }
    }

    #[test]
    fn malformed_key_error_never_echoes_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let socket = Path::new("/tmp/cc-socks/9.sock");
        let secret = "0011223344556677889900aabbccddee";
        // Valid hex but the file is truncated JSON, so parsing fails
        // with the raw text in scope.
        write_key(
            dir.path(),
            9,
            socket,
            &format!(r#"{{"peerToken":"{secret}""#),
        );
        let err = read_key(dir.path(), 9, socket).unwrap_err();
        assert!(
            !err.to_string().contains(secret),
            "peer token leaked into an error message: {err}"
        );
    }

    #[test]
    fn missing_key_file_is_unreadable_not_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_key(dir.path(), 9, Path::new("/tmp/cc-socks/9.sock")).unwrap_err();
        assert!(matches!(err, PeerError::KeyUnreadable { .. }));
    }

    #[test]
    fn oversized_key_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let socket = Path::new("/tmp/cc-socks/9.sock");
        write_key(
            dir.path(),
            9,
            socket,
            &"x".repeat(MAX_KEY_BYTES as usize + 1),
        );
        let err = read_key(dir.path(), 9, socket).unwrap_err();
        assert!(matches!(err, PeerError::KeyMalformed { .. }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn absent_proc_start_skips_the_check() {
        assert!(verify_proc_start(std::process::id(), None).await.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn mismatched_proc_start_reports_pid_reuse() {
        let err = verify_proc_start(std::process::id(), Some("Thu Jan  1 00:00:00 1970"))
            .await
            .unwrap_err();
        assert!(matches!(err, PeerError::PidRecycled { .. }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn our_own_proc_start_round_trips() {
        let pid = std::process::id();
        let observed = read_proc_start(pid).await.unwrap();
        // `ps` exists on every supported Unix; if it produced a value,
        // feeding it straight back must verify.
        if let Some(observed) = observed {
            assert!(verify_proc_start(pid, Some(&observed)).await.is_ok());
        }
    }
}

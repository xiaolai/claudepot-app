//! Unified delivery surface for session exports.
//!
//! Both the CLI (`claudepot session export …`) and the GUI (Tauri
//! `session_share_gist_start`) need the same set of side-effects after
//! a transcript has been rendered to a string:
//!   * write to a file with user-only permissions, or
//!   * push to a gist via the existing [`session_share`] uploader, or
//!   * copy to the system clipboard.
//!
//! Before this module each surface owned its own copy of the destination
//! logic — file ACL hardening, GitHub PAT lookup, filename derivation —
//! and the two copies had drifted (CLI hashed the session id; GUI used
//! the raw id; CLI shelled out to `icacls`; GUI never hardened).
//!
//! The shape here is deliberately small:
//!   * [`ExportDestination`] is the discriminated request.
//!   * [`DeliveryReceipt`] is the discriminated response.
//!   * [`DeliverError`] flattens every failure mode the caller cares about.
//!   * [`ClipboardWriter`] is an injected dependency — the CLI provides
//!     a subprocess implementation, the GUI passes `None`.
//!   * [`deliver`] is the only public action.
//!
//! Filename convention is unified on `session-<short_hash>.<ext>` — the
//! short hash is the first 16 hex chars of `sha256(session_id)`, which
//! is stable across runs and platforms. (We follow the existing
//! `config_view::discover::blake3_id` precedent: a sha256 prefix named
//! after the conceptual "blake3" — no real blake3 dep is pulled in.)

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use thiserror::Error;

use crate::cli_backend::secret_file;
use crate::project_progress::ProgressSink;
use crate::session_export::ExportFormat;
use crate::session_share::{share_gist, GistResult, ShareError};

const GH_TOKEN_SERVICE: &str = "claudepot";
const GH_TOKEN_ENTRY: &str = "github-token";

/// Where the rendered body should land. One arm per supported sink.
#[derive(Debug, Clone)]
pub enum ExportDestination {
    /// Atomic write to `path`, then harden ACL/perms to user-only.
    File { path: PathBuf },
    /// Hand the body to the injected [`ClipboardWriter`].
    Clipboard,
    /// Upload to GitHub Gist via [`session_share::share_gist`]. Token
    /// is resolved internally (env first, keychain fallback).
    Gist {
        filename: String,
        description: String,
        public: bool,
    },
}

/// Discriminated success payload — caller renders a different message
/// per destination, so the shape is per-arm.
#[derive(Debug, Clone)]
pub enum DeliveryReceipt {
    File { path: PathBuf, bytes: usize },
    Clipboard { bytes: usize },
    Gist { result: GistResult, bytes: usize },
}

/// Every failure mode `deliver` can surface to the caller. The CLI
/// converts these into anyhow errors; the Tauri command converts them
/// into Strings.
#[derive(Debug, Error)]
pub enum DeliverError {
    #[error("file write failed: {0}")]
    File(#[from] std::io::Error),
    #[error("clipboard unavailable: {0}")]
    Clipboard(String),
    #[error("gist upload failed: {0}")]
    Gist(#[from] ShareError),
    #[error("no GitHub token; set GITHUB_TOKEN or store one in keychain")]
    NoToken,
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("permission hardening failed: {0}")]
    Harden(String),
}

/// Injection point for clipboard writing. Implementations live outside
/// `claudepot-core` so the core crate stays free of subprocess concerns
/// and the GUI doesn't drag in subprocess machinery it doesn't need.
#[async_trait]
pub trait ClipboardWriter: Send + Sync {
    /// Write `body` to the clipboard. Errors are returned as a string
    /// so callers don't need to know the implementation's error type.
    async fn write_text(&self, body: &str) -> Result<(), String>;
}

/// Resolve a GitHub PAT for gist uploads. Env-var (`GITHUB_TOKEN`)
/// wins; the Claudepot-managed keychain entry
/// `("claudepot", "github-token")` is the fallback. Both the CLI and
/// the GUI read from the same two sources; this is the one
/// implementation.
pub fn github_token_resolve() -> Result<String, DeliverError> {
    if let Ok(v) = std::env::var("GITHUB_TOKEN") {
        if !v.trim().is_empty() {
            return Ok(v);
        }
    }
    let entry = keyring::Entry::new(GH_TOKEN_SERVICE, GH_TOKEN_ENTRY)
        .map_err(|e| DeliverError::Keychain(e.to_string()))?;
    match entry.get_password() {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Err(DeliverError::NoToken),
    }
}

/// Default filename for a session export: `session-<short_hash>.<ext>`.
///
/// `short_hash` is the first 16 hex chars of `sha256(session_id)`,
/// matching the `blake3_id` helper in `config_view/discover.rs`. This
/// shape is what the CLI used pre-unification (CLI used a 8-char FNV
/// hash, the design intentionally widens it to 16 hex for less
/// collision risk and parity with the rest of the codebase).
pub fn default_export_filename(session_id: &str, ext: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(session_id.as_bytes());
    let digest = h.finalize();
    let short = &hex::encode(digest)[..16];
    format!("session-{short}.{ext}")
}

/// File extension matching the requested export format.
pub fn extension_for(format: &ExportFormat) -> &'static str {
    match format {
        ExportFormat::Markdown | ExportFormat::MarkdownSlim => "md",
        ExportFormat::Html { .. } => "html",
        ExportFormat::Json => "json",
    }
}

/// Deliver `body` to `dest`. Routes to file / clipboard / gist arms.
///
/// `clipboard` may be `None` when the destination isn't `Clipboard`;
/// it's only required for that arm. `progress` is forwarded to the gist
/// uploader so callers see `preparing → uploading → complete` events
/// — for file/clipboard arms the sink is unused (delivery is fast and
/// indivisible).
pub async fn deliver(
    body: &str,
    dest: ExportDestination,
    clipboard: Option<&dyn ClipboardWriter>,
    progress: &dyn ProgressSink,
) -> Result<DeliveryReceipt, DeliverError> {
    match dest {
        ExportDestination::File { path } => deliver_file(body, &path).await,
        ExportDestination::Clipboard => deliver_clipboard(body, clipboard).await,
        ExportDestination::Gist {
            filename,
            description,
            public,
        } => deliver_gist(body, &filename, &description, public, progress).await,
    }
}

/// Atomic file write + permission harden.
///
/// Steps:
///   1. Resolve the parent directory; create it if missing.
///   2. Stage the body in a temp file co-located in the parent (to
///      keep `persist` a `rename(2)` — same-volume only, important on
///      Windows where cross-volume falls back to copy-then-delete).
///   3. `persist` (i.e. atomic rename) over the destination path.
///   4. `secret_file::harden_path` — the canonical chmod-0o600 / DACL
///      narrowing surface. Idempotent. Re-runs even if `persist`
///      replaced an existing widened file.
async fn deliver_file(body: &str, path: &Path) -> Result<DeliveryReceipt, DeliverError> {
    let bytes_len = body.len();
    let body = body.to_string();
    let path_buf = path.to_path_buf();
    // The whole file dance is sync; lift it onto a blocking pool so we
    // don't park the runtime worker on a fsync.
    let receipt = tokio::task::spawn_blocking(move || -> Result<DeliveryReceipt, DeliverError> {
        let parent = path_buf
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        if !parent.exists() {
            std::fs::create_dir_all(&parent)?;
        }
        let mut tmp = tempfile::NamedTempFile::new_in(&parent)?;
        {
            use std::io::Write;
            tmp.write_all(body.as_bytes())?;
            tmp.as_file_mut().sync_all()?;
        }
        // `persist` does an atomic rename on the same filesystem.
        // Failure here returns a `PersistError`, which carries both
        // the io::Error and the original NamedTempFile — we keep the
        // io::Error for the caller and let the temp file drop (auto-
        // delete).
        tmp.persist(&path_buf)
            .map_err(|e| DeliverError::File(e.error))?;
        secret_file::harden_path(&path_buf)
            .map_err(|e| DeliverError::Harden(e.to_string()))?;
        Ok(DeliveryReceipt::File {
            path: path_buf,
            bytes: bytes_len,
        })
    })
    .await
    .map_err(|e| DeliverError::Clipboard(format!("blocking task join failed: {e}")))??;
    Ok(receipt)
}

async fn deliver_clipboard(
    body: &str,
    writer: Option<&dyn ClipboardWriter>,
) -> Result<DeliveryReceipt, DeliverError> {
    let writer = writer.ok_or_else(|| {
        DeliverError::Clipboard("no clipboard writer supplied".to_string())
    })?;
    writer
        .write_text(body)
        .await
        .map_err(DeliverError::Clipboard)?;
    Ok(DeliveryReceipt::Clipboard { bytes: body.len() })
}

async fn deliver_gist(
    body: &str,
    filename: &str,
    description: &str,
    public: bool,
    progress: &dyn ProgressSink,
) -> Result<DeliveryReceipt, DeliverError> {
    let token = github_token_resolve()?;
    let result = share_gist(body, filename, description, public, &token, progress).await?;
    Ok(DeliveryReceipt::Gist {
        result,
        bytes: body.len(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_progress::NoopSink;
    use std::sync::Mutex;

    /// Recording clipboard writer — captures the last body it saw.
    #[derive(Default)]
    struct FakeClipboard {
        last: Mutex<Option<String>>,
    }

    #[async_trait]
    impl ClipboardWriter for FakeClipboard {
        async fn write_text(&self, body: &str) -> Result<(), String> {
            *self.last.lock().unwrap() = Some(body.to_string());
            Ok(())
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn deliver_file_writes_and_hardens() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.md");
        let receipt = deliver(
            "hello",
            ExportDestination::File { path: path.clone() },
            None,
            &NoopSink,
        )
        .await
        .unwrap();
        match receipt {
            DeliveryReceipt::File { path: p, bytes } => {
                assert_eq!(p, path);
                assert_eq!(bytes, 5);
            }
            other => panic!("unexpected receipt: {other:?}"),
        }
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "hello");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0600, got {mode:o}");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn deliver_file_overwrites_existing_widened_perms() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("export.md");
        // Pre-create with relaxed perms.
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let widened = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(widened, 0o644, "test setup: expected 0644 pre-state");

        deliver(
            "new",
            ExportDestination::File { path: path.clone() },
            None,
            &NoopSink,
        )
        .await
        .unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "new");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected 0600 after deliver, got {mode:o}");
    }

    #[tokio::test]
    async fn deliver_clipboard_calls_writer() {
        let fake = FakeClipboard::default();
        let receipt = deliver(
            "clip body",
            ExportDestination::Clipboard,
            Some(&fake),
            &NoopSink,
        )
        .await
        .unwrap();
        match receipt {
            DeliveryReceipt::Clipboard { bytes } => assert_eq!(bytes, "clip body".len()),
            other => panic!("unexpected receipt: {other:?}"),
        }
        assert_eq!(
            fake.last.lock().unwrap().as_deref(),
            Some("clip body"),
            "writer should have recorded the body"
        );
    }

    #[tokio::test]
    async fn deliver_clipboard_no_writer_errors() {
        let err = deliver("body", ExportDestination::Clipboard, None, &NoopSink)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DeliverError::Clipboard(_)),
            "expected Clipboard error, got {err:?}"
        );
    }

    #[test]
    fn default_export_filename_is_stable() {
        // Golden test — locks the hashing scheme so future
        // refactors don't silently change the filename users see.
        // sha256("00000000-0000-0000-0000-000000000000")[..16]
        // computed once and frozen here.
        let got = default_export_filename("00000000-0000-0000-0000-000000000000", "md");
        assert_eq!(got, "session-12b9377cbe7e5c94.md");
    }

    #[test]
    fn extension_for_covers_all_formats() {
        assert_eq!(extension_for(&ExportFormat::Markdown), "md");
        assert_eq!(extension_for(&ExportFormat::MarkdownSlim), "md");
        assert_eq!(extension_for(&ExportFormat::Json), "json");
        assert_eq!(extension_for(&ExportFormat::Html { no_js: false }), "html");
        assert_eq!(extension_for(&ExportFormat::Html { no_js: true }), "html");
    }

    #[test]
    fn github_token_resolve_uses_env_var_when_set() {
        // Use a unique token value so we don't conflict with the user's
        // actual env. Set it, call, unset.
        let unique = "test-token-xyz123";
        // Intentionally overwrite for this test — saved & restored.
        let prior = std::env::var("GITHUB_TOKEN").ok();
        std::env::set_var("GITHUB_TOKEN", unique);
        let got = github_token_resolve().unwrap();
        assert_eq!(got, unique);
        match prior {
            Some(v) => std::env::set_var("GITHUB_TOKEN", v),
            None => std::env::remove_var("GITHUB_TOKEN"),
        }
    }
}

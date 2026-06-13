//! Cross-platform helpers for files holding credential blobs.
//!
//! Centralizes:
//! * Atomic write with `0600` perms on Unix.
//! * On Windows, set a user-only DACL (current user — full control;
//!   no other ACEs) on the destination after persist.
//! * Read-time permission/ACL verification with auto-repair.
//!
//! Every credential file in `cli_backend/` (the per-account private slot
//! `<data_dir>/credentials/<uuid>.json` and CC's own
//! `.credentials.json`) MUST go through `harden_path` after write and
//! `verify_path` before read. The functions are no-ops on platforms
//! where the OS already gates access by user identity (none today —
//! Unix needs explicit chmod, Windows inherits parent ACLs which can
//! be wide-open for files dropped under `%LocalAppData%`).

use crate::error::SwapError;
use std::path::Path;

/// Apply user-only access to `path`. On Unix this is `chmod 0o600`. On
/// Windows this replaces the file's DACL with one ACE granting the
/// current process token's user FILE_ALL_ACCESS, removing inheritance.
///
/// Idempotent — safe to call repeatedly. Returns Ok if `path` doesn't
/// exist (a brand-new file may not have been persisted yet).
///
/// The platform mechanics live in [`crate::secure_perms`] so the
/// secret SQLite stores (`keys.db`, `env-vault.db`) share the exact
/// same implementation.
pub fn harden_path(path: &Path) -> Result<(), SwapError> {
    crate::secure_perms::harden_user_only(path).map_err(SwapError::FileError)
}

/// Verify the file at `path` is locked down to the current user. If it
/// isn't, log a warning and re-harden. Returns the original `Ok(())`
/// either way unless re-hardening fails outright.
///
/// Callers should invoke this BEFORE reading credential content so a
/// previously misconfigured file (e.g. an admin tool widened perms) is
/// brought back into spec before any token bytes leave the disk.
pub fn verify_path(path: &Path) -> Result<(), SwapError> {
    if !path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 {
                tracing::warn!(
                    "credential file {} has permissions {:o} (expected 600), fixing",
                    path.display(),
                    mode
                );
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                    .map_err(SwapError::FileError)?;
            }
        }
    }
    #[cfg(windows)]
    {
        // On Windows we can't cheaply assert "DACL has exactly one ACE"
        // — the call to GetNamedSecurityInfo is expensive and the
        // structure is hard to compare directly. Re-apply the user-only
        // DACL unconditionally; this is idempotent and cheap relative
        // to the file read that follows.
        crate::secure_perms::harden_user_only(path).map_err(SwapError::FileError)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harden_nonexistent_path_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist");
        // Must not error on missing files.
        harden_path(&path).unwrap();
        verify_path(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn test_harden_sets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        std::fs::write(&path, b"x").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        harden_path(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn test_verify_repairs_widened_perms() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        std::fs::write(&path, b"x").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        // Widen to simulate external tampering.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        verify_path(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

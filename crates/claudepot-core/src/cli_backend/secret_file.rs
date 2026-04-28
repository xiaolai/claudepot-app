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
pub fn harden_path(path: &Path) -> Result<(), SwapError> {
    if !path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(SwapError::FileError)?;
    }
    #[cfg(windows)]
    {
        windows_impl::set_user_only_dacl(path)?;
    }
    Ok(())
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
        windows_impl::set_user_only_dacl(path)?;
    }
    Ok(())
}

#[cfg(windows)]
mod windows_impl {
    use super::SwapError;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        SetNamedSecurityInfoW, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER,
    };
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, SET_ACCESS, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::Authorization::SetEntriesInAclW;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION,
        NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// `FILE_ALL_ACCESS` from Win32 — STANDARD_RIGHTS_REQUIRED |
    /// SYNCHRONIZE | 0x1FF. Hardcoded so we don't pull the heavy
    /// `Win32_Storage_FileSystem` feature surface for one constant.
    const FILE_ALL_ACCESS: u32 = 0x001F_01FF;

    /// Replace `path`'s DACL with a single ACE granting the current
    /// user FILE_ALL_ACCESS, with inheritance disabled and no other
    /// ACEs (PROTECTED_DACL_SECURITY_INFORMATION blocks parent
    /// inheritance from re-adding access).
    pub fn set_user_only_dacl(path: &Path) -> Result<(), SwapError> {
        unsafe {
            // 1. Open the current process token to read the user SID.
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(SwapError::WriteFailed(
                    "OpenProcessToken failed".to_string(),
                ));
            }
            // RAII-ish: ensure CloseHandle on early returns.
            struct TokenGuard(HANDLE);
            impl Drop for TokenGuard {
                fn drop(&mut self) {
                    unsafe {
                        windows_sys::Win32::Foundation::CloseHandle(self.0);
                    }
                }
            }
            let _guard = TokenGuard(token);

            // 2. Read TOKEN_USER (variable-length: SID lives past the struct).
            let mut needed: u32 = 0;
            GetTokenInformation(
                token,
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut needed,
            );
            if needed == 0 {
                return Err(SwapError::WriteFailed(
                    "GetTokenInformation size probe failed".to_string(),
                ));
            }
            let mut buf = vec![0u8; needed as usize];
            if GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr() as *mut _,
                needed,
                &mut needed,
            ) == 0
            {
                return Err(SwapError::WriteFailed(
                    "GetTokenInformation read failed".to_string(),
                ));
            }
            let token_user_ptr = buf.as_ptr() as *const TOKEN_USER;
            let user_sid = (*token_user_ptr).User.Sid;

            // 3. Build a single EXPLICIT_ACCESS_W → ACL.
            let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
            ea.grfAccessPermissions = FILE_ALL_ACCESS;
            ea.grfAccessMode = SET_ACCESS;
            ea.grfInheritance = NO_INHERITANCE;
            let trustee = TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: user_sid as *mut _,
            };
            ea.Trustee = trustee;

            let mut new_acl: *mut ACL = std::ptr::null_mut();
            let rc = SetEntriesInAclW(1, &ea, std::ptr::null_mut(), &mut new_acl);
            if rc != ERROR_SUCCESS {
                return Err(SwapError::WriteFailed(format!(
                    "SetEntriesInAclW failed: {rc}"
                )));
            }

            // 4. Apply the protected DACL to the file by name.
            let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
            wide.push(0);
            let info_flags =
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
            let rc = SetNamedSecurityInfoW(
                wide.as_ptr() as *mut _,
                SE_FILE_OBJECT,
                info_flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_acl,
                std::ptr::null_mut(),
            );
            // SetEntriesInAclW allocated `new_acl` via LocalAlloc.
            LocalFree(new_acl as _);

            if rc != ERROR_SUCCESS {
                return Err(SwapError::WriteFailed(format!(
                    "SetNamedSecurityInfoW failed: {rc}"
                )));
            }
        }
        Ok(())
    }
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

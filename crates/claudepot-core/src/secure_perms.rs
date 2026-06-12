//! User-only filesystem permissions for secret-bearing files.
//!
//! One home for the two platform mechanisms `rust-conventions.md`
//! requires before writing any file containing credentials:
//!
//! * Unix — `chmod 0600`.
//! * Windows — replace the file's DACL with a single ACE granting the
//!   current user `FILE_ALL_ACCESS` (files dropped under
//!   `%LocalAppData%` inherit parent ACLs that can be wide-open).
//!
//! Callers:
//! * `cli_backend::secret_file` delegates here for credential blobs.
//! * The secret SQLite stores (`keys.db`, `env-vault.db`) call
//!   [`harden_user_only`] for the db/-wal/-shm trio after open, and
//!   [`precreate_user_only`] *before* `Connection::open` to close the
//!   create→chmod window (`Connection::open` creates files at the
//!   process umask, typically 0644 — another local user can open the
//!   file in that window and keep the fd).

use std::io;
use std::path::Path;

/// Apply user-only access to `path`. On Unix this is `chmod 0o600`. On
/// Windows this replaces the file's DACL with one ACE granting the
/// current process token's user `FILE_ALL_ACCESS`, with inheritance
/// disabled.
///
/// Idempotent — safe to call repeatedly. Returns `Ok` if `path`
/// doesn't exist (a sidecar like `-wal` may not have been created yet).
pub fn harden_user_only(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    {
        windows_impl::set_user_only_dacl(path)?;
    }
    Ok(())
}

/// Atomically pre-create `path` with user-only permissions BEFORE a
/// writer (rusqlite's `Connection::open`, notably) creates it at
/// umask-default permissions. Mirrors the M9 fix in
/// `session_index::SessionIndex::open`.
///
/// Best-effort by design: if the file already exists we leave its
/// perms alone here (the caller's post-init [`harden_user_only`] is
/// the backstop), and a `create_new` race with another process is
/// likewise absorbed by the post-init harden. Real failures (missing
/// parent dir, EACCES) surface at the writer's own open call, which
/// reports them with better context.
pub fn precreate_user_only(path: &Path) {
    if path.exists() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path);
    }
    #[cfg(windows)]
    {
        // No umask on Windows — the new file inherits the parent
        // DACL at create time. Create it empty, then immediately
        // narrow the DACL, still ahead of any secret bytes landing.
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .is_ok()
        {
            let _ = windows_impl::set_user_only_dacl(path);
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::SetEntriesInAclW;
    use windows_sys::Win32::Security::Authorization::{
        SetNamedSecurityInfoW, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER,
    };
    use windows_sys::Win32::Security::Authorization::{EXPLICIT_ACCESS_W, SET_ACCESS, TRUSTEE_W};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
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
    pub fn set_user_only_dacl(path: &Path) -> io::Result<()> {
        unsafe {
            // 1. Open the current process token to read the user SID.
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::other("OpenProcessToken failed"));
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
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
            if needed == 0 {
                return Err(io::Error::other("GetTokenInformation size probe failed"));
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
                return Err(io::Error::other("GetTokenInformation read failed"));
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
                return Err(io::Error::other(format!("SetEntriesInAclW failed: {rc}")));
            }

            // 4. Apply the protected DACL to the file by name.
            let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
            wide.push(0);
            let info_flags = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
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
                return Err(io::Error::other(format!(
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
    fn test_secure_perms_harden_nonexistent_path_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist");
        harden_user_only(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn test_secure_perms_harden_sets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        std::fs::write(&path, b"x").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        harden_user_only(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn test_secure_perms_precreate_creates_file_at_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.db");
        precreate_user_only(&path);
        // The file must exist at 0600 from birth — that is the whole
        // point (no create→chmod window).
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn test_secure_perms_precreate_leaves_existing_file_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.db");
        std::fs::write(&path, b"data").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        precreate_user_only(&path);
        // Contents and perms untouched — the post-init harden is the
        // layer that tightens an existing file.
        assert_eq!(std::fs::read(&path).unwrap(), b"data");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }
}

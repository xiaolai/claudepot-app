//! Locating and vetting the TLS material for the remote surface.
//!
//! Claudepot terminates TLS itself (the tailnet's control server cannot
//! issue certs, so `tailscale serve` is not an option here), which means
//! a private key sits in the data directory. This module decides whether
//! that key is fit to use *before* a socket exists.
//!
//! ## Fail closed, loudly
//!
//! Missing or unreadable material is not a reason to fall back to plain
//! HTTP. A server that silently downgrades is worse than one that does
//! not start: the user believes traffic is protected, the phone shows no
//! warning it would not also show for a self-signed cert, and nothing
//! ever surfaces the difference. Every failure here stops the server.
//!
//! ## Why the permission check is not paranoia
//!
//! The key authenticates this machine to every paired device. A
//! world-readable key in `~/.claudepot` is readable by any process
//! running as any user on the machine, which on a shared or
//! multi-account Mac is a real boundary. `secure_perms` already owns
//! this rule for credential files; this reuses the same standard rather
//! than inventing a second one.

use std::path::{Path, PathBuf};

/// Certificate chain, PEM. Public — no permission requirement.
pub const CERT_FILENAME: &str = "remote-cert.pem";
/// Private key, PEM. Must be owner-only.
pub const KEY_FILENAME: &str = "remote-key.pem";

/// A key file larger than this is not a key.
const MAX_PEM_BYTES: u64 = 128 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TlsMaterialError {
    #[error(
        "no TLS certificate at {path} — the remote surface will not start \
         without one. Mint a CA and a leaf certificate first (see \
         `scripts/mint-remote-cert.sh`)."
    )]
    CertMissing { path: String },

    #[error(
        "no TLS private key at {path} — the remote surface will not start \
         without one. Mint a CA and a leaf certificate first (see \
         `scripts/mint-remote-cert.sh`)."
    )]
    KeyMissing { path: String },

    #[error("cannot read {path}: {source}")]
    Unreadable {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{path} does not look like PEM (no `-----BEGIN` block). The remote \
         surface refuses to start rather than present material it cannot vet."
    )]
    NotPem { path: String },

    #[error(
        "the TLS private key at {path} is mode {mode:o}; it must be readable \
         only by its owner (0600). It authenticates this machine to every \
         paired device."
    )]
    KeyTooOpen { path: String, mode: u32 },

    #[error("{path} is {len} bytes, which is not a certificate or a key")]
    ImplausibleSize { path: String, len: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsMaterial {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

pub fn cert_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(CERT_FILENAME)
}

pub fn key_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(KEY_FILENAME)
}

pub fn load() -> Result<TlsMaterial, TlsMaterialError> {
    load_from(&cert_path(), &key_path())
}

pub fn load_from(cert: &Path, key: &Path) -> Result<TlsMaterial, TlsMaterialError> {
    if !cert.exists() {
        return Err(TlsMaterialError::CertMissing {
            path: cert.display().to_string(),
        });
    }
    if !key.exists() {
        return Err(TlsMaterialError::KeyMissing {
            path: key.display().to_string(),
        });
    }

    let cert_pem = read_pem(cert)?;
    check_key_permissions(key)?;
    let key_pem = read_pem(key)?;

    Ok(TlsMaterial {
        cert_pem,
        key_pem,
        cert_path: cert.to_path_buf(),
        key_path: key.to_path_buf(),
    })
}

fn read_pem(path: &Path) -> Result<Vec<u8>, TlsMaterialError> {
    let meta = std::fs::metadata(path).map_err(|source| TlsMaterialError::Unreadable {
        path: path.display().to_string(),
        source,
    })?;
    if meta.len() == 0 || meta.len() > MAX_PEM_BYTES {
        return Err(TlsMaterialError::ImplausibleSize {
            path: path.display().to_string(),
            len: meta.len(),
        });
    }
    let bytes = std::fs::read(path).map_err(|source| TlsMaterialError::Unreadable {
        path: path.display().to_string(),
        source,
    })?;
    // Structural sniff only. Real parsing belongs to rustls at the
    // point of use; the value here is refusing an obviously wrong file
    // (a DER blob, a stray note) with a message that names the file.
    if !bytes.windows(11).any(|w| w == b"-----BEGIN ") {
        return Err(TlsMaterialError::NotPem {
            path: path.display().to_string(),
        });
    }
    Ok(bytes)
}

#[cfg(unix)]
fn check_key_permissions(path: &Path) -> Result<(), TlsMaterialError> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|source| TlsMaterialError::Unreadable {
        path: path.display().to_string(),
        source,
    })?;
    let mode = meta.permissions().mode() & 0o777;
    // Any group or world bit is disqualifying. Not just "not 0600" —
    // 0640 and 0604 are both readable by someone else.
    if mode & 0o077 != 0 {
        return Err(TlsMaterialError::KeyTooOpen {
            path: path.display().to_string(),
            mode,
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_key_permissions(_path: &Path) -> Result<(), TlsMaterialError> {
    // Windows ACLs are not a mode bitmask; `secure_perms` owns that
    // story for credential files and this surface is Unix-only for now.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const CERT: &str = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
    const KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----\n";

    fn write(dir: &TempDir, name: &str, body: &str, mode: u32) -> PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let _ = mode;
        p
    }

    #[test]
    fn well_formed_material_loads() {
        let d = TempDir::new().unwrap();
        let c = write(&d, CERT_FILENAME, CERT, 0o644);
        let k = write(&d, KEY_FILENAME, KEY, 0o600);
        let m = load_from(&c, &k).unwrap();
        assert!(m.cert_pem.starts_with(b"-----BEGIN"));
        assert!(m.key_pem.starts_with(b"-----BEGIN"));
    }

    #[test]
    fn a_missing_cert_is_named_distinctly_from_a_missing_key() {
        let d = TempDir::new().unwrap();
        let k = write(&d, KEY_FILENAME, KEY, 0o600);
        let missing = d.path().join(CERT_FILENAME);
        assert!(matches!(
            load_from(&missing, &k).unwrap_err(),
            TlsMaterialError::CertMissing { .. }
        ));

        let c = write(&d, CERT_FILENAME, CERT, 0o644);
        assert!(matches!(
            load_from(&c, &d.path().join("nope.pem")).unwrap_err(),
            TlsMaterialError::KeyMissing { .. }
        ));
    }

    #[test]
    fn the_error_tells_the_user_how_to_fix_it() {
        let d = TempDir::new().unwrap();
        let err = load_from(&d.path().join("a.pem"), &d.path().join("b.pem")).unwrap_err();
        assert!(
            err.to_string().contains("mint-remote-cert"),
            "a fail-closed error must name the remedy: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_group_or_world_readable_key_is_refused() {
        let d = TempDir::new().unwrap();
        let c = write(&d, CERT_FILENAME, CERT, 0o644);
        for mode in [0o644, 0o640, 0o604, 0o660, 0o666] {
            let k = write(&d, KEY_FILENAME, KEY, mode);
            assert!(
                matches!(
                    load_from(&c, &k).unwrap_err(),
                    TlsMaterialError::KeyTooOpen { .. }
                ),
                "mode {mode:o} leaves the key readable by someone else"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_modes_are_accepted() {
        let d = TempDir::new().unwrap();
        let c = write(&d, CERT_FILENAME, CERT, 0o644);
        for mode in [0o600, 0o400] {
            let k = write(&d, KEY_FILENAME, KEY, mode);
            assert!(load_from(&c, &k).is_ok(), "mode {mode:o} is owner-only");
        }
    }

    #[test]
    fn a_non_pem_file_is_refused() {
        let d = TempDir::new().unwrap();
        let c = write(&d, CERT_FILENAME, "not a certificate at all", 0o644);
        let k = write(&d, KEY_FILENAME, KEY, 0o600);
        assert!(matches!(
            load_from(&c, &k).unwrap_err(),
            TlsMaterialError::NotPem { .. }
        ));
    }

    #[test]
    fn an_empty_file_is_refused_before_the_pem_sniff() {
        let d = TempDir::new().unwrap();
        let c = write(&d, CERT_FILENAME, "", 0o644);
        let k = write(&d, KEY_FILENAME, KEY, 0o600);
        assert!(matches!(
            load_from(&c, &k).unwrap_err(),
            TlsMaterialError::ImplausibleSize { len: 0, .. }
        ));
    }

    #[test]
    fn the_key_permission_check_runs_before_the_key_is_read() {
        // Ordering matters: reading first would pull a world-readable
        // key into memory and only then complain about it.
        let d = TempDir::new().unwrap();
        let c = write(&d, CERT_FILENAME, CERT, 0o644);
        let k = write(&d, KEY_FILENAME, "not pem either", 0o644);
        let err = load_from(&c, &k).unwrap_err();
        #[cfg(unix)]
        assert!(
            matches!(err, TlsMaterialError::KeyTooOpen { .. }),
            "permissions must be judged before contents: {err}"
        );
        #[cfg(not(unix))]
        assert!(matches!(err, TlsMaterialError::NotPem { .. }));
    }
}

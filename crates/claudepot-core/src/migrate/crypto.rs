//! Bundle encryption + signing — deferred stub for v0.
//!
//! See `dev-docs/project-migrate-spec.md` §3.3.
//!
//! Real implementation will use:
//!   - `age` for passphrase-based symmetric encryption (default-on).
//!   - `minisign` for an optional signature over `manifest.json`'s sha256
//!     (required for `--unattended-import`).
//!
//! For v0 we ship plaintext bundles. The CLI flags `--encrypt` and
//! `--sign KEYFILE` return `MigrateError::NotImplemented` so callers
//! see a deliberate refusal instead of a silent downgrade. `--no-encrypt`
//! is a no-op pass-through that succeeds.

use crate::migrate::error::MigrateError;

/// Plaintext mode is the only mode supported in v0. Callers that
/// request encryption get a hard error so they can plan migrations
/// accordingly (or carry the bundle over an already-encrypted channel
/// like SSH or Syncthing+full-disk-encryption).
pub fn require_plaintext_only(encrypt: bool) -> Result<(), MigrateError> {
    if encrypt {
        return Err(MigrateError::NotImplemented(
            "bundle encryption (`age`) — v0 ships plaintext bundles only. \
             Carry the bundle over an encrypted channel (SSH / Syncthing) \
             or use --no-encrypt explicitly."
                .to_string(),
        ));
    }
    Ok(())
}

/// Same shape as `require_plaintext_only`: refusing instead of
/// pretending. When `keyfile` is `Some`, the bundle would have been
/// signed; we error so the user's automation pipeline halts loudly.
pub fn require_unsigned(keyfile: Option<&str>) -> Result<(), MigrateError> {
    if keyfile.is_some() {
        return Err(MigrateError::NotImplemented(
            "bundle signing (`minisign`) — v0 ships unsigned bundles only.".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_encrypt_is_ok() {
        assert!(require_plaintext_only(false).is_ok());
    }

    #[test]
    fn encrypt_returns_not_implemented() {
        assert!(matches!(
            require_plaintext_only(true),
            Err(MigrateError::NotImplemented(_))
        ));
    }

    #[test]
    fn no_signing_is_ok() {
        assert!(require_unsigned(None).is_ok());
    }

    #[test]
    fn signing_returns_not_implemented() {
        assert!(matches!(
            require_unsigned(Some("/path/to/key")),
            Err(MigrateError::NotImplemented(_))
        ));
    }
}

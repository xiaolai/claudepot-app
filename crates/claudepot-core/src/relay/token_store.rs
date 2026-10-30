//! Where a client keeps its relay device tokens: the OS credential
//! store, one item per paired server, keyed by server id.
//!
//! macOS uses the `/usr/bin/security` verified-write pattern, because the
//! GUI and the CLI are differently signed and both must read the token
//! (`rules/architecture.md`, surface 2): write via `security -i` with the
//! value hex-encoded over stdin, never argv, then read it back — a zero
//! exit from `security -i` is not proof the item landed. Linux and
//! Windows use `keyring`, which is per-user there.
//!
//! On macOS the pattern itself is [`crate::security_cli`], shared with
//! route secrets; this module adds the async boundary and is
//! parameterized by service so the integration test can use its own slot.

use super::secret::Secret;

/// The production service name. One item per server; the account
/// attribute is the server id.
pub const SERVICE: &str = "claudepot-relay-device";

#[derive(Debug, thiserror::Error)]
pub enum TokenStoreError {
    #[error("the credential store is locked — unlock it and retry")]
    Locked,
    #[error("credential store: {0}")]
    Backend(String),
    #[error("the credential store write did not take effect")]
    NotVerified,
    #[error("unusable credential store key")]
    BadKey,
}

// macOS checks keys inside `security_cli`; the rule is the shared one.
#[cfg(any(not(target_os = "macos"), test))]
use super::secret::store_key_ok as key_ok;

#[cfg(target_os = "macos")]
impl From<crate::security_cli::SecurityCliError> for TokenStoreError {
    fn from(e: crate::security_cli::SecurityCliError) -> Self {
        use crate::security_cli::SecurityCliError as E;
        match e {
            E::Locked => Self::Locked,
            E::Backend(m) => Self::Backend(m),
            E::NotVerified => Self::NotVerified,
            E::BadKey => Self::BadKey,
            E::TooLong => Self::Backend("the value is too long for the keychain".into()),
        }
    }
}

/// Runs one blocking `security` call off the async runtime.
#[cfg(target_os = "macos")]
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, crate::security_cli::SecurityCliError> + Send + 'static,
) -> Result<T, TokenStoreError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| TokenStoreError::Backend(format!("keychain task: {e}")))?
        .map_err(Into::into)
}

#[cfg(target_os = "macos")]
pub async fn read(service: &str, server_id: &str) -> Result<Option<Secret>, TokenStoreError> {
    let (service, server_id) = (service.to_owned(), server_id.to_owned());
    blocking(move || crate::security_cli::read(&service, &server_id)).await
}

#[cfg(target_os = "macos")]
pub async fn write(service: &str, server_id: &str, token: &Secret) -> Result<(), TokenStoreError> {
    let (service, server_id, token) = (service.to_owned(), server_id.to_owned(), token.clone());
    blocking(move || crate::security_cli::write(&service, &server_id, &token)).await
}

#[cfg(target_os = "macos")]
pub async fn delete(service: &str, server_id: &str) -> Result<(), TokenStoreError> {
    let (service, server_id) = (service.to_owned(), server_id.to_owned());
    blocking(move || crate::security_cli::delete(&service, &server_id)).await
}

#[cfg(not(target_os = "macos"))]
fn entry(service: &str, server_id: &str) -> Result<keyring::Entry, TokenStoreError> {
    if !key_ok(service) || !key_ok(server_id) {
        return Err(TokenStoreError::BadKey);
    }
    keyring::Entry::new(service, server_id).map_err(|e| TokenStoreError::Backend(e.to_string()))
}

#[cfg(not(target_os = "macos"))]
pub async fn read(service: &str, server_id: &str) -> Result<Option<Secret>, TokenStoreError> {
    match entry(service, server_id)?.get_password() {
        Ok(v) if !v.is_empty() => Ok(Some(Secret::new(v))),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(TokenStoreError::Backend(e.to_string())),
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn write(service: &str, server_id: &str, token: &Secret) -> Result<(), TokenStoreError> {
    let e = entry(service, server_id)?;
    e.set_password(token.expose())
        .map_err(|e| TokenStoreError::Backend(e.to_string()))?;
    // Wrapped before comparing, so the read-back is zeroized and the
    // comparison is constant-time.
    match e.get_password().map(Secret::new) {
        Ok(back) if back == *token => Ok(()),
        _ => Err(TokenStoreError::NotVerified),
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn delete(service: &str, server_id: &str) -> Result<(), TokenStoreError> {
    match entry(service, server_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(TokenStoreError::Backend(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_that_could_break_the_security_command_line_are_refused() {
        assert!(key_ok("2f0c8a6e-1d0b-4f5a-9b1e-7c3d2a1b0c9d"));
        assert!(key_ok(SERVICE));
        for bad in ["", "a\"b", "a b", "a\nb", "a\\b", "a\rb"] {
            assert!(!key_ok(bad), "{bad:?} must be refused");
        }
    }
}

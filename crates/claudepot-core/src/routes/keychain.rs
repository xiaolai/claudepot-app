//! OS-keychain storage for route secrets — opt-in alternative to the
//! plaintext-in-JSON default.
//!
//! Service name: `claudepot-routes`. Account: `<route-uuid>-<field>`,
//! e.g. `<uuid>-api_key`, `<uuid>-bearer_token`,
//! `<uuid>-foundry_api_key`. Each field is its own keychain entry so
//! one route's bearer-token and api-key don't collide.
//!
//! This is **distinct** from the `Claude Code-credentials` keychain
//! item that CC owns. We never read or write that one from here —
//! per `.claude/rules/architecture.md`.
//!
//! On macOS the cell is written through `/usr/bin/security`
//! ([`crate::security_cli`], surface 2): the wrapper's helper script reads
//! it with `security find-generic-password`, and the GUI and the CLI are
//! differently signed, so an item `keyring` created would be ACL-locked to
//! whichever binary wrote it. Linux and Windows use `keyring`.
//!
//! Until 2026-10 this module used `keyring` on every platform, and the
//! workspace enables no `keyring` backend feature, so `keyring` fell back
//! to its in-memory mock store: every write "succeeded" and was gone by
//! the next process. A route saved with "store in keychain" before then
//! has no secret anywhere and must be re-entered. The Linux/Windows arm is
//! still on the mock until a backend feature is chosen.

use super::error::RouteError;
use super::types::RouteId;

/// The keychain service every route secret lives under. The wrapper's
/// helper script and the relay's serving path read the same name.
pub const SERVICE: &str = "claudepot-routes";

/// Secret-field selectors within a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretField {
    GatewayApiKey,
    BedrockBearerToken,
    FoundryApiKey,
}

impl SecretField {
    fn suffix(self) -> &'static str {
        match self {
            SecretField::GatewayApiKey => "api_key",
            SecretField::BedrockBearerToken => "bearer_token",
            SecretField::FoundryApiKey => "foundry_api_key",
        }
    }
}

/// The account attribute for one (route, field) cell.
pub fn account(route_id: RouteId, field: SecretField) -> String {
    format!("{}-{}", route_id, field.suffix())
}

#[cfg(target_os = "macos")]
fn security_to_route_error(e: crate::security_cli::SecurityCliError) -> RouteError {
    RouteError::Io(std::io::Error::other(format!("keychain: {e}")))
}

/// Write or replace the secret for one (route, field) cell. Verified: on
/// macOS the value is read back before this returns `Ok`.
#[cfg(target_os = "macos")]
pub fn store_secret(route_id: RouteId, field: SecretField, secret: &str) -> Result<(), RouteError> {
    let secret = crate::relay::secret::Secret::new(secret.to_owned());
    crate::security_cli::write(SERVICE, &account(route_id, field), &secret)
        .map_err(security_to_route_error)
}

/// Read the secret. `Ok(None)` when no entry exists; `Err` for I/O
/// or platform-permission failures.
#[cfg(target_os = "macos")]
pub fn read_secret(route_id: RouteId, field: SecretField) -> Result<Option<String>, RouteError> {
    crate::security_cli::read(SERVICE, &account(route_id, field))
        .map(|v| v.map(|s| s.expose().to_owned()))
        .map_err(security_to_route_error)
}

/// Delete the entry. Idempotent — missing-entry errors are swallowed.
#[cfg(target_os = "macos")]
pub fn delete_secret(route_id: RouteId, field: SecretField) -> Result<(), RouteError> {
    crate::security_cli::delete(SERVICE, &account(route_id, field)).map_err(security_to_route_error)
}

#[cfg(not(target_os = "macos"))]
fn entry(route_id: RouteId, field: SecretField) -> Result<keyring::Entry, RouteError> {
    let acct = account(route_id, field);
    keyring::Entry::new(SERVICE, &acct).map_err(keyring_to_route_error)
}

#[cfg(not(target_os = "macos"))]
fn keyring_to_route_error(e: keyring::Error) -> RouteError {
    RouteError::Io(std::io::Error::other(format!("keychain: {e}")))
}

/// Write or replace the secret for one (route, field) cell.
#[cfg(not(target_os = "macos"))]
pub fn store_secret(route_id: RouteId, field: SecretField, secret: &str) -> Result<(), RouteError> {
    let e = entry(route_id, field)?;
    e.set_password(secret).map_err(keyring_to_route_error)?;
    Ok(())
}

/// Read the secret. `Ok(None)` when no entry exists; `Err` for I/O
/// or platform-permission failures.
#[cfg(not(target_os = "macos"))]
pub fn read_secret(route_id: RouteId, field: SecretField) -> Result<Option<String>, RouteError> {
    let e = entry(route_id, field)?;
    match e.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(other) => Err(keyring_to_route_error(other)),
    }
}

/// Delete the entry. Idempotent — missing-entry errors are swallowed.
#[cfg(not(target_os = "macos"))]
pub fn delete_secret(route_id: RouteId, field: SecretField) -> Result<(), RouteError> {
    let e = entry(route_id, field)?;
    match e.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(other) => Err(keyring_to_route_error(other)),
    }
}

/// Tear down every secret we've stored for a route. Call when the
/// route is deleted or the user opts out of keychain storage.
pub fn delete_all_for_route(route_id: RouteId) -> Result<(), RouteError> {
    for field in [
        SecretField::GatewayApiKey,
        SecretField::BedrockBearerToken,
        SecretField::FoundryApiKey,
    ] {
        delete_secret(route_id, field)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_format() {
        let id = uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        assert_eq!(
            account(id, SecretField::GatewayApiKey),
            "11111111-1111-1111-1111-111111111111-api_key"
        );
        assert_eq!(
            account(id, SecretField::BedrockBearerToken),
            "11111111-1111-1111-1111-111111111111-bearer_token"
        );
        assert_eq!(
            account(id, SecretField::FoundryApiKey),
            "11111111-1111-1111-1111-111111111111-foundry_api_key"
        );
    }
}

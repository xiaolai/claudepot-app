//! OS keychain wrapper for Claudepot's own key secrets.
//!
//! Uses the `keyring` crate (same code-signing identity reads and
//! writes). Services are namespaced `com.claudepot.keys.api` and
//! `com.claudepot.keys.oauth`; the keychain "username" is the key's
//! UUID string, so each row has a unique keychain item.
//!
//! NEVER use these helpers for CC's shared `Claude Code-credentials`
//! item — that slot is owned by `cli_backend::keychain` and only ever
//! touched via `/usr/bin/security`. Mixing backends corrupts CC state.

use super::error::KeyError;
use uuid::Uuid;

const SERVICE_API: &str = "com.claudepot.keys.api";
const SERVICE_OAUTH: &str = "com.claudepot.keys.oauth";

fn entry(service: &str, uuid: Uuid) -> Result<keyring::Entry, KeyError> {
    keyring::Entry::new(service, &uuid.to_string())
        .map_err(|e| KeyError::Keychain(format!("open {service}: {e}")))
}

pub fn write_api_secret(uuid: Uuid, token: &str) -> Result<(), KeyError> {
    entry(SERVICE_API, uuid)?
        .set_password(token)
        .map_err(|e| KeyError::Keychain(format!("write api: {e}")))
}

pub fn read_api_secret(uuid: Uuid) -> Result<String, KeyError> {
    entry(SERVICE_API, uuid)?
        .get_password()
        .map_err(|e| KeyError::Keychain(format!("read api: {e}")))
}

pub fn delete_api_secret(uuid: Uuid) -> Result<(), KeyError> {
    match entry(SERVICE_API, uuid)?.delete_credential() {
        Ok(()) => Ok(()),
        // Idempotent: missing item is a valid post-condition for remove.
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(KeyError::Keychain(format!("delete api: {e}"))),
    }
}

pub fn write_oauth_secret(uuid: Uuid, token: &str) -> Result<(), KeyError> {
    entry(SERVICE_OAUTH, uuid)?
        .set_password(token)
        .map_err(|e| KeyError::Keychain(format!("write oauth: {e}")))
}

pub fn read_oauth_secret(uuid: Uuid) -> Result<String, KeyError> {
    entry(SERVICE_OAUTH, uuid)?
        .get_password()
        .map_err(|e| KeyError::Keychain(format!("read oauth: {e}")))
}

pub fn delete_oauth_secret(uuid: Uuid) -> Result<(), KeyError> {
    match entry(SERVICE_OAUTH, uuid)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(KeyError::Keychain(format!("delete oauth: {e}"))),
    }
}

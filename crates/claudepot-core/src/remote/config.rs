//! Server settings and the persisted auth state.
//!
//! Kept in `remote-config.json`, separate from `remote-devices.json`,
//! because the two have very different write rates: the throttle counter
//! here changes on every failed login, while the device list changes
//! when someone pairs or revokes. Sharing one file would rewrite the
//! revocation list on every wrong password.
//!
//! **Off by default.** A remote surface that turns itself on because the
//! app was installed is not a feature. `enabled` starts false and no
//! socket is opened until a human sets a password and switches it on.

use std::net::{IpAddr, Ipv4Addr};

use serde::{Deserialize, Serialize};

use super::bind::{self, BindAddr, BindRefusal};
use super::login::AuthConfig;
use super::totp::{TotpSecret, TotpState};

pub const SCHEMA_VERSION: u32 = 1;
pub const CONFIG_FILENAME: &str = "remote-config.json";

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// Chosen from IANA's dynamic range, deliberately not 8080/3000/5000 —
/// those collide with whatever else the user is running.
pub const DEFAULT_PORT: u16 = 8420;

fn default_port() -> u16 {
    DEFAULT_PORT
}

/// Loopback by default: the safe address, and the one that needs no
/// certificate. Turning the appliance on for the LAN is a second,
/// explicit decision.
fn default_bind() -> IpAddr {
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_bind")]
    pub bind: IpAddr,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Extra `Host` names to answer to, beyond the shapes always
    /// accepted (IP literals, single labels, `.local`, `.internal`).
    /// For the one case the shape rule cannot cover: a user fronting
    /// the appliance with a real public domain — which is exactly what
    /// a DNS-rebinding attack needs, so it has to be opt-in.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_bind(),
            port: DEFAULT_PORT,
            allowed_hosts: Vec::new(),
        }
    }
}

impl ServerConfig {
    /// Validate the bind address. The only way to get a [`BindAddr`].
    pub fn checked_bind(&self) -> Result<BindAddr, BindRefusal> {
        bind::check(self.bind)
    }
}

/// On-disk shape. `AuthConfig` is not serialized directly because
/// `TotpSecret` deliberately has no `Serialize` — a secret that
/// serializes by default ends up in a log line or a DTO by accident.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfigFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub server: ServerConfig,
    /// PHC string. `None` = not set up.
    #[serde(default)]
    pub password_hash: Option<String>,
    #[serde(default)]
    pub totp_secret_base32: Option<String>,
    /// High-water mark for spent TOTP counters. Persisted, or a restart
    /// would reopen the replay window a used code was burned to close.
    #[serde(default)]
    pub totp_last_counter: Option<u64>,
    /// Persisted, or a restart resets the throttle and an attacker just
    /// has to make the process crash to get unlimited guesses.
    #[serde(default)]
    pub failed_attempts: u32,
    /// Registered passkeys. **Public keys only** — that is the whole
    /// reason a passkey beats both of the credentials above: reading
    /// this file gives an attacker a cracking job for the password hash,
    /// working access for a TOTP secret, and nothing at all for these.
    ///
    /// They live here rather than on a `Device` in `remote-devices.json`
    /// because a passkey is an *account* credential, like the password.
    /// Attaching one to a session record would delete it when that
    /// session expired, and revoking a lost phone would silently destroy
    /// the way back in from every other one.
    #[serde(default)]
    pub passkeys: Vec<super::passkey::PasskeyRecord>,
    /// Stable WebAuthn user handle for this appliance.
    ///
    /// WebAuthn requires a user id, and this appliance has exactly one
    /// user — so the value carries no information beyond "this machine".
    /// It has to be *stable*, though: a fresh handle per registration
    /// makes the platform file each passkey under a separate account, so
    /// a phone that re-registers ends up showing two Claudepot entries
    /// rather than replacing the first. Minted on first use.
    #[serde(default)]
    pub passkey_user_handle: Option<uuid::Uuid>,
}

/// Hand-written because `#[derive(Default)]` would produce
/// `schema_version: 0`. Serde's `default = "..."` applies only when
/// *deserializing* a missing field, so a derived default is a file that
/// fails its own validation and cannot be saved — which is exactly what
/// two tests caught.
impl Default for RemoteConfigFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            server: ServerConfig::default(),
            password_hash: None,
            totp_secret_base32: None,
            totp_last_counter: None,
            failed_attempts: 0,
            passkeys: Vec::new(),
            passkey_user_handle: None,
        }
    }
}

impl RemoteConfigFile {
    /// The WebAuthn user handle, minting one the first time it is asked
    /// for. Returns whether it had to mint, so the caller knows a save
    /// is required — a handle that lives only in memory would be a new
    /// handle after every restart, which is the failure this field
    /// exists to prevent.
    pub fn passkey_user_handle_or_mint(&mut self) -> (uuid::Uuid, bool) {
        match self.passkey_user_handle {
            Some(id) => (id, false),
            None => {
                let id = uuid::Uuid::new_v4();
                self.passkey_user_handle = Some(id);
                (id, true)
            }
        }
    }

    pub fn auth(&self) -> AuthConfig {
        AuthConfig {
            password_hash: self.password_hash.clone(),
            totp_secret: self
                .totp_secret_base32
                .as_deref()
                .and_then(TotpSecret::from_base32),
            totp_state: TotpState {
                last_used_counter: self.totp_last_counter,
            },
            throttle: super::password::Throttle {
                consecutive_failures: self.failed_attempts,
            },
        }
    }

    /// Fold a mutated `AuthConfig` back in. Callers must do this after
    /// **every** attempt, success or failure — see `login::attempt`.
    pub fn absorb(&mut self, auth: &AuthConfig) {
        self.password_hash = auth.password_hash.clone();
        self.totp_secret_base32 = auth.totp_secret.as_ref().map(|s| s.to_base32());
        self.totp_last_counter = auth.totp_state.last_used_counter;
        self.failed_attempts = auth.throttle.consecutive_failures;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn the_default_is_off_and_loopback() {
        let c = ServerConfig::default();
        assert!(!c.enabled, "a remote surface must not switch itself on");
        assert!(c.bind.is_loopback());
        assert!(!c.checked_bind().unwrap().requires_tls());
    }

    #[test]
    fn an_empty_file_is_a_safe_default() {
        let f: RemoteConfigFile = serde_json::from_str("{}").unwrap();
        assert!(!f.server.enabled);
        assert!(f.password_hash.is_none());
        assert_eq!(f.server.port, DEFAULT_PORT);
    }

    #[test]
    fn a_lan_bind_is_accepted_and_requires_tls() {
        let c = ServerConfig {
            enabled: true,
            bind: IpAddr::from_str("192.168.1.42").unwrap(),
            port: DEFAULT_PORT,
            allowed_hosts: Vec::new(),
        };
        assert!(c.checked_bind().unwrap().requires_tls());
    }

    #[test]
    fn a_public_bind_is_refused_at_the_config_layer() {
        let c = ServerConfig {
            enabled: true,
            bind: IpAddr::from_str("8.8.8.8").unwrap(),
            port: DEFAULT_PORT,
            allowed_hosts: Vec::new(),
        };
        assert!(c.checked_bind().is_err());
    }

    #[test]
    fn auth_state_round_trips_through_the_file() {
        let secret = TotpSecret::generate();
        let mut f = RemoteConfigFile {
            password_hash: Some("$scrypt$x".into()),
            totp_secret_base32: Some(secret.to_base32()),
            totp_last_counter: Some(42),
            failed_attempts: 3,
            ..Default::default()
        };
        let auth = f.auth();
        assert_eq!(auth.totp_state.last_used_counter, Some(42));
        assert_eq!(auth.throttle.consecutive_failures, 3);
        assert_eq!(
            auth.totp_secret.as_ref().unwrap().as_bytes(),
            secret.as_bytes()
        );

        let mut mutated = auth;
        mutated.throttle.consecutive_failures = 9;
        mutated.totp_state.last_used_counter = Some(99);
        f.absorb(&mutated);
        assert_eq!(f.failed_attempts, 9);
        assert_eq!(f.totp_last_counter, Some(99));
    }

    #[test]
    fn the_throttle_and_the_spent_counter_survive_a_restart() {
        // Both are persisted for the same reason: if either resets when
        // the process does, an attacker only has to make it restart to
        // get unlimited guesses, or to reopen a burned code's window.
        let mut f = RemoteConfigFile {
            failed_attempts: 7,
            totp_last_counter: Some(1234),
            ..Default::default()
        };
        let json = serde_json::to_string(&f).unwrap();
        f = serde_json::from_str(&json).unwrap();
        assert_eq!(f.failed_attempts, 7);
        assert_eq!(f.totp_last_counter, Some(1234));
    }

    #[test]
    fn the_totp_secret_is_stored_as_base32_not_as_a_debug_blob() {
        let secret = TotpSecret::generate();
        let f = RemoteConfigFile {
            totp_secret_base32: Some(secret.to_base32()),
            ..Default::default()
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains(&secret.to_base32()));
        assert!(!json.contains("TotpSecret"));
    }
}

// ── Persistence ─────────────────────────────────────────────────────
//
// Fails loud on corruption, like the other two stores in this feature,
// but for a third reason: this file holds the throttle counter and the
// spent-TOTP high-water mark. Recovering it silently to empty resets
// both — which hands an attacker unlimited guesses and reopens the
// replay window of every code that was burned to close it.

use crate::json_store::{self, SaveError};
pub use crate::json_store::{CorruptionRecovery, Loaded};
use std::path::{Path, PathBuf};

const STORE: &str = "remote_config_store";

pub fn config_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(CONFIG_FILENAME)
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigValidationError {
    #[error("remote config is schema version {found}, expected {expected}")]
    UnknownSchema { expected: u32, found: u32 },
    #[error("port 0 asks the OS to pick, which a client could never find again")]
    ZeroPort,
    #[error("the configured bind address is refused: {0}")]
    BadBind(String),
}

impl RemoteConfigFile {
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ConfigValidationError::UnknownSchema {
                expected: SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if self.server.port == 0 {
            return Err(ConfigValidationError::ZeroPort);
        }
        // Validated on the way to disk as well as at bind time, so a
        // hand-edited public address is refused before anything listens.
        self.server
            .checked_bind()
            .map_err(|e| ConfigValidationError::BadBind(e.to_string()))?;
        Ok(())
    }
}

impl json_store::Validate for RemoteConfigFile {
    type Error = ConfigValidationError;
    fn validate(&self) -> Result<(), ConfigValidationError> {
        RemoteConfigFile::validate(self)
    }
}

pub fn load_at(path: &Path) -> std::io::Result<Loaded<RemoteConfigFile>> {
    let loaded = json_store::load_or_recover::<RemoteConfigFile>(path, STORE)?;
    if loaded.recovery.is_some() {
        tracing::error!(
            store = STORE,
            path = %path.display(),
            "remote config was unreadable and has been reset; the login \
             throttle and the spent-TOTP counter are back to zero, and the \
             remote surface is OFF until reconfigured"
        );
    }
    Ok(loaded)
}

pub fn load() -> std::io::Result<Loaded<RemoteConfigFile>> {
    load_at(&config_path())
}

pub fn save_at(
    path: &Path,
    file: &RemoteConfigFile,
) -> Result<(), SaveError<ConfigValidationError>> {
    json_store::save(path, file)
}

pub fn save(file: &RemoteConfigFile) -> Result<(), SaveError<ConfigValidationError>> {
    save_at(&config_path(), file)
}

#[cfg(test)]
mod persist_tests {
    use super::*;

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILENAME);
        let f = RemoteConfigFile {
            failed_attempts: 2,
            ..Default::default()
        };
        save_at(&path, &f).unwrap();
        assert_eq!(load_at(&path).unwrap().value.failed_attempts, 2);
    }

    #[test]
    fn a_corrupt_file_reports_its_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILENAME);
        std::fs::write(&path, "{ not json").unwrap();
        let loaded = load_at(&path).unwrap();
        assert!(
            loaded.recovery.is_some(),
            "a silent reset would zero the throttle and reopen every burned \
             TOTP counter"
        );
        assert!(!loaded.value.server.enabled, "and it must come back OFF");
    }

    #[test]
    fn a_public_bind_cannot_be_written_to_disk() {
        use std::net::IpAddr;
        use std::str::FromStr;
        let dir = tempfile::tempdir().unwrap();
        let f = RemoteConfigFile {
            server: ServerConfig {
                enabled: true,
                bind: IpAddr::from_str("8.8.8.8").unwrap(),
                port: DEFAULT_PORT,
                allowed_hosts: Vec::new(),
            },
            ..Default::default()
        };
        assert!(save_at(&dir.path().join(CONFIG_FILENAME), &f).is_err());
    }

    #[test]
    fn port_zero_is_refused() {
        let f = RemoteConfigFile {
            server: ServerConfig {
                port: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(f.validate(), Err(ConfigValidationError::ZeroPort)));
    }
}

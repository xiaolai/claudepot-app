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

/// Serde needs a fn for a non-`false` bool default. See
/// `RemoteConfigFile::approvals_enabled` for why this one is `true`.
fn default_true() -> bool {
    true
}

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
    /// When the most recent failure happened. Persisted *with* the
    /// counter and for the same reason — but it is also what makes the
    /// backoff expire at all. A counter with no timestamp cannot say how
    /// much of the wait has been served, so it can only ever hold the
    /// caller forever; see `password::Throttle`. Absent on files written
    /// before this field existed, which yields no delay rather than a
    /// permanent one.
    #[serde(default)]
    pub last_failed_at: Option<chrono::DateTime<chrono::Utc>>,
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
    /// May a phone answer a permission prompt?
    ///
    /// The one capability on this surface that *grants* something rather
    /// than reading or messaging — see `remote::approval`. Everything
    /// else a stolen bearer token can reach is a transcript to read or
    /// text Claude Code will refuse to treat as approval; with this on,
    /// that token can approve a tool call, which is arbitrary code
    /// execution as this user.
    ///
    /// Separable from `server.enabled` because the two are different
    /// questions and were previously one: the hook was installed by the
    /// mere act of starting the server, so wanting the panel — read a
    /// transcript, send a prompt from the sofa — meant taking the
    /// approval capability with it whether or not it was wanted.
    ///
    /// **Defaults to `true`**, unlike `server.enabled`. That asymmetry
    /// is deliberate: `enabled` defaults off because a surface that
    /// switches itself on at install is not a feature, whereas this
    /// field is only ever consulted on a machine whose owner has already
    /// turned the surface on and set a password. Defaulting it off would
    /// silently remove a working capability from every existing install
    /// on upgrade, which is a worse surprise than the status quo it
    /// preserves. Turn it off to narrow what a stolen token can do.
    #[serde(default = "default_true")]
    pub approvals_enabled: bool,
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
            last_failed_at: None,
            passkeys: Vec::new(),
            passkey_user_handle: None,
            approvals_enabled: true,
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
                last_failure_at: self.last_failed_at,
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
        self.last_failed_at = auth.throttle.last_failure_at;
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
        //
        // The *timestamp* is part of that: without it a restart leaves
        // failures with no recorded time, which yields no delay — the
        // same unlimited-guesses hole, reached by crashing the process
        // rather than by waiting.
        let stamp = chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let mut f = RemoteConfigFile {
            failed_attempts: 7,
            last_failed_at: Some(stamp),
            totp_last_counter: Some(1234),
            ..Default::default()
        };
        let json = serde_json::to_string(&f).unwrap();
        f = serde_json::from_str(&json).unwrap();
        assert_eq!(f.failed_attempts, 7);
        assert_eq!(f.last_failed_at, Some(stamp));
        assert_eq!(f.totp_last_counter, Some(1234));
    }

    #[test]
    fn a_totp_secret_that_cannot_be_parsed_is_refused_not_ignored() {
        // The bug this guards: `auth()` resolves the secret with
        // `and_then(from_base32)`, so an unparseable value becomes
        // `None`, which is exactly what "TOTP was never configured"
        // looks like. Login then passes on the password alone and
        // `absorb()` persists the `None`, destroying the evidence.
        // Two factors become one, silently and permanently.
        for bad in ["", "!!!!not base32!!!!", "AAAA", &"A".repeat(64)] {
            let f = RemoteConfigFile {
                totp_secret_base32: Some(bad.to_string()),
                ..Default::default()
            };
            assert!(
                matches!(f.validate(), Err(ConfigValidationError::BadTotpSecret)),
                "{bad:?} must be refused"
            );
        }
        let good = TotpSecret::generate().to_base32();
        let f = RemoteConfigFile {
            totp_secret_base32: Some(good),
            ..Default::default()
        };
        assert!(f.validate().is_ok());
        assert!(f.auth().totp_enabled());
    }

    #[test]
    fn duplicate_or_empty_passkey_ids_are_refused() {
        let mk = |id: &str| super::super::passkey::PasskeyRecord {
            credential: super::super::webauthn::Credential {
                id: id.to_string(),
                public_key: "cHVibGlj".to_string(),
                sign_count: 0,
            },
            label: "phone".into(),
            created_at: chrono::Utc::now(),
            last_used: None,
            origin: "https://appliance.internal:8420".into(),
        };
        let dup = RemoteConfigFile {
            passkeys: vec![mk("same"), mk("same")],
            ..Default::default()
        };
        assert!(matches!(
            dup.validate(),
            Err(ConfigValidationError::DuplicatePasskeyId(_))
        ));
        let empty = RemoteConfigFile {
            passkeys: vec![mk("")],
            ..Default::default()
        };
        assert!(matches!(
            empty.validate(),
            Err(ConfigValidationError::EmptyPasskeyId)
        ));
        let ok = RemoteConfigFile {
            passkeys: vec![mk("a"), mk("b")],
            ..Default::default()
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn the_failure_stamp_round_trips_through_auth_and_back() {
        // `auth()` and `absorb()` are the only path between the file and
        // the throttle. A field added to one and not the other loses the
        // deadline on every login attempt, which is invisible: the
        // counter still rises, so the file still looks like it is
        // throttling.
        let stamp = chrono::DateTime::from_timestamp(1_700_000_500, 0).unwrap();
        let f = RemoteConfigFile {
            failed_attempts: 9,
            last_failed_at: Some(stamp),
            ..Default::default()
        };
        let auth = f.auth();
        assert_eq!(auth.throttle.last_failure_at, Some(stamp));

        let mut back = RemoteConfigFile::default();
        back.absorb(&auth);
        assert_eq!(back.last_failed_at, Some(stamp));
        assert_eq!(back.failed_attempts, 9);
    }

    #[test]
    fn a_config_written_before_the_stamp_existed_still_loads() {
        // Forward compatibility for the file that motivated the fix:
        // failures recorded, no time. It must deserialize, and it must
        // not hold the owner.
        let json = r#"{"schema_version":1,"server":{"enabled":false,"bind":"127.0.0.1","port":8420},"failed_attempts":12}"#;
        let f: RemoteConfigFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.failed_attempts, 12);
        assert_eq!(f.last_failed_at, None);
        assert_eq!(
            f.auth().throttle.required_delay_secs(chrono::Utc::now()),
            0,
            "an owner locked out by the old build must be let back in"
        );
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
    #[error(
        "the stored TOTP secret is not a valid {SECRET_LEN_HINT}-byte base32 value; \
         2FA cannot be evaluated and the file is refused rather than treated as 2FA-off"
    )]
    BadTotpSecret,
    #[error("two registered passkeys share the credential id {0}")]
    DuplicatePasskeyId(String),
    #[error("a registered passkey has an empty credential id")]
    EmptyPasskeyId,
}

/// Only for the error text above; the authority is `totp::SECRET_LEN`.
const SECRET_LEN_HINT: usize = 20;

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

        // A secret that is present and unparseable must NOT be read as
        // "the second factor is off". `auth()` resolves it with
        // `and_then(from_base32)`, so a `None` there is indistinguishable
        // from never having configured TOTP — the login then succeeds on
        // the password alone, and `absorb()` writes the `None` back,
        // erasing the secret that would have proved otherwise. A silent
        // downgrade from two factors to one, made permanent by the next
        // login. This file fails loud on corruption precisely so that
        // cannot happen quietly.
        if let Some(raw) = self.totp_secret_base32.as_deref() {
            if TotpSecret::from_base32(raw).is_none() {
                return Err(ConfigValidationError::BadTotpSecret);
            }
        }

        // Passkeys are the way back in. A duplicate id silently shadows
        // one credential with another at lookup time, and an empty id
        // matches nothing, so both are states in which a phone that
        // still holds a working passkey is refused with no explanation.
        let mut seen = std::collections::HashSet::new();
        for pk in &self.passkeys {
            if pk.credential.id.is_empty() {
                return Err(ConfigValidationError::EmptyPasskeyId);
            }
            if !seen.insert(pk.credential.id.as_str()) {
                return Err(ConfigValidationError::DuplicatePasskeyId(
                    pk.credential.id.clone(),
                ));
            }
        }
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

    /// A file written before this field existed keeps the capability it
    /// already had.
    ///
    /// `approvals_enabled` is the one field here that defaults to
    /// `true`, and the reason is upgrades: it is only ever read on a
    /// machine whose owner has already turned the surface on and set a
    /// password, so defaulting it off would silently withdraw a working
    /// feature from every existing install. `#[serde(default)]` on a
    /// bool gives `false`, which is why this needs its own helper — and
    /// its own test, because losing the helper is a silent change of
    /// behaviour rather than a compile error.
    #[test]
    fn an_older_config_still_allows_approvals() {
        let older = r#"{"schema_version":1,"server":{"enabled":true}}"#;
        let cfg: RemoteConfigFile = serde_json::from_str(older).unwrap();
        assert!(cfg.approvals_enabled, "a missing field must not disable it");
    }

    /// …and an explicit `false` survives a round trip, which is the
    /// whole point of the toggle.
    #[test]
    fn approvals_off_round_trips() {
        let raw = r#"{"schema_version":1,"server":{"enabled":true},"approvals_enabled":false}"#;
        let cfg: RemoteConfigFile = serde_json::from_str(raw).unwrap();
        assert!(!cfg.approvals_enabled);

        let again: RemoteConfigFile =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert!(!again.approvals_enabled, "a save must not re-enable it");
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

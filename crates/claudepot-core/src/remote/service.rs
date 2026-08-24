//! The one implementation of every verb the remote surface offers.
//!
//! Both callers go through here: `claudepot remote …` and the GUI's
//! Settings → Remote pane. That is not tidiness — it is the lesson
//! `services::account_service` was extracted to record. From AGENTS.md,
//! on the panel's account-activate route: *"It exists because there
//! were briefly two, and the second had already dropped `resolve_email`:
//! a prefix that worked at the keyboard answered 'account not found'
//! from the phone."*
//!
//! The refusal in [`revoke_all`] is the one that would hurt most if a
//! second implementation dropped it. `remote-devices.json` **is** the
//! revocation list, so if it was unreadable and recovered to empty,
//! every `revoked_at` is already gone — and writing over it now makes
//! that loss permanent while reporting "Revoked 0", which reads as
//! "there was nothing to revoke" rather than "every stolen token is
//! live". A GUI button that did that would be worse than no button.
//!
//! What stays with each caller is **presentation**. The CLI prints a
//! paragraph about `0.0.0.0`; the pane renders an inline banner. They
//! say the same thing in different registers, and neither decides it —
//! [`Status::exposure`] does.

use std::net::IpAddr;

use chrono::Utc;
use uuid::Uuid;

use super::bind::Exposure;
use super::config::{self, RemoteConfigFile};
use super::{approval, password, store as device_store, Device, DevicesFile};

/// Every way a remote verb can fail.
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("could not read the remote configuration: {0}")]
    ConfigUnreadable(String),

    #[error("could not write the remote configuration: {0}")]
    ConfigUnwritable(String),

    #[error("could not read the paired-device records: {0}")]
    DevicesUnreadable(String),

    #[error("could not write the paired-device records: {0}")]
    DevicesUnwritable(String),

    #[error("not an IP address: {0}")]
    BadBindAddress(String),

    #[error(transparent)]
    BindRefused(#[from] super::bind::BindRefusal),

    #[error(
        "set a password first — enabling without one would open a surface \
         nobody can log into, and that is not a useful state to persist"
    )]
    NoPassword,

    #[error("the password is empty")]
    EmptyPassword,

    #[error("could not hash the password: {0}")]
    Hash(String),

    /// The device file was recovered, so every previous revocation is
    /// already lost. See the module docs — this is a refusal, not a
    /// warning, because the alternative reports success.
    #[error(
        "the paired-device file could not be read and was reset ({0}). Every previous \
         revocation has been lost, so this cannot honestly revoke anything. Re-pair the \
         devices you still want, and treat any token from before now as live."
    )]
    DevicesRecovered(String),

    #[error("no such device: {0}")]
    NoSuchDevice(Uuid),
}

/// One paired device or password-issued session, as a surface shows it.
///
/// The token hash is deliberately absent. It is not a secret worth
/// protecting on its own, but it is also not information — nothing a
/// user can do with it, and rendering it invites the belief that
/// comparing hashes is a thing they should be doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSummary {
    pub id: Uuid,
    pub name: String,
    pub created_at: chrono::DateTime<Utc>,
    pub last_seen: Option<chrono::DateTime<Utc>>,
    pub revoked_at: Option<chrono::DateTime<Utc>>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

impl From<&Device> for DeviceSummary {
    fn from(d: &Device) -> Self {
        Self {
            id: d.id,
            name: d.name.clone(),
            created_at: d.created_at,
            last_seen: d.last_seen,
            revoked_at: d.revoked_at,
            expires_at: d.expires_at,
        }
    }
}

/// Everything a surface needs to describe the remote appliance.
#[derive(Debug, Clone)]
pub struct Status {
    /// The stored preference. **Not liveness** — see [`Status::serving`].
    pub enabled: bool,
    /// Whether a server is actually up, from the heartbeat.
    ///
    /// `enabled` survives a `kill -9`, so a surface that renders it as
    /// "Running" lies in exactly the way `remote::approval`'s runtime
    /// gate was built not to: *"the server heartbeats every 5 s and the
    /// hook believes the heartbeat, not the preference."* A pane owes
    /// the user the same distinction the hook already makes, which is
    /// why these are two fields and not one.
    pub serving: bool,
    pub bind: IpAddr,
    pub port: u16,
    /// `None` when the bind address is refused; `bind_error` says why.
    pub exposure: Option<Exposure>,
    pub bind_error: Option<String>,
    /// True when the address is refused, too — a refused bind cannot be
    /// served in plaintext either, and reporting `false` would read as
    /// "no certificate needed".
    pub requires_tls: bool,
    pub password_set: bool,
    pub totp_enabled: bool,
    pub passkeys: usize,
    /// The config file was unreadable and was reset. The login throttle
    /// and the spent-TOTP high-water mark were in it.
    pub config_recovered: bool,
    /// The device file was unreadable and was reset — every previous
    /// revocation is gone. Revoking is refused while this holds.
    pub devices_recovered: bool,
    pub devices: Vec<DeviceSummary>,
}

impl Status {
    /// Active, unrevoked, unexpired devices.
    pub fn active_device_count(&self, now: chrono::DateTime<Utc>) -> usize {
        self.devices
            .iter()
            .filter(|d| d.revoked_at.is_none() && d.expires_at.is_none_or(|e| e > now))
            .count()
    }
}

fn load_config() -> Result<(RemoteConfigFile, bool), ServiceError> {
    let loaded = config::load().map_err(|e| ServiceError::ConfigUnreadable(e.to_string()))?;
    Ok((loaded.value, loaded.recovery.is_some()))
}

fn load_devices() -> Result<(DevicesFile, Option<String>), ServiceError> {
    let loaded =
        device_store::load().map_err(|e| ServiceError::DevicesUnreadable(e.to_string()))?;
    let recovery = loaded.recovery.as_ref().map(|r| format!("{r:?}"));
    Ok((loaded.value, recovery))
}

fn save_config(cfg: &RemoteConfigFile) -> Result<(), ServiceError> {
    config::save(cfg).map_err(|e| ServiceError::ConfigUnwritable(format!("{e:?}")))
}

/// Read everything, including whether a server is actually up.
///
/// `now_ms` is injected so a test can drive the heartbeat window.
pub fn status(now_ms: u64) -> Result<Status, ServiceError> {
    let (cfg, config_recovered) = load_config()?;
    let (devices, devices_recovery) = load_devices()?;
    let checked = cfg.server.checked_bind();

    Ok(Status {
        enabled: cfg.server.enabled,
        serving: approval::store::is_serving(&approval::store::dir(), now_ms),
        bind: cfg.server.bind,
        port: cfg.server.port,
        exposure: checked.as_ref().ok().map(|b| b.exposure()),
        bind_error: checked.as_ref().err().map(|e| e.to_string()),
        // A refused address is not "no TLS needed".
        requires_tls: checked.as_ref().map(|b| b.requires_tls()).unwrap_or(true),
        password_set: cfg.password_hash.is_some(),
        totp_enabled: cfg.totp_secret_base32.is_some(),
        passkeys: cfg.passkeys.len(),
        config_recovered,
        devices_recovered: devices_recovery.is_some(),
        devices: devices.devices.iter().map(DeviceSummary::from).collect(),
    })
}

/// What [`enable`] settled on, so the caller can warn about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enabled {
    pub bind: IpAddr,
    pub port: u16,
    pub exposure: Exposure,
    pub requires_tls: bool,
}

/// Turn the surface on, refusing before anything reaches disk.
///
/// Order is load-bearing twice over: the bind is checked before the
/// write so a refused address never persists, and the password is
/// required before the write so "enabled" never means "listening and
/// unloggable".
pub fn enable(bind: Option<&str>, port: Option<u16>) -> Result<Enabled, ServiceError> {
    let (mut cfg, _) = load_config()?;
    if let Some(b) = bind {
        cfg.server.bind = b
            .parse()
            .map_err(|_| ServiceError::BadBindAddress(b.to_string()))?;
    }
    if let Some(p) = port {
        cfg.server.port = p;
    }
    let checked = cfg.server.checked_bind()?;
    if cfg.password_hash.is_none() {
        return Err(ServiceError::NoPassword);
    }
    cfg.server.enabled = true;
    save_config(&cfg)?;

    Ok(Enabled {
        bind: cfg.server.bind,
        port: cfg.server.port,
        exposure: checked.exposure(),
        requires_tls: checked.requires_tls(),
    })
}

/// Turn the surface off.
///
/// This is the *preference* only. Stopping a running server is the
/// caller's job — the CLI's is a foreground process that ends with the
/// command, and the GUI holds a task handle. Writing `false` here while
/// a server keeps serving would be the `enabled`-is-not-liveness
/// confusion from the other side.
pub fn disable() -> Result<(), ServiceError> {
    let (mut cfg, _) = load_config()?;
    cfg.server.enabled = false;
    save_config(&cfg)
}

/// Set the admin password. `plain` is zeroized on every path.
pub fn set_password(plain: &mut String) -> Result<(), ServiceError> {
    use zeroize::Zeroize;
    if plain.is_empty() {
        plain.zeroize();
        return Err(ServiceError::EmptyPassword);
    }
    // `hash_password` zeroizes on every path, success or error.
    let hash = password::hash_password(plain).map_err(|e| ServiceError::Hash(e.to_string()))?;

    let (mut cfg, _) = load_config()?;
    cfg.password_hash = Some(hash);
    // A new password ends the lockout: whoever can set it is the owner,
    // and leaving them throttled would be theatre.
    cfg.failed_attempts = 0;
    cfg.last_failed_at = None;
    save_config(&cfg)
}

/// Revoke every session and paired device. Returns how many changed.
pub fn revoke_all() -> Result<usize, ServiceError> {
    let (mut devices, recovery) = load_devices()?;
    if let Some(r) = recovery {
        return Err(ServiceError::DevicesRecovered(r));
    }
    let now = Utc::now();
    let mut n = 0;
    for d in devices.devices.iter_mut() {
        if d.revoked_at.is_none() {
            // Set, never delete — a deleted device is merely unknown,
            // a revoked one is refused.
            d.revoked_at = Some(now);
            n += 1;
        }
    }
    device_store::save(&devices).map_err(|e| ServiceError::DevicesUnwritable(format!("{e:?}")))?;
    Ok(n)
}

/// Revoke one device. `false` when it was already revoked.
///
/// Refuses on a recovered file for the same reason [`revoke_all`] does:
/// the other records' `revoked_at` marks are already gone, and saving
/// now writes that loss to disk.
pub fn revoke_device(id: Uuid) -> Result<bool, ServiceError> {
    let (mut devices, recovery) = load_devices()?;
    if let Some(r) = recovery {
        return Err(ServiceError::DevicesRecovered(r));
    }
    let Some(d) = devices.devices.iter_mut().find(|d| d.id == id) else {
        return Err(ServiceError::NoSuchDevice(id));
    };
    if d.revoked_at.is_some() {
        return Ok(false);
    }
    d.revoked_at = Some(Utc::now());
    device_store::save(&devices).map_err(|e| ServiceError::DevicesUnwritable(format!("{e:?}")))?;
    Ok(true)
}

/// Writes both files the surface owns.
///
/// Moved here from the CLI, where a second caller would have had to
/// reimplement it — and a `Persist` that saved only one of the two
/// files is a silent half-write on every mutation the server makes.
pub struct FilePersist;

impl super::server::Persist for FilePersist {
    fn save(&self, cfg: &RemoteConfigFile, devices: &DevicesFile) -> std::io::Result<()> {
        config::save(cfg).map_err(|e| std::io::Error::other(format!("{e:?}")))?;
        device_store::save(devices).map_err(|e| std::io::Error::other(format!("{e:?}")))
    }
}

/// Installs CC's `PermissionRequest` hook for the life of a server, and
/// takes it back out on the way down.
///
/// Moved here from the CLI so the GUI arms it the same way. The
/// coupling is the security argument, not a convenience: AGENTS.md —
/// *"Remote approval is armed for exactly as long as the surface that
/// reaches the phone is up — that coupling is what makes it acceptable
/// to hand a network client the ability to grant a permission at all."*
/// Two implementations of "arm" is two chances to get the disarm wrong.
pub struct ApprovalHook {
    beat: tokio::task::JoinHandle<()>,
    installed: bool,
    /// Where the warnings went, so a GUI can surface what a terminal
    /// would have printed to stderr. Empty is the good case.
    pub warnings: Vec<String>,
}

impl ApprovalHook {
    /// Arm it. Never fails: a hook that could not be installed means
    /// permission prompts are drawn at the machine as they always were,
    /// which is the fall-through the whole design rests on.
    pub fn arm() -> Self {
        let dir = approval::store::dir();
        // Heartbeat first: it is what the hook actually believes, so a
        // hook that starts before the first beat must read "not
        // serving" rather than wait on a server that is not up yet.
        let _ = approval::store::mark_serving(&dir, approval::now_ms());

        let mut warnings = Vec::new();
        // `current_exe` and not a looked-up path: this binary is the one
        // carrying the verb, so the hook cannot point at a Claudepot
        // that is not this one.
        let installed = match std::env::current_exe() {
            Ok(binary) => match approval::install::install(&binary) {
                Ok(_) => true,
                Err(e) => {
                    warnings.push(format!(
                        "remote approval is OFF — could not install Claude Code's \
                         PermissionRequest hook: {e}"
                    ));
                    false
                }
            },
            Err(e) => {
                warnings.push(format!(
                    "remote approval is OFF — cannot locate this binary: {e}"
                ));
                false
            }
        };

        let beat = tokio::spawn(async move {
            let mut tick = tokio::time::interval(approval::HEARTBEAT);
            loop {
                tick.tick().await;
                let now = approval::now_ms();
                let _ = approval::store::mark_serving(&dir, now);
                // Cheap, and the only thing that clears up after a hook
                // that was killed with its session.
                approval::store::sweep(&dir, now);
            }
        });

        Self {
            beat,
            installed,
            warnings,
        }
    }
}

impl Drop for ApprovalHook {
    fn drop(&mut self) {
        self.beat.abort();
        approval::store::stop_serving(&approval::store::dir());
        if self.installed {
            // Best effort: the runtime gate is what makes a survivor
            // harmless, so a failure here is untidy rather than unsafe.
            let _ = approval::install::uninstall();
        }
    }
}

/// Stable words for `--json` and for IPC. `Exposure`'s `Debug` is not a
/// wire format.
pub fn exposure_word(e: Exposure) -> &'static str {
    match e {
        Exposure::Loopback => "loopback",
        Exposure::PrivateNetwork => "private_network",
        Exposure::EveryInterface => "every_interface",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(revoked: bool, expires: Option<chrono::DateTime<Utc>>) -> DeviceSummary {
        DeviceSummary {
            id: Uuid::new_v4(),
            name: "phone".into(),
            created_at: Utc::now(),
            last_seen: None,
            revoked_at: revoked.then(Utc::now),
            expires_at: expires,
        }
    }

    fn status_with(devices: Vec<DeviceSummary>) -> Status {
        Status {
            enabled: false,
            serving: false,
            bind: "127.0.0.1".parse().unwrap(),
            port: 8420,
            exposure: Some(Exposure::Loopback),
            bind_error: None,
            requires_tls: false,
            password_set: false,
            totp_enabled: false,
            passkeys: 0,
            config_recovered: false,
            devices_recovered: false,
            devices,
        }
    }

    #[test]
    fn a_revoked_device_is_not_active() {
        let s = status_with(vec![device(true, None), device(false, None)]);
        assert_eq!(s.active_device_count(Utc::now()), 1);
    }

    #[test]
    fn an_expired_session_is_not_active() {
        // A password-issued session carries an expiry; a paired device
        // does not. Counting the expired one would tell the user a
        // credential is live when nothing would accept it.
        let past = Utc::now() - chrono::Duration::hours(1);
        let future = Utc::now() + chrono::Duration::hours(1);
        let s = status_with(vec![device(false, Some(past)), device(false, Some(future))]);
        assert_eq!(s.active_device_count(Utc::now()), 1);
    }

    #[test]
    fn exposure_words_are_stable() {
        // These reach `--json` and IPC. Renaming one silently changes a
        // wire value that a renderer branches on.
        assert_eq!(exposure_word(Exposure::Loopback), "loopback");
        assert_eq!(exposure_word(Exposure::PrivateNetwork), "private_network");
        assert_eq!(exposure_word(Exposure::EveryInterface), "every_interface");
    }
}

//! Paired devices for the remote-control surface.
//!
//! Claudepot's server binds **loopback only** and is exposed to the
//! user's other machines by `tailscale serve`. That choice does most of
//! the security work — nothing on the LAN can reach the port, TLS and
//! tailnet identity are handled outside the process, and if Tailscale
//! is down the surface is simply unreachable rather than quietly
//! listening.
//!
//! ## What a device token actually defends against — and what it does not
//!
//! It authenticates a **remote** device. It does not, and cannot,
//! defend against arbitrary code running as the same Unix user.
//!
//! `remote-devices.json` is owner-writable JSON and `authenticate`
//! trusts any active record in it, so a same-UID process can simply
//! insert `SHA256(token_of_its_choosing)` and log in. That is not worth
//! fixing, because the same process can already read
//! `~/.claude/sessions/<pid>.<hash>.key` and drive CC's socket directly
//! without involving Claudepot at all — CC's own boundary here is the
//! Unix user, as `peer::key`'s docs say. Claudepot adds no exposure it
//! could take away.
//!
//! The honest claim is therefore narrow: this stops *another device*
//! from acting without pairing, and *a revoked device* from acting at
//! all. Anything stronger needs a real OS isolation boundary — a
//! separate UID or a sandbox — not a bearer token. Do not write
//! documentation or UI copy implying otherwise.
//!
//! The `Tailscale-User-*` headers a proxy might inject are likewise not
//! authentication: a local process can forge them. They say which
//! tailnet user, never which device.
//!
//! ## Pairing
//!
//! The GUI starts a pairing window and shows an eight-character code.
//! The phone posts the code once and receives a device token it keeps.
//! The code is short because a human retypes it, so its defence is
//! expiry plus an attempt cap rather than entropy — see [`token`] for
//! why that asymmetry is deliberate.
//!
//! Revocation is per device and immediate: the token hash stays on
//! record with `revoked_at` set, so a revoked device is *refused*
//! rather than *forgotten*. Deleting the row would let the same token
//! pair again as a "new" device if it were ever replayed, and would
//! erase the audit trail of what had been let in.

pub mod api;
pub mod assets;
pub mod bind;
pub mod config;
pub mod idempotency;
pub mod login;
pub mod panel;
pub mod passkey;
pub mod password;
pub mod serve;
pub mod server;
pub mod store;
pub mod tls;
pub mod token;
pub mod totp;
pub mod webauthn;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use bind::{check as check_bind, BindAddr, BindRefusal};
pub use store::{devices_path, load, save, DEVICES_FILENAME};
pub use token::{new_device_token, new_pairing_code, NewToken};

pub const SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

/// How long a pairing code stays valid.
///
/// Short on purpose. The window is "the user is holding their phone in
/// front of the screen", not "sometime today".
pub const PAIRING_WINDOW_SECS: i64 = 180;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Device {
    pub id: Uuid,
    /// User-supplied label. Display only — never trusted for anything.
    pub name: String,
    /// SHA-256 of the token. The plaintext is never stored.
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub last_seen: Option<DateTime<Utc>>,
    /// Set rather than removed — see the module docs.
    #[serde(default)]
    pub revoked_at: Option<DateTime<Utc>>,
    /// `None` for a paired device (valid until revoked); `Some` for a
    /// password-issued session. A session and a paired device are the
    /// same thing — a token hash `authenticate` accepts — so they share
    /// one representation rather than growing a parallel one with its
    /// own revocation path to forget about.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

impl Device {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }

    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|e| now >= e)
    }

    pub fn is_usable_at(&self, now: DateTime<Utc>) -> bool {
        self.is_active() && !self.is_expired_at(now)
    }
}

/// A pairing window the user opened from the GUI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingPairing {
    /// Hash, not the code. A pairing file readable at rest must not
    /// hand over a live code.
    pub code_hash: String,
    pub started_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub attempts: u32,
}

impl PendingPairing {
    pub fn start(code: &str, now: DateTime<Utc>) -> Self {
        Self {
            code_hash: token::hash_secret(code),
            started_at: now,
            expires_at: now + Duration::seconds(PAIRING_WINDOW_SECS),
            attempts: 0,
        }
    }

    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevicesFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub devices: Vec<Device>,
    /// At most one window open at a time. Two concurrent codes double
    /// the guessing surface for no user benefit.
    #[serde(default)]
    pub pending: Option<PendingPairing>,
}

impl Default for DevicesFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            devices: Vec::new(),
            pending: None,
        }
    }
}

/// Why a pairing attempt did not produce a device.
///
/// Every rejection is one variant so the caller cannot accidentally
/// tell the client *which* thing was wrong. The GUI can be specific
/// because it is the user; the HTTP surface must not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingRejection {
    /// No window is open.
    NotOpen,
    /// The window's deadline passed.
    Expired,
    /// Too many wrong guesses; the window is destroyed.
    Burned,
    /// Wrong code. The attempt has been counted.
    Wrong,
}

/// Decide a pairing attempt. Pure; the caller performs the effects.
///
/// Returns the rejection *and* the pending record as it should now be
/// stored — `None` meaning "destroy the window". Making the caller
/// write back an updated record is what stops an attacker from
/// retrying forever: a version of this that only returned a verdict
/// would leave `attempts` un-incremented on the losing path, which is
/// precisely the bug that turns an 8-character code into a password.
pub fn judge_pairing(
    pending: Option<&PendingPairing>,
    code: &str,
    now: DateTime<Utc>,
) -> Result<(), (PairingRejection, Option<PendingPairing>)> {
    let Some(p) = pending else {
        return Err((PairingRejection::NotOpen, None));
    };
    if p.is_expired_at(now) {
        return Err((PairingRejection::Expired, None));
    }
    if p.attempts >= token::MAX_CODE_ATTEMPTS {
        return Err((PairingRejection::Burned, None));
    }
    if token::verify_secret(code, &p.code_hash) {
        // Single use: the window closes on success too.
        return Ok(());
    }
    let mut next = p.clone();
    next.attempts += 1;
    if next.attempts >= token::MAX_CODE_ATTEMPTS {
        return Err((PairingRejection::Burned, None));
    }
    Err((PairingRejection::Wrong, Some(next)))
}

/// Find the active device a bearer token belongs to.
///
/// Revoked devices are skipped, not matched-then-rejected, so a revoked
/// token is indistinguishable from an unknown one to the caller.
pub fn authenticate<'a>(
    devices: &'a [Device],
    bearer: &str,
    now: DateTime<Utc>,
) -> Option<&'a Device> {
    devices
        .iter()
        .filter(|d| d.is_usable_at(now))
        .find(|d| token::verify_secret(bearer, &d.token_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, 10, m, 0).unwrap()
    }

    fn device(name: &str, tok: &NewToken, revoked: bool) -> Device {
        Device {
            id: Uuid::new_v4(),
            name: name.into(),
            token_hash: tok.hash.clone(),
            created_at: at(0),
            last_seen: None,
            revoked_at: revoked.then(|| at(5)),
            expires_at: None,
        }
    }

    #[test]
    fn a_correct_code_pairs() {
        let code = new_pairing_code();
        let p = PendingPairing::start(&code, at(0));
        assert!(judge_pairing(Some(&p), &code, at(1)).is_ok());
    }

    #[test]
    fn the_stored_record_is_a_hash_not_the_code() {
        let code = new_pairing_code();
        let p = PendingPairing::start(&code, at(0));
        assert_ne!(p.code_hash, code);
        assert!(!p.code_hash.contains(&code));
    }

    #[test]
    fn no_open_window_rejects() {
        let (why, next) = judge_pairing(None, "ABCDEFGH", at(0)).unwrap_err();
        assert_eq!(why, PairingRejection::NotOpen);
        assert!(next.is_none());
    }

    #[test]
    fn an_expired_window_rejects_a_correct_code() {
        let code = new_pairing_code();
        let p = PendingPairing::start(&code, at(0));
        let (why, next) = judge_pairing(Some(&p), &code, at(10)).unwrap_err();
        assert_eq!(why, PairingRejection::Expired);
        assert!(next.is_none(), "an expired window is destroyed, not kept");
    }

    #[test]
    fn a_wrong_code_counts_the_attempt() {
        let code = new_pairing_code();
        let p = PendingPairing::start(&code, at(0));
        let (why, next) = judge_pairing(Some(&p), "WRONGWRO", at(1)).unwrap_err();
        assert_eq!(why, PairingRejection::Wrong);
        assert_eq!(
            next.expect("window survives one wrong guess").attempts,
            1,
            "an un-incremented counter turns the code into a password"
        );
    }

    #[test]
    fn the_window_burns_after_the_attempt_cap() {
        let code = new_pairing_code();
        let mut p = PendingPairing::start(&code, at(0));
        for i in 1..token::MAX_CODE_ATTEMPTS {
            let (why, next) = judge_pairing(Some(&p), "WRONGWRO", at(1)).unwrap_err();
            assert_eq!(why, PairingRejection::Wrong, "attempt {i}");
            p = next.expect("still open");
        }
        let (why, next) = judge_pairing(Some(&p), "WRONGWRO", at(1)).unwrap_err();
        assert_eq!(why, PairingRejection::Burned);
        assert!(next.is_none(), "a burned window must not be retryable");
    }

    #[test]
    fn a_burned_window_refuses_even_the_right_code() {
        let code = new_pairing_code();
        let mut p = PendingPairing::start(&code, at(0));
        p.attempts = token::MAX_CODE_ATTEMPTS;
        let (why, _) = judge_pairing(Some(&p), &code, at(1)).unwrap_err();
        assert_eq!(why, PairingRejection::Burned);
    }

    #[test]
    fn a_valid_token_authenticates() {
        let tok = new_device_token();
        let devices = vec![device("phone", &tok, false)];
        assert_eq!(
            authenticate(&devices, &tok.plaintext, at(1)).unwrap().name,
            "phone"
        );
    }

    #[test]
    fn a_revoked_device_never_authenticates() {
        let tok = new_device_token();
        let devices = vec![device("old phone", &tok, true)];
        assert!(
            authenticate(&devices, &tok.plaintext, at(1)).is_none(),
            "revocation must be immediate and total"
        );
    }

    #[test]
    fn an_unknown_token_authenticates_nothing() {
        let tok = new_device_token();
        let devices = vec![device("phone", &tok, false)];
        assert!(authenticate(&devices, &new_device_token().plaintext, at(1)).is_none());
        assert!(authenticate(&devices, "", at(1)).is_none());
    }

    #[test]
    fn revoking_one_device_leaves_the_others() {
        let a = new_device_token();
        let b = new_device_token();
        let devices = vec![device("revoked", &a, true), device("live", &b, false)];
        assert!(authenticate(&devices, &a.plaintext, at(1)).is_none());
        assert_eq!(
            authenticate(&devices, &b.plaintext, at(1)).unwrap().name,
            "live"
        );
    }

    #[test]
    fn an_expired_session_stops_authenticating() {
        let tok = new_device_token();
        let mut d = device("phone", &tok, false);
        d.expires_at = Some(at(30));
        let devices = vec![d];
        assert!(authenticate(&devices, &tok.plaintext, at(29)).is_some());
        assert!(
            authenticate(&devices, &tok.plaintext, at(30)).is_none(),
            "expiry is inclusive — a session that survives its own deadline \
             can be extended by a slow clock"
        );
        assert!(authenticate(&devices, &tok.plaintext, at(31)).is_none());
    }

    #[test]
    fn a_paired_device_without_an_expiry_never_times_out() {
        let tok = new_device_token();
        let d = device("phone", &tok, false);
        assert_eq!(d.expires_at, None);
        assert!(authenticate(&[d], &tok.plaintext, at(59)).is_some());
    }

    #[test]
    fn revocation_beats_a_live_expiry() {
        let tok = new_device_token();
        let mut d = device("phone", &tok, true);
        d.expires_at = Some(at(59));
        assert!(authenticate(&[d], &tok.plaintext, at(1)).is_none());
    }

    #[test]
    fn a_device_record_never_carries_the_plaintext_token() {
        let tok = new_device_token();
        let d = device("phone", &tok, false);
        let json = serde_json::to_string(&d).unwrap();
        assert!(
            !json.contains(&tok.plaintext),
            "the on-disk device record leaked its bearer token"
        );
    }
}

//! Passkeys over HTTP: the ceremony state, and the origin rules.
//!
//! [`super::webauthn`] does the cryptography — ES256, no attestation,
//! counter-regression refused. This module is everything around it: what
//! a challenge is, how long it lives, and what relying-party identity
//! the server claims.
//!
//! ## The RP ID must be a hostname, and it is derived, never configured
//!
//! WebAuthn requires the relying-party ID to be a valid domain. An
//! IP-address origin has none, so `https://100.64.x.x:8420` cannot
//! register a passkey however capable the phone is — and
//! `isUserVerifyingPlatformAuthenticatorAvailable()` returns `true` on
//! exactly that origin, because it answers "does this DEVICE have Face
//! ID". The client checks the origin separately; the server refuses with
//! [`PasskeyError::RpIdUnavailable`] rather than registering a
//! credential no browser will ever offer back.
//!
//! Deriving it from the request's `Host` rather than storing it means
//! the same appliance reached by `.local` and by its MagicDNS name gets
//! two credentials rather than one broken one — which is correct: a
//! passkey is scoped to an origin by design, and the minted certificate
//! already covers both names. `guard_origin` has already refused any
//! `Host` we are not willing to answer to before this runs, so the
//! derived RP ID is not attacker-chosen in any way that matters.
//!
//! ## Registration requires an authenticated session
//!
//! Otherwise anyone who can reach the page enrols themselves. The
//! password stays the bootstrap and recovery credential; a passkey is
//! something an already-trusted session adds.
//!
//! ## Challenges are in memory, short-lived, single-use
//!
//! Same reasoning as [`super::idempotency`]: a ceremony completes in
//! seconds, and a restart between begin and finish is indistinguishable
//! from a ceremony the user abandoned — which the client already
//! handles. Single-use is not optional: a replayable challenge turns a
//! captured assertion into a reusable credential.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::webauthn::{self, Credential, WebauthnError};

/// How long a ceremony may take. The browser's own timeout is 60s; this
/// is the server's, and it is deliberately a little longer so the
/// browser is what tells the user it gave up.
pub const CHALLENGE_TTL: Duration = Duration::from_secs(120);

/// Concurrent open ceremonies. Registration is authenticated, but
/// `login/begin` is not — anything an unauthenticated caller can make
/// the server accumulate needs a ceiling.
pub const MAX_CHALLENGES: usize = 64;

/// Registered credentials. A passkey is a credential, not a device: it
/// signs you in, and revoking a *session* must not delete the thing you
/// sign in with.
pub const MAX_CREDENTIALS: usize = 20;

/// Bytes of challenge. 32 is the spec's recommendation and what every
/// implementation uses.
const CHALLENGE_BYTES: usize = 32;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PasskeyError {
    #[error("this origin has no hostname, so it cannot be a WebAuthn relying party")]
    RpIdUnavailable,
    #[error("no such challenge, or it has already been used")]
    UnknownChallenge,
    #[error("no passkey is registered")]
    NoCredential,
    #[error("as many passkeys as this appliance will hold are already registered")]
    TooManyCredentials,
    #[error("verification failed: {0}")]
    Verification(String),
    #[error("malformed base64url in the credential response")]
    BadBase64,
}

impl From<WebauthnError> for PasskeyError {
    fn from(e: WebauthnError) -> Self {
        PasskeyError::Verification(e.to_string())
    }
}

/// Which ceremony a challenge was issued for.
///
/// Kept on the record rather than inferred from the endpoint: a
/// registration challenge finishing a login is exactly the confusion
/// WebAuthn's own `type` field exists to prevent, and checking it in two
/// independent places costs nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ceremony {
    Register,
    Login,
}

#[derive(Debug)]
struct Pending {
    challenge: Vec<u8>,
    ceremony: Ceremony,
    /// The exact origin string the client must report back.
    origin: String,
    rp_id: String,
    at: Instant,
}

/// Open ceremonies, keyed by an opaque id the client echoes back.
#[derive(Default)]
pub struct Challenges {
    entries: HashMap<String, Pending>,
}

/// What `begin` hands the client.
#[derive(Debug)]
pub struct Issued {
    pub challenge_id: String,
    pub challenge_b64: String,
    pub rp_id: String,
}

impl Challenges {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Mint a challenge for `origin`.
    pub fn issue(
        &mut self,
        ceremony: Ceremony,
        origin: &str,
        now: Instant,
    ) -> Result<Issued, PasskeyError> {
        let rp_id = rp_id_from_origin(origin).ok_or(PasskeyError::RpIdUnavailable)?;
        self.expire(now);

        // `token::random_bytes` rather than a new dependency: it is
        // already the module that reasons about where randomness comes
        // from, and about which UUID bytes are not random.
        let challenge = super::token::random_bytes(CHALLENGE_BYTES);
        let id = uuid::Uuid::new_v4().to_string();

        if self.entries.len() >= MAX_CHALLENGES {
            // Oldest first: a flood costs bounded memory and at worst
            // makes an honest in-flight ceremony fail, which the client
            // recovers from by starting another.
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, p)| p.at)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest);
            }
        }

        let issued = Issued {
            challenge_id: id.clone(),
            challenge_b64: URL_SAFE_NO_PAD.encode(&challenge),
            rp_id: rp_id.clone(),
        };
        self.entries.insert(
            id,
            Pending {
                challenge,
                ceremony,
                origin: origin.to_string(),
                rp_id,
                at: now,
            },
        );
        Ok(issued)
    }

    /// Take a challenge. Single use: present or not, it is gone after
    /// this call.
    fn take(
        &mut self,
        id: &str,
        ceremony: Ceremony,
        now: Instant,
    ) -> Result<Pending, PasskeyError> {
        self.expire(now);
        let pending = self
            .entries
            .remove(id)
            .ok_or(PasskeyError::UnknownChallenge)?;
        if pending.ceremony != ceremony {
            return Err(PasskeyError::UnknownChallenge);
        }
        Ok(pending)
    }

    fn expire(&mut self, now: Instant) {
        self.entries
            .retain(|_, p| now.duration_since(p.at) < CHALLENGE_TTL);
    }
}

/// The client's half of a registration.
#[derive(Debug, Deserialize)]
pub struct RegisterResponse {
    pub challenge_id: String,
    pub client_data_json: String,
    pub attestation_object: String,
}

/// The client's half of an assertion.
#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    pub challenge_id: String,
    /// base64url credential id, as `PublicKeyCredential.id`.
    pub id: String,
    pub client_data_json: String,
    pub authenticator_data: String,
    pub signature: String,
}

/// A registered passkey as stored in `remote-config.json`.
///
/// Only a public key crosses the disk, which is the whole point: reading
/// the file gives an attacker nothing, where a password hash gives them
/// a cracking job and a TOTP secret gives them working access.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PasskeyRecord {
    pub credential: Credential,
    /// User-supplied label. Display only.
    pub label: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub last_used: Option<chrono::DateTime<chrono::Utc>>,
    /// The origin this credential was registered against. A passkey is
    /// scoped to an origin, so the same appliance reached by two names
    /// legitimately holds two.
    pub origin: String,
}

/// Verify a registration and produce the record to store.
pub fn finish_registration(
    challenges: &mut Challenges,
    body: &RegisterResponse,
    label: &str,
    existing: &[PasskeyRecord],
    now: Instant,
    wall: chrono::DateTime<chrono::Utc>,
) -> Result<PasskeyRecord, PasskeyError> {
    let pending = challenges.take(&body.challenge_id, Ceremony::Register, now)?;
    let client_data = decode(&body.client_data_json)?;
    let attestation = decode(&body.attestation_object)?;

    let credential = webauthn::verify_registration(
        &client_data,
        &attestation,
        &pending.challenge,
        &pending.origin,
        &pending.rp_id,
    )?;

    // The cap is checked **after** verification, because only then is
    // the credential id known — and with it, whether this registration
    // adds a credential or replaces one.
    if !cap_allows(existing, &credential.id) {
        return Err(PasskeyError::TooManyCredentials);
    }

    Ok(PasskeyRecord {
        credential,
        label: label.to_string(),
        created_at: wall,
        last_used: None,
        origin: pending.origin,
    })
}

/// Verify an assertion. Returns the index of the credential that signed
/// and its new counter, so the caller can persist both.
pub fn finish_login(
    challenges: &mut Challenges,
    body: &LoginResponse,
    stored: &[PasskeyRecord],
    now: Instant,
) -> Result<(usize, u32), PasskeyError> {
    let pending = challenges.take(&body.challenge_id, Ceremony::Login, now)?;
    if stored.is_empty() {
        return Err(PasskeyError::NoCredential);
    }
    let index = stored
        .iter()
        .position(|r| r.credential.id == body.id)
        .ok_or(PasskeyError::NoCredential)?;

    let client_data = decode(&body.client_data_json)?;
    let auth_data = decode(&body.authenticator_data)?;
    let signature = decode(&body.signature)?;

    let counter = webauthn::verify_assertion(
        &stored[index].credential,
        &client_data,
        &auth_data,
        &signature,
        &pending.challenge,
        &pending.origin,
        &pending.rp_id,
    )?;
    Ok((index, counter))
}

/// Whether a registration may proceed given what is already stored.
///
/// Pure, and separate, for two reasons. It is the rule worth testing —
/// exercising it through `finish_registration` needs a real attestation,
/// which a unit test cannot mint. And stating it as one expression makes
/// the asymmetry visible: **a replacement is always allowed**, because
/// the caller replaces by credential id and the stored count therefore
/// does not grow. Checking the cap first refused re-enrolling a phone
/// the appliance already trusted, on arithmetic about a number that was
/// never going to change.
fn cap_allows(existing: &[PasskeyRecord], credential_id: &str) -> bool {
    existing.iter().any(|r| r.credential.id == credential_id) || existing.len() < MAX_CREDENTIALS
}

fn decode(s: &str) -> Result<Vec<u8>, PasskeyError> {
    URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|_| PasskeyError::BadBase64)
}

/// The RP ID a browser would derive from this origin, or `None` when the
/// origin disqualifies itself.
///
/// Refused: IP literals of either family, and anything with no host at
/// all. An IP has no domain, and WebAuthn's RP ID is required to be one.
pub fn rp_id_from_origin(origin: &str) -> Option<String> {
    let rest = origin.split_once("://").map(|(_, r)| r).unwrap_or(origin);
    let host = rest.split(['/', '?', '#']).next()?;
    // IPv6 arrives bracketed; strip and refuse.
    if let Some(inner) = host.strip_prefix('[') {
        let _ = inner.split_once(']')?;
        return None;
    }
    let name = match host.rsplit_once(':') {
        Some((n, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => n,
        _ => host,
    };
    if name.is_empty() {
        return None;
    }
    if name.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    // A bare label is a valid RP ID (`localhost`, `laptop`).
    Some(name.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ip_origin_has_no_relying_party_id() {
        // The trap this exists for: the device reports a platform
        // authenticator is available on exactly the origin that cannot
        // use one.
        // Documentation/reserved ranges only (RFC 5737, RFC 3849) plus
        // the CGNAT block Tailscale uses. A real address from someone's
        // network would test nothing extra and would outlive the test.
        for origin in [
            "https://100.64.0.1:8420",
            "https://192.0.2.10",
            "http://127.0.0.1:8420",
            "https://[2001:db8::1]:8420",
        ] {
            assert_eq!(rp_id_from_origin(origin), None, "{origin}");
        }
    }

    #[test]
    fn a_named_origin_yields_its_hostname() {
        assert_eq!(
            rp_id_from_origin("https://laptop.tailnet-example.ts.net:8420").as_deref(),
            Some("laptop.tailnet-example.ts.net")
        );
        assert_eq!(
            rp_id_from_origin("https://laptop.local").as_deref(),
            Some("laptop.local")
        );
        assert_eq!(
            rp_id_from_origin("http://localhost:5173").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            rp_id_from_origin("https://Laptop.Local").as_deref(),
            Some("laptop.local"),
            "an RP ID comparison is case-sensitive, so normalise once here"
        );
    }

    #[test]
    fn issuing_against_an_ip_origin_is_refused_not_deferred() {
        let mut c = Challenges::new();
        let err = c
            .issue(
                Ceremony::Register,
                "https://100.64.1.2:8420",
                Instant::now(),
            )
            .unwrap_err();
        assert_eq!(err, PasskeyError::RpIdUnavailable);
        assert!(
            c.is_empty(),
            "a refused ceremony must not leave state behind"
        );
    }

    #[test]
    fn a_challenge_is_single_use() {
        let mut c = Challenges::new();
        let now = Instant::now();
        let issued = c.issue(Ceremony::Login, "https://host.local", now).unwrap();
        assert!(c.take(&issued.challenge_id, Ceremony::Login, now).is_ok());
        assert_eq!(
            c.take(&issued.challenge_id, Ceremony::Login, now)
                .unwrap_err(),
            PasskeyError::UnknownChallenge,
            "a replayable challenge makes a captured assertion reusable"
        );
    }

    #[test]
    fn a_registration_challenge_cannot_finish_a_login() {
        let mut c = Challenges::new();
        let now = Instant::now();
        let issued = c
            .issue(Ceremony::Register, "https://host.local", now)
            .unwrap();
        assert_eq!(
            c.take(&issued.challenge_id, Ceremony::Login, now)
                .unwrap_err(),
            PasskeyError::UnknownChallenge
        );
    }

    #[test]
    fn a_challenge_expires() {
        let mut c = Challenges::new();
        let now = Instant::now();
        let issued = c.issue(Ceremony::Login, "https://host.local", now).unwrap();
        let later = now + CHALLENGE_TTL + Duration::from_secs(1);
        assert_eq!(
            c.take(&issued.challenge_id, Ceremony::Login, later)
                .unwrap_err(),
            PasskeyError::UnknownChallenge
        );
    }

    #[test]
    fn open_ceremonies_are_capped() {
        // `login/begin` is unauthenticated, so this map is something an
        // anonymous caller can grow.
        let mut c = Challenges::new();
        let now = Instant::now();
        for _ in 0..(MAX_CHALLENGES * 3) {
            c.issue(Ceremony::Login, "https://host.local", now).unwrap();
        }
        assert!(c.len() <= MAX_CHALLENGES, "{} open", c.len());
    }

    #[test]
    fn two_challenges_are_never_equal() {
        let mut c = Challenges::new();
        let now = Instant::now();
        let a = c.issue(Ceremony::Login, "https://host.local", now).unwrap();
        let b = c.issue(Ceremony::Login, "https://host.local", now).unwrap();
        assert_ne!(a.challenge_b64, b.challenge_b64);
        assert_ne!(a.challenge_id, b.challenge_id);
    }

    #[test]
    fn a_login_against_an_unknown_credential_id_is_refused() {
        let mut c = Challenges::new();
        let now = Instant::now();
        let issued = c.issue(Ceremony::Login, "https://host.local", now).unwrap();
        let body = LoginResponse {
            challenge_id: issued.challenge_id,
            id: "not-a-registered-id".into(),
            client_data_json: String::new(),
            authenticator_data: String::new(),
            signature: String::new(),
        };
        let stored = vec![PasskeyRecord {
            credential: Credential {
                id: "real".into(),
                public_key: "x".into(),
                sign_count: 0,
            },
            label: "iPhone".into(),
            created_at: chrono::Utc::now(),
            last_used: None,
            origin: "https://host.local".into(),
        }];
        assert_eq!(
            finish_login(&mut c, &body, &stored, now).unwrap_err(),
            PasskeyError::NoCredential
        );
    }

    #[test]
    fn re_registering_a_known_authenticator_is_allowed_at_the_cap() {
        // Registration replaces by credential id, so the count does not
        // grow — refusing here was arithmetic about a number that was
        // never going to change, and it locked a full appliance out of
        // re-enrolling a phone it already trusted.
        //
        // The ceremony still fails on the (empty) attestation, which is
        // correct and is the point: it must fail on *verification*, not
        // on the cap.
        let mut c = Challenges::new();
        let now = Instant::now();
        let issued = c
            .issue(Ceremony::Register, "https://host.local", now)
            .unwrap();
        let existing: Vec<PasskeyRecord> = (0..MAX_CREDENTIALS)
            .map(|i| PasskeyRecord {
                credential: Credential {
                    id: format!("c{i}"),
                    public_key: "x".into(),
                    sign_count: 0,
                },
                label: "x".into(),
                created_at: chrono::Utc::now(),
                last_used: None,
                origin: "https://host.local".into(),
            })
            .collect();
        let body = RegisterResponse {
            challenge_id: issued.challenge_id,
            client_data_json: String::new(),
            attestation_object: String::new(),
        };
        let err = finish_registration(&mut c, &body, "n", &existing, now, chrono::Utc::now())
            .unwrap_err();
        assert_ne!(
            err,
            PasskeyError::TooManyCredentials,
            "the cap must not be reached before verification"
        );
    }

    fn stored(n: usize) -> Vec<PasskeyRecord> {
        (0..n)
            .map(|i| PasskeyRecord {
                credential: Credential {
                    id: format!("c{i}"),
                    public_key: "x".into(),
                    sign_count: 0,
                },
                label: "x".into(),
                created_at: chrono::Utc::now(),
                last_used: None,
                origin: "https://host.local".into(),
            })
            .collect()
    }

    #[test]
    fn a_new_credential_is_refused_at_the_cap() {
        assert!(cap_allows(&stored(MAX_CREDENTIALS - 1), "brand-new"));
        assert!(!cap_allows(&stored(MAX_CREDENTIALS), "brand-new"));
    }

    #[test]
    fn a_replacement_is_allowed_even_at_the_cap() {
        // The caller replaces by credential id, so the stored count does
        // not grow. Refusing here locked a full appliance out of
        // re-enrolling a phone it already trusted.
        assert!(cap_allows(&stored(MAX_CREDENTIALS), "c0"));
    }
}

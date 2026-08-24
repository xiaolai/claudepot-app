//! Composing password, throttle, TOTP, and session issuance into one
//! decision.
//!
//! Each ingredient is tested in its own module. This one exists because
//! the *composition* is where authentication goes wrong: the order the
//! checks run in, and what each failure is allowed to reveal.
//!
//! Four orderings are load-bearing, and three of them are the kind that
//! look like refactoring opportunities later:
//!
//! 1. **The throttle is consulted before the password is verified.**
//!    scrypt is deliberately expensive; verifying first would let an
//!    unauthenticated caller spend that cost at will, turning the KDF
//!    into a denial-of-service amplifier aimed at the owner's own
//!    machine.
//! 2. **A failed TOTP counts as a failed attempt.** Otherwise a caller
//!    who holds the password gets unlimited free guesses at six digits,
//!    which is 20 bits with no rate limit — the second factor would be
//!    weaker than the first.
//! 3. **A wrong password never reveals whether TOTP is configured.**
//!    `TotpRequired` is only ever returned once the password is known
//!    correct. Returning it earlier would tell an attacker which
//!    accounts are worth phishing a code for.
//! 4. **The password is verified even when the caller omitted a
//!    required TOTP code.** Returning `TotpRequired` without doing the
//!    scrypt work would make "is this password right?" answerable in
//!    microseconds by leaving the code out.

use chrono::{DateTime, Duration, Utc};

use super::password::{self, PasswordError, Throttle};
use super::token::{self, NewToken};
use super::totp::{TotpSecret, TotpState, TotpVerdict};
use super::Device;

/// How long a session token stays valid without being used.
///
/// Long, on purpose: the client is a phone, and the alternative to a
/// long-lived session is the user typing an admin password into a
/// handset repeatedly, which trains exactly the habit this design
/// should not train.
pub const SESSION_TTL_DAYS: i64 = 30;

/// Everything the login decision reads. Persisted by the caller.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    /// PHC string. `None` means the appliance has not been set up.
    pub password_hash: Option<String>,
    /// `None` means the second factor is off.
    pub totp_secret: Option<TotpSecret>,
    pub totp_state: TotpState,
    pub throttle: Throttle,
}

impl AuthConfig {
    pub fn is_configured(&self) -> bool {
        self.password_hash.is_some()
    }

    pub fn totp_enabled(&self) -> bool {
        self.totp_secret.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    /// The plaintext token is returned **once**; only its hash is kept.
    Success { token: String, device: Device },
    /// No password has been set. The caller should offer setup rather
    /// than a login form.
    NotConfigured,
    /// Wrong password. Says nothing about whether TOTP is on.
    Invalid,
    /// Password correct, second factor enabled, no code supplied.
    TotpRequired,
    /// Password correct, code wrong or already spent. Deliberately one
    /// variant: a caller that distinguished them would tell an attacker
    /// when they had hit a real, already-used code.
    TotpInvalid,
    /// Too many recent failures. Wait, then retry — never a permanent
    /// lockout, see `password`'s module docs.
    Throttled { wait_secs: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error(transparent)]
    Password(#[from] PasswordError),
}

/// Evaluate one login attempt, mutating `config`'s throttle and TOTP
/// counter as a side effect. The caller persists `config` afterwards —
/// **including on failure**, or the attempt counter never rises and the
/// throttle is decorative.
pub fn attempt(
    config: &mut AuthConfig,
    password_candidate: &str,
    totp_candidate: Option<&str>,
    device_label: &str,
    now: DateTime<Utc>,
) -> Result<LoginOutcome, LoginError> {
    let Some(stored) = config.password_hash.clone() else {
        return Ok(LoginOutcome::NotConfigured);
    };

    // (1) Before any expensive work.
    let wait = config.throttle.required_delay_secs(now);
    if wait > 0 {
        return Ok(LoginOutcome::Throttled { wait_secs: wait });
    }

    // (4) Always paid, even when a required code is missing.
    let password_ok = password::verify_password(password_candidate, &stored)?;
    if !password_ok {
        config.throttle.record_failure(now);
        // (3) Never leaks whether a second factor exists.
        return Ok(LoginOutcome::Invalid);
    }

    if let Some(secret) = config.totp_secret.clone() {
        let Some(code) = totp_candidate else {
            // Not a failure: the caller did everything right and simply
            // has another step. Counting it would let a client that
            // always omits the code lock its own owner out.
            return Ok(LoginOutcome::TotpRequired);
        };
        match config
            .totp_state
            .consume(&secret, code, now.timestamp().max(0) as u64)
        {
            TotpVerdict::Accepted { .. } => {}
            // (2) A bad second factor is a failed attempt.
            TotpVerdict::Wrong | TotpVerdict::Replayed { .. } => {
                config.throttle.record_failure(now);
                return Ok(LoginOutcome::TotpInvalid);
            }
        }
    }

    config.throttle.record_success();
    let issued = issue_session(device_label, now);
    Ok(LoginOutcome::Success {
        token: issued.0,
        device: issued.1,
    })
}

/// Mint a session as a `Device` row with an expiry.
///
/// A logged-in session and a paired device are the same thing — a token
/// hash that `authenticate` will accept — so they share one
/// representation rather than growing a parallel one that would need
/// its own revocation path.
pub(super) fn issue_session(label: &str, now: DateTime<Utc>) -> (String, Device) {
    let NewToken { plaintext, hash } = token::new_device_token();
    let device = Device {
        id: uuid::Uuid::new_v4(),
        name: label.to_string(),
        token_hash: hash,
        created_at: now,
        last_seen: Some(now),
        revoked_at: None,
        expires_at: Some(now + Duration::days(SESSION_TTL_DAYS)),
    };
    (plaintext, device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, h, 0, 0).unwrap()
    }

    const PW: &str = "correct horse battery";

    fn configured() -> AuthConfig {
        let mut p = PW.to_string();
        AuthConfig {
            password_hash: Some(password::hash_password(&mut p).unwrap()),
            ..Default::default()
        }
    }

    fn with_totp() -> (AuthConfig, TotpSecret) {
        let secret = TotpSecret::generate();
        let mut c = configured();
        c.totp_secret = Some(secret.clone());
        (c, secret)
    }

    fn code_now(secret: &TotpSecret, now: DateTime<Utc>) -> String {
        super::super::totp::code_at_counter(
            secret,
            super::super::totp::counter_at(now.timestamp() as u64),
        )
    }

    #[test]
    fn an_unconfigured_appliance_offers_setup_not_a_denial() {
        let mut c = AuthConfig::default();
        assert_eq!(
            attempt(&mut c, "anything", None, "phone", at(10)).unwrap(),
            LoginOutcome::NotConfigured
        );
    }

    #[test]
    fn the_right_password_issues_a_session() {
        let mut c = configured();
        let out = attempt(&mut c, PW, None, "phone", at(10)).unwrap();
        let LoginOutcome::Success { token, device } = out else {
            panic!("expected success, got {out:?}");
        };
        assert_eq!(device.name, "phone");
        assert!(device.is_active());
        assert_eq!(
            device.expires_at,
            Some(at(10) + Duration::days(SESSION_TTL_DAYS))
        );
        // The row stores a hash, never the bearer.
        assert_ne!(device.token_hash, token);
        assert!(super::super::token::verify_secret(
            &token,
            &device.token_hash
        ));
    }

    #[test]
    fn the_wrong_password_is_refused_and_counted() {
        let mut c = configured();
        assert_eq!(
            attempt(&mut c, "wrong", None, "phone", at(10)).unwrap(),
            LoginOutcome::Invalid
        );
        assert_eq!(c.throttle.consecutive_failures, 1);
    }

    #[test]
    fn a_success_clears_the_failure_count() {
        let mut c = configured();
        for _ in 0..3 {
            attempt(&mut c, "wrong", None, "phone", at(10)).unwrap();
        }
        assert_eq!(c.throttle.consecutive_failures, 3);
        attempt(&mut c, PW, None, "phone", at(10)).unwrap();
        assert_eq!(c.throttle.consecutive_failures, 0);
    }

    #[test]
    fn the_throttle_is_consulted_before_the_password_is_verified() {
        // Otherwise an unauthenticated caller can spend scrypt time at
        // will — a DoS amplifier pointed at the owner's own machine.
        let mut c = configured();
        for _ in 0..10 {
            attempt(&mut c, "wrong", None, "phone", at(10)).unwrap();
        }
        let before = c.throttle.consecutive_failures;
        // Even the CORRECT password is turned away while throttled, and
        // the attempt is not counted, because no verification happened.
        assert!(matches!(
            attempt(&mut c, PW, None, "phone", at(10)).unwrap(),
            LoginOutcome::Throttled { .. }
        ));
        assert_eq!(c.throttle.consecutive_failures, before);
    }

    #[test]
    fn a_throttled_caller_is_let_back_in_once_the_wait_has_elapsed() {
        // The other half of the test above, and the one that was
        // missing: turning the correct password away is only acceptable
        // *while* the delay is being served. Without this, "throttled"
        // and "locked out forever" are the same code path, and the
        // module docs promise the first while shipping the second.
        let mut c = configured();
        for _ in 0..10 {
            attempt(&mut c, "wrong", None, "phone", at(10)).unwrap();
        }
        let LoginOutcome::Throttled { wait_secs } =
            attempt(&mut c, PW, None, "phone", at(10)).unwrap()
        else {
            panic!("expected the backoff to be in force");
        };
        assert!(wait_secs > 0);

        // One second short: still held.
        assert!(matches!(
            attempt(
                &mut c,
                PW,
                None,
                "phone",
                at(10) + Duration::seconds(wait_secs as i64 - 1)
            )
            .unwrap(),
            LoginOutcome::Throttled { .. }
        ));

        // The wait is served — the correct password works again.
        let out = attempt(
            &mut c,
            PW,
            None,
            "phone",
            at(10) + Duration::seconds(wait_secs as i64),
        )
        .unwrap();
        assert!(
            matches!(out, LoginOutcome::Success { .. }),
            "after the delay a correct password must be accepted, got {out:?}"
        );
        assert_eq!(c.throttle.consecutive_failures, 0);
    }

    #[test]
    fn a_wrong_password_during_the_wait_does_not_count_again() {
        // The gate returns before verification, so a caller hammering
        // the endpoint cannot inflate their own backoff — and, more to
        // the point, cannot push the deadline out indefinitely for the
        // owner who is waiting it out.
        let mut c = configured();
        for _ in 0..10 {
            attempt(&mut c, "wrong", None, "phone", at(10)).unwrap();
        }
        let before = c.throttle;
        for _ in 0..50 {
            attempt(&mut c, "wrong", None, "phone", at(10)).unwrap();
        }
        assert_eq!(c.throttle, before, "throttled attempts must not accumulate");
    }

    #[test]
    fn a_wrong_password_never_reveals_whether_totp_is_enabled() {
        // The two configurations must be indistinguishable to someone
        // who does not know the password.
        let mut without = configured();
        let (mut with, _) = with_totp();
        assert_eq!(
            attempt(&mut without, "wrong", None, "phone", at(10)).unwrap(),
            attempt(&mut with, "wrong", None, "phone", at(10)).unwrap()
        );
    }

    #[test]
    fn a_correct_password_without_a_required_code_asks_for_one() {
        let (mut c, _) = with_totp();
        assert_eq!(
            attempt(&mut c, PW, None, "phone", at(10)).unwrap(),
            LoginOutcome::TotpRequired
        );
        assert_eq!(
            c.throttle.consecutive_failures, 0,
            "asking for the second step is not a failed attempt — counting \
             it would let a client that always omits the code lock its own \
             owner out"
        );
    }

    #[test]
    fn password_plus_the_right_code_succeeds() {
        let (mut c, secret) = with_totp();
        let code = code_now(&secret, at(10));
        assert!(matches!(
            attempt(&mut c, PW, Some(&code), "phone", at(10)).unwrap(),
            LoginOutcome::Success { .. }
        ));
    }

    #[test]
    fn a_wrong_code_is_a_failed_attempt() {
        // Without this the second factor is six digits with no rate
        // limit — weaker than the first.
        let (mut c, _) = with_totp();
        assert_eq!(
            attempt(&mut c, PW, Some("000000"), "phone", at(10)).unwrap(),
            LoginOutcome::TotpInvalid
        );
        assert_eq!(c.throttle.consecutive_failures, 1);
    }

    #[test]
    fn a_replayed_code_is_refused_and_counted() {
        let (mut c, secret) = with_totp();
        let code = code_now(&secret, at(10));
        assert!(matches!(
            attempt(&mut c, PW, Some(&code), "phone", at(10)).unwrap(),
            LoginOutcome::Success { .. }
        ));
        assert_eq!(
            attempt(&mut c, PW, Some(&code), "phone", at(10)).unwrap(),
            LoginOutcome::TotpInvalid,
            "a spent code must not log in twice"
        );
        assert_eq!(c.throttle.consecutive_failures, 1);
    }

    #[test]
    fn a_replay_is_not_distinguishable_from_a_wrong_code() {
        // Distinguishing them would tell an attacker they had hit a
        // real, already-used code.
        let (mut c, secret) = with_totp();
        let code = code_now(&secret, at(10));
        attempt(&mut c, PW, Some(&code), "phone", at(10)).unwrap();
        let replayed = attempt(&mut c, PW, Some(&code), "phone", at(10)).unwrap();
        let (mut c2, _) = with_totp();
        let wrong = attempt(&mut c2, PW, Some("000000"), "phone", at(10)).unwrap();
        assert_eq!(replayed, wrong);
    }

    #[test]
    fn every_session_gets_its_own_token() {
        let mut c = configured();
        let mut hashes = std::collections::HashSet::new();
        for _ in 0..20 {
            let LoginOutcome::Success { device, .. } =
                attempt(&mut c, PW, None, "phone", at(10)).unwrap()
            else {
                panic!("expected success")
            };
            assert!(
                hashes.insert(device.token_hash),
                "token reuse across logins"
            );
        }
    }
}

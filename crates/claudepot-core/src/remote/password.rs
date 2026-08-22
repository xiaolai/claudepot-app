//! The admin password for the remote surface.
//!
//! Claudepot's remote surface is an appliance: reachable on the LAN,
//! one password, and whoever holds it is admin. That model is only
//! sound if the password is treated as the entire security boundary,
//! because it is — behind it sits the ability to drive Claude Code
//! sessions, which is arbitrary code execution as this user.
//!
//! ## This reverses an earlier decision, deliberately
//!
//! `remote::token` argues *against* a memory-hard KDF, and is right to:
//! a 256-bit machine-generated token has nothing to brute force, so
//! Argon2/scrypt would buy nothing and cost per-request latency. That
//! reasoning depended entirely on there being no low-entropy secret.
//!
//! A human-chosen password is exactly the secret it excluded, so the
//! conclusion flips here. Both modules are correct for their own
//! input, and the difference between them is the input, not taste. Do
//! not "unify" them onto one hash.
//!
//! ## Two defences, against two different attacks
//!
//! - **Offline** (someone reads the stored hash): the KDF. scrypt is
//!   memory-hard, so a GPU cannot parallelise its way through a decent
//!   password.
//! - **Online** (someone guesses over the network): the throttle below.
//!   The KDF barely helps here — an attacker making one guess a second
//!   does not care that each costs 100ms.
//!
//! ## Why the throttle backs off instead of locking out
//!
//! A hard lockout on a LAN-reachable appliance is a denial-of-service
//! handed to anyone on the wifi: they lock the owner out of their own
//! machine and cannot themselves be locked out of anything. So failures
//! buy an exponentially growing *delay*, capped, and it never becomes
//! permanent. At the cap a brute-force attempt is dead, while the owner
//! who fat-fingered their password waits once and gets on with it.
//!
//! The pairing code in `remote::token` can afford to burn itself
//! because the user can always mint another from the machine. An
//! admin locked out over the network cannot.

use scrypt::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use scrypt::Scrypt;
use zeroize::Zeroize;

/// Length, not character-class theatre. Composition rules push users
/// toward `Password1!` and buy roughly nothing; length is what makes a
/// memory-hard KDF expensive to attack.
pub const MIN_PASSWORD_LEN: usize = 12;

/// Failures before the delay starts growing. The first few are almost
/// always a typo.
pub const FREE_ATTEMPTS: u32 = 3;

/// Ceiling on the backoff. At 30s per guess an online search is over
/// before it starts, and an owner who mistyped waits once.
pub const MAX_BACKOFF_SECS: u64 = 30;

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("the password must be at least {min} characters (got {got})")]
    TooShort { min: usize, got: usize },
    #[error("cannot hash the password: {0}")]
    Hash(String),
    #[error("the stored password hash is unreadable: {0}")]
    StoredHashInvalid(String),
}

/// Hash a new password into a PHC string.
///
/// The returned value carries its own algorithm, parameters and salt,
/// so changing scrypt's cost — or moving to argon2id — does not
/// invalidate passwords already set.
///
/// `plain` is zeroized before return on every path.
pub fn hash_password(plain: &mut String) -> Result<String, PasswordError> {
    let result = hash_inner(plain);
    plain.zeroize();
    result
}

fn hash_inner(plain: &str) -> Result<String, PasswordError> {
    if plain.chars().count() < MIN_PASSWORD_LEN {
        return Err(PasswordError::TooShort {
            min: MIN_PASSWORD_LEN,
            got: plain.chars().count(),
        });
    }
    let salt = SaltString::generate(&mut scrypt::password_hash::rand_core::OsRng);
    Scrypt
        .hash_password(plain.as_bytes(), &salt)
        // The error is deliberately not interpolated with the password.
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hash(e.to_string()))
}

/// Verify a candidate against a stored PHC string.
///
/// A malformed stored hash is an error, never `false`: silently
/// treating an unreadable hash as "wrong password" would lock the owner
/// out of their own appliance with no explanation, and would look
/// identical to a forgotten password.
pub fn verify_password(candidate: &str, stored: &str) -> Result<bool, PasswordError> {
    let parsed =
        PasswordHash::new(stored).map_err(|e| PasswordError::StoredHashInvalid(e.to_string()))?;

    // A PHC string can parse and still carry no hash output —
    // `$scrypt$garbage` does exactly that. Verifying it returns
    // `Error::Password`, which is indistinguishable from a wrong
    // password, so the owner would be told their correct password is
    // wrong, forever, with nothing to indicate the file was the problem.
    // Measured against scrypt 0.11; checked here rather than trusted.
    if parsed.hash.is_none() {
        return Err(PasswordError::StoredHashInvalid(
            "the stored PHC string carries no hash output".into(),
        ));
    }

    match Scrypt.verify_password(candidate.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        // The ONLY error that means "wrong password". `PasswordHash::new`
        // is lenient enough to accept structurally broken input
        // (`$scrypt$garbage` parses), so the real distinction is made
        // here. Collapsing everything to `false` with `.is_ok()` — which
        // this did first — reports an unusable hash as a failed login,
        // and the owner sees "wrong password" forever with a correct one.
        Err(scrypt::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(PasswordError::StoredHashInvalid(e.to_string())),
    }
}

/// Online-guessing throttle. Pure; the caller owns the clock and the
/// storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Throttle {
    pub consecutive_failures: u32,
}

impl Throttle {
    pub fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// Seconds a caller must wait after the most recent failure before
    /// another attempt is accepted.
    ///
    /// Doubling from 1s after the free attempts, capped. Never
    /// infinite — see the module docs on why lockout is the wrong shape
    /// for something reachable by anyone on the network.
    pub fn required_delay_secs(&self) -> u64 {
        let over = self.consecutive_failures.saturating_sub(FREE_ATTEMPTS);
        if over == 0 {
            return 0;
        }
        let shift = (over - 1).min(16);
        (1u64 << shift).min(MAX_BACKOFF_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pw(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn a_password_verifies_against_its_own_hash() {
        let stored = hash_password(&mut pw("correct horse battery")).unwrap();
        assert!(verify_password("correct horse battery", &stored).unwrap());
        assert!(!verify_password("Correct horse battery", &stored).unwrap());
        assert!(!verify_password("", &stored).unwrap());
    }

    #[test]
    fn the_stored_hash_is_phc_and_names_its_algorithm() {
        let stored = hash_password(&mut pw("correct horse battery")).unwrap();
        assert!(
            stored.starts_with("$scrypt$"),
            "stored hashes must carry their own algorithm so the cost can \
             change later without invalidating passwords: {stored}"
        );
    }

    #[test]
    fn the_same_password_hashes_differently_every_time() {
        let a = hash_password(&mut pw("correct horse battery")).unwrap();
        let b = hash_password(&mut pw("correct horse battery")).unwrap();
        assert_ne!(a, b, "a missing salt would make these identical");
        assert!(verify_password("correct horse battery", &a).unwrap());
        assert!(verify_password("correct horse battery", &b).unwrap());
    }

    #[test]
    fn the_hash_never_contains_the_password() {
        let stored = hash_password(&mut pw("correct horse battery")).unwrap();
        assert!(!stored.contains("correct"));
        assert!(!stored.contains("battery"));
    }

    #[test]
    fn the_plaintext_is_zeroized_after_hashing() {
        let mut p = pw("correct horse battery");
        let _ = hash_password(&mut p).unwrap();
        assert!(p.is_empty(), "the caller's copy must not survive the call");
    }

    #[test]
    fn the_plaintext_is_zeroized_even_when_hashing_fails() {
        let mut p = pw("short");
        assert!(hash_password(&mut p).is_err());
        assert!(p.is_empty(), "the error path must scrub too");
    }

    #[test]
    fn short_passwords_are_refused() {
        for s in ["", "a", "elevenchars"] {
            let mut p = pw(s);
            assert!(
                matches!(hash_password(&mut p), Err(PasswordError::TooShort { .. })),
                "{s:?} is under {MIN_PASSWORD_LEN}"
            );
        }
        assert!(hash_password(&mut pw("twelvechars!")).is_ok());
    }

    #[test]
    fn length_is_counted_in_characters_not_bytes() {
        // Twelve CJK characters is a fine password and 36 bytes. A byte
        // check would accept four of them, which is not twelve.
        let mut ok = pw("密码密码密码密码密码密码");
        assert_eq!(ok.chars().count(), 12);
        assert!(hash_password(&mut ok).is_ok());

        let mut short = pw("密码密码");
        assert!(matches!(
            hash_password(&mut short),
            Err(PasswordError::TooShort { got: 4, .. })
        ));
    }

    #[test]
    fn a_corrupt_stored_hash_errors_rather_than_reporting_a_wrong_password() {
        // Reporting `false` here would look exactly like a forgotten
        // password and lock the owner out with no explanation.
        for bad in ["", "not a phc string", "$scrypt$garbage"] {
            assert!(
                matches!(
                    verify_password("anything", bad),
                    Err(PasswordError::StoredHashInvalid(_))
                ),
                "{bad:?} must be an error, not a denial"
            );
        }
    }

    #[test]
    fn the_first_few_failures_cost_nothing() {
        let mut t = Throttle::default();
        for _ in 0..FREE_ATTEMPTS {
            assert_eq!(t.required_delay_secs(), 0);
            t.record_failure();
        }
        // FREE_ATTEMPTS failures are still free; the next one starts the
        // backoff.
        assert_eq!(t.required_delay_secs(), 0);
        t.record_failure();
        assert_eq!(t.required_delay_secs(), 1);
    }

    #[test]
    fn the_delay_doubles_and_then_stops() {
        let mut t = Throttle::default();
        for _ in 0..FREE_ATTEMPTS {
            t.record_failure();
        }
        let mut seen = Vec::new();
        for _ in 0..12 {
            t.record_failure();
            seen.push(t.required_delay_secs());
        }
        assert_eq!(&seen[..6], &[1, 2, 4, 8, 16, MAX_BACKOFF_SECS]);
        assert!(seen.iter().all(|d| *d <= MAX_BACKOFF_SECS));
    }

    #[test]
    fn the_throttle_never_becomes_permanent() {
        // A hard lockout on a LAN appliance is a DoS anyone on the wifi
        // can perform against the owner. The delay must stay finite.
        let mut t = Throttle::default();
        for _ in 0..100_000 {
            t.record_failure();
        }
        assert_eq!(t.required_delay_secs(), MAX_BACKOFF_SECS);
    }

    #[test]
    fn a_success_clears_the_backoff() {
        let mut t = Throttle::default();
        for _ in 0..10 {
            t.record_failure();
        }
        assert!(t.required_delay_secs() > 0);
        t.record_success();
        assert_eq!(t.required_delay_secs(), 0);
    }
}

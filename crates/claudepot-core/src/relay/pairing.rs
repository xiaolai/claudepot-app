//! The relay's pairing code: one at a time, eight characters, five
//! minutes, five wrong guesses and it is gone.
//!
//! The code reuses remote control's *generator* (`remote::token`) and
//! comparison (`remote::ct`), never its store: a relay code redeems only a
//! relay device token, and a remote code is never held here. The threat
//! model is the one `remote::token` documents — a low-entropy secret a
//! human retypes, defended by expiry and the attempt cap, not by entropy.

use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::remote::ct::constant_time_eq;
use crate::remote::token::{new_pairing_code, CODE_LEN, MAX_CODE_ATTEMPTS};

pub const CODE_TTL: Duration = Duration::from_secs(5 * 60);

struct Pending {
    code: String,
    expires_at: Instant,
    attempts: u32,
    include_future: bool,
}

/// The code to show the user, once.
pub struct Offered {
    pub code: String,
    pub expires_in: Duration,
}

/// What a redeemed code authorizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Redeemed {
    pub include_future: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PairError {
    /// No code is open, it expired, or it was burned by wrong guesses.
    #[error("pairing_unavailable")]
    Unavailable,
    #[error("pairing_code_invalid")]
    Wrong,
}

#[derive(Default)]
pub struct Pairing(Mutex<Option<Pending>>);

impl Pairing {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a fresh code, replacing any open one.
    pub fn offer(&self, include_future: bool, now: Instant) -> Offered {
        let code = new_pairing_code();
        *self.0.lock() = Some(Pending {
            code: code.clone(),
            expires_at: now + CODE_TTL,
            attempts: 0,
            include_future,
        });
        Offered {
            code,
            expires_in: CODE_TTL,
        }
    }

    pub fn cancel(&self) {
        *self.0.lock() = None;
    }

    pub fn is_open(&self, now: Instant) -> bool {
        self.0.lock().as_ref().is_some_and(|p| now < p.expires_at)
    }

    /// Single use: a match consumes the code. A miss counts, and the
    /// fifth miss destroys it.
    pub fn redeem(&self, presented: &str, now: Instant) -> Result<Redeemed, PairError> {
        let mut slot = self.0.lock();
        let Some(p) = slot.as_mut() else {
            return Err(PairError::Unavailable);
        };
        if now >= p.expires_at {
            *slot = None;
            return Err(PairError::Unavailable);
        }
        let normalized: String = presented
            .trim()
            .chars()
            .filter(|c| *c != '-' && *c != ' ')
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let ok = normalized.len() == CODE_LEN
            && constant_time_eq(normalized.as_bytes(), p.code.as_bytes());
        if ok {
            let include_future = p.include_future;
            *slot = None;
            return Ok(Redeemed { include_future });
        }
        p.attempts += 1;
        if p.attempts >= MAX_CODE_ATTEMPTS {
            *slot = None;
        }
        Err(PairError::Wrong)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_redeems_once() {
        let p = Pairing::new();
        let now = Instant::now();
        let o = p.offer(true, now);
        assert_eq!(
            p.redeem(&o.code.to_lowercase(), now),
            Ok(Redeemed {
                include_future: true
            })
        );
        assert_eq!(p.redeem(&o.code, now), Err(PairError::Unavailable));
    }

    #[test]
    fn the_fifth_wrong_guess_burns_the_code() {
        let p = Pairing::new();
        let now = Instant::now();
        let o = p.offer(false, now);
        for _ in 0..MAX_CODE_ATTEMPTS {
            assert_eq!(p.redeem("XXXXXXXX", now), Err(PairError::Wrong));
        }
        assert_eq!(p.redeem(&o.code, now), Err(PairError::Unavailable));
    }

    #[test]
    fn an_expired_code_is_unavailable() {
        let p = Pairing::new();
        let now = Instant::now();
        let o = p.offer(false, now);
        assert!(p.is_open(now));
        let later = now + CODE_TTL;
        assert!(!p.is_open(later));
        assert_eq!(p.redeem(&o.code, later), Err(PairError::Unavailable));
    }

    #[test]
    fn a_new_offer_replaces_the_old_code() {
        let p = Pairing::new();
        let now = Instant::now();
        let first = p.offer(false, now);
        let second = p.offer(false, now);
        if first.code != second.code {
            assert_eq!(p.redeem(&first.code, now), Err(PairError::Wrong));
        }
        assert!(p.redeem(&second.code, now).is_ok());
    }
}

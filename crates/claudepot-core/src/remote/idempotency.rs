//! Making a retried mutation safe to repeat.
//!
//! A phone loses signal mid-request and the client retries. Without
//! this, "send a prompt" runs twice and the session gets it twice —
//! which for a surface that drives Claude Code means the work happens
//! twice, not merely that a counter is wrong.
//!
//! The client sends `Idempotency-Key: <opaque>`; the first request with
//! a given key executes and its response is remembered, and any repeat
//! within the window returns that stored response without re-running
//! anything.
//!
//! ## Reserving, not just remembering
//!
//! Remembering the *response* is not enough on its own. The window
//! between "this key has not been seen" and "here is what it produced"
//! is exactly as long as the mutation takes — a socket round trip to
//! Claude Code — and two requests carrying the same key can both land
//! inside it. Both would see `Execute` and both would send the prompt,
//! which is the one outcome this module exists to prevent.
//!
//! So [`Lookup::Execute`] **reserves** the key. A concurrent duplicate
//! gets [`Lookup::InFlight`] and is told to retry rather than being
//! silently executed or silently dropped; a caller that finishes calls
//! [`Idempotency::remember`], which turns the reservation into a stored
//! response.
//!
//! A reservation whose caller never finishes — the client disconnected
//! and axum dropped the future — is reclaimed by [`RESERVATION_TTL`],
//! which is deliberately **shorter** than the response TTL and not the
//! same number. A stuck key nobody can retry is a worse failure than the
//! double-send, so the reservation has to expire; but expiring it while
//! the first caller is still working would reopen the race. The window
//! is therefore sized to exceed any handler on this surface, and the
//! trade is stated rather than hidden: a mutation that somehow ran
//! longer than [`RESERVATION_TTL`] could be executed twice by a
//! determined retry. Nothing here can — a peer send is bounded by its
//! socket timeout and a read mark is one file write.
//!
//! ## In memory, and that is not a shortcut
//!
//! A retry happens within seconds of the original. Persisting these
//! would mean a disk write on every mutation to defend a window that
//! closes before the disk write matters, and would need its own pruning
//! and its own corruption story. A process restart between a request
//! and its retry is indistinguishable from a request that never
//! arrived — which is the case the client must already handle.
//!
//! ## Bounded, because the key is client-supplied
//!
//! Anything a caller controls and the server accumulates is a memory
//! exhaustion primitive. Entries expire, and the map is capped: past
//! the cap the oldest go first, so a flood of junk keys costs bounded
//! memory and at worst makes a *legitimate* retry re-execute — the
//! behaviour we had before this module, not something worse.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a completed response stays replayable. Long enough for a
/// phone changing networks, short enough to be irrelevant to anything
/// deliberate.
pub const TTL: Duration = Duration::from_secs(300);

/// Maximum remembered responses.
pub const MAX_ENTRIES: usize = 512;

/// Maximum accepted key length. A key is an opaque client string; there
/// is no reason for it to be large, and refusing early keeps a big body
/// out of the map.
pub const MAX_KEY_LEN: usize = 200;

/// How long an unfinished reservation is believed.
///
/// Its own constant, not [`TTL`]. They answer different questions: `TTL`
/// is "how long may a client retry and get the same answer", which wants
/// to be generous; this is "how long do we assume the first caller is
/// still working", which wants to be just longer than the slowest
/// handler. Reusing one number for both would make an abandoned key
/// unusable for five minutes *and* would tie the race window to a value
/// chosen for a different reason.
pub const RESERVATION_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub status: u16,
    pub body: String,
}

struct Entry {
    /// `None` while the first caller is still running.
    stored: Option<Stored>,
    at: Instant,
}

#[derive(Default)]
pub struct Idempotency {
    entries: HashMap<String, Entry>,
}

/// What the caller should do with a request carrying a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// No prior request. The key is now **reserved** — execute, then
    /// call [`Idempotency::remember`].
    Execute,
    /// A prior response exists. Return it and execute nothing.
    Replay(Stored),
    /// A request with this key is running right now. Execute nothing and
    /// tell the client to retry: the first one may still succeed, so
    /// neither replaying an answer we do not have nor running the
    /// mutation a second time is correct.
    InFlight,
    /// The key itself is unusable.
    Rejected(&'static str),
}

impl Idempotency {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lookup(&mut self, key: &str, now: Instant) -> Lookup {
        if key.is_empty() {
            return Lookup::Rejected("empty idempotency key");
        }
        if key.len() > MAX_KEY_LEN {
            return Lookup::Rejected("idempotency key too long");
        }
        self.expire(now);
        match self.entries.get(key) {
            Some(e) => match &e.stored {
                Some(stored) => Lookup::Replay(stored.clone()),
                None => Lookup::InFlight,
            },
            None => {
                // Reserve before returning, while the caller still holds
                // whatever lock brought it here. Returning `Execute`
                // without reserving is the race this module exists to
                // close.
                if self.entries.len() >= MAX_ENTRIES {
                    self.evict_oldest();
                }
                self.entries.insert(
                    key.to_string(),
                    Entry {
                        stored: None,
                        at: now,
                    },
                );
                Lookup::Execute
            }
        }
    }

    pub fn remember(&mut self, key: &str, stored: Stored, now: Instant) {
        if key.is_empty() || key.len() > MAX_KEY_LEN {
            return;
        }
        self.expire(now);
        if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(key) {
            self.evict_oldest();
        }
        self.entries.insert(
            key.to_string(),
            Entry {
                stored: Some(stored),
                at: now,
            },
        );
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn expire(&mut self, now: Instant) {
        self.entries.retain(|_, e| {
            let limit = if e.stored.is_some() {
                TTL
            } else {
                RESERVATION_TTL
            };
            now.duration_since(e.at) < limit
        });
    }

    fn evict_oldest(&mut self) {
        if let Some(k) = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.at)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_concurrent_duplicate_is_told_to_wait_not_run_again() {
        // The race this closes: `lookup` used to return `Execute`
        // without recording anything, so two requests carrying one key
        // could both be told to execute during the seconds a prompt
        // takes to reach Claude Code — and the session got it twice.
        let mut i = Idempotency::new();
        let now = Instant::now();
        assert_eq!(i.lookup("k", now), Lookup::Execute);
        assert_eq!(
            i.lookup("k", now),
            Lookup::InFlight,
            "the second caller must not be told to execute"
        );
    }

    #[test]
    fn a_reservation_becomes_a_replay_once_the_response_lands() {
        let mut i = Idempotency::new();
        let now = Instant::now();
        assert_eq!(i.lookup("k", now), Lookup::Execute);
        let stored = Stored {
            status: 202,
            body: "{}".into(),
        };
        i.remember("k", stored.clone(), now);
        assert_eq!(i.lookup("k", now), Lookup::Replay(stored));
    }

    #[test]
    fn a_reservation_whose_caller_died_is_reclaimed_on_its_own_clock() {
        // A key stuck reserved forever is a worse failure than the
        // double-send: the client can never retry and never find out
        // why. It is reclaimed on RESERVATION_TTL, which is shorter than
        // the response TTL — an abandoned key must not sit unusable for
        // as long as a *successful* one stays replayable.
        let mut i = Idempotency::new();
        let now = Instant::now();
        assert_eq!(i.lookup("k", now), Lookup::Execute);
        assert_eq!(
            i.lookup("k", now + RESERVATION_TTL - Duration::from_secs(1)),
            Lookup::InFlight,
            "the first caller is still believed to be working"
        );
        assert_eq!(
            i.lookup("k", now + RESERVATION_TTL + Duration::from_secs(1)),
            Lookup::Execute
        );
    }

    #[test]
    fn a_stored_response_outlives_a_reservation() {
        // The two clocks are separate on purpose; this is the assertion
        // that says so.
        assert!(RESERVATION_TTL < TTL);
        let mut i = Idempotency::new();
        let now = Instant::now();
        i.lookup("k", now);
        i.remember("k", stored(202), now);
        let later = now + RESERVATION_TTL + Duration::from_secs(1);
        assert!(
            matches!(i.lookup("k", later), Lookup::Replay(_)),
            "a finished response must not expire on the reservation clock"
        );
    }

    #[test]
    fn an_unusable_key_reserves_nothing() {
        let mut i = Idempotency::new();
        let now = Instant::now();
        assert!(matches!(i.lookup("", now), Lookup::Rejected(_)));
        assert!(matches!(
            i.lookup(&"x".repeat(MAX_KEY_LEN + 1), now),
            Lookup::Rejected(_)
        ));
        assert!(i.is_empty(), "a refused key must not occupy an entry");
    }

    fn stored(n: u16) -> Stored {
        Stored {
            status: n,
            body: format!("{{\"n\":{n}}}"),
        }
    }

    #[test]
    fn a_first_request_executes() {
        let mut i = Idempotency::new();
        assert_eq!(i.lookup("k1", Instant::now()), Lookup::Execute);
    }

    #[test]
    fn a_repeat_replays_instead_of_executing() {
        // The whole point: a retried prompt must not reach the session
        // a second time.
        let mut i = Idempotency::new();
        let now = Instant::now();
        i.remember("k1", stored(200), now);
        assert_eq!(i.lookup("k1", now), Lookup::Replay(stored(200)));
    }

    #[test]
    fn different_keys_do_not_collide() {
        let mut i = Idempotency::new();
        let now = Instant::now();
        i.remember("k1", stored(200), now);
        assert_eq!(i.lookup("k2", now), Lookup::Execute);
    }

    #[test]
    fn an_entry_expires() {
        let mut i = Idempotency::new();
        let now = Instant::now();
        i.remember("k1", stored(200), now);
        i.remember("k2", stored(200), now);
        let later = now + TTL + Duration::from_secs(1);
        assert_eq!(i.lookup("k1", later), Lookup::Execute);
        // One entry, not three: the expired responses were dropped
        // rather than merely ignored, and what remains is the fresh
        // reservation `lookup` just took. Asserting `is_empty` here
        // would now be asserting that `lookup` does not reserve.
        assert_eq!(i.len(), 1, "expired entries are dropped, not just ignored");
    }

    #[test]
    fn an_entry_survives_up_to_the_ttl() {
        let mut i = Idempotency::new();
        let now = Instant::now();
        i.remember("k1", stored(200), now);
        let almost = now + TTL - Duration::from_secs(1);
        assert_eq!(i.lookup("k1", almost), Lookup::Replay(stored(200)));
    }

    #[test]
    fn an_empty_or_oversized_key_is_rejected() {
        let mut i = Idempotency::new();
        let now = Instant::now();
        assert!(matches!(i.lookup("", now), Lookup::Rejected(_)));
        let long = "x".repeat(MAX_KEY_LEN + 1);
        assert!(matches!(i.lookup(&long, now), Lookup::Rejected(_)));
        // And a rejected key must not be storable either, or the cap
        // could be bypassed by remembering what lookup refuses.
        i.remember(&long, stored(200), now);
        assert!(i.is_empty());
    }

    #[test]
    fn the_map_is_capped_against_a_client_supplied_key() {
        // A client controls the key, so an unbounded map is a memory
        // exhaustion primitive.
        let mut i = Idempotency::new();
        let now = Instant::now();
        for n in 0..(MAX_ENTRIES * 2) {
            i.remember(&format!("k{n}"), stored(200), now);
        }
        assert!(i.len() <= MAX_ENTRIES, "got {}", i.len());
    }

    #[test]
    fn eviction_takes_the_oldest_first() {
        let mut i = Idempotency::new();
        let t0 = Instant::now();
        i.remember("oldest", stored(1), t0);
        for n in 0..MAX_ENTRIES {
            i.remember(&format!("k{n}"), stored(2), t0 + Duration::from_millis(1));
        }
        assert_eq!(
            i.lookup("oldest", t0 + Duration::from_millis(2)),
            Lookup::Execute,
            "the oldest entry should have been evicted first"
        );
    }

    #[test]
    fn re_remembering_the_same_key_does_not_grow_the_map() {
        let mut i = Idempotency::new();
        let now = Instant::now();
        for _ in 0..10 {
            i.remember("k1", stored(200), now);
        }
        assert_eq!(i.len(), 1);
    }
}

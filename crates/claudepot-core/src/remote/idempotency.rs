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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub status: u16,
    pub body: String,
}

struct Entry {
    stored: Stored,
    at: Instant,
}

#[derive(Default)]
pub struct Idempotency {
    entries: HashMap<String, Entry>,
}

/// What the caller should do with a request carrying a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// No prior response. Execute, then call [`Idempotency::remember`].
    Execute,
    /// A prior response exists. Return it and execute nothing.
    Replay(Stored),
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
            Some(e) => Lookup::Replay(e.stored.clone()),
            None => Lookup::Execute,
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
        self.entries
            .insert(key.to_string(), Entry { stored, at: now });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn expire(&mut self, now: Instant) {
        self.entries.retain(|_, e| now.duration_since(e.at) < TTL);
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
        let later = now + TTL + Duration::from_secs(1);
        assert_eq!(i.lookup("k1", later), Lookup::Execute);
        assert!(
            i.is_empty(),
            "expired entries are dropped, not just ignored"
        );
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

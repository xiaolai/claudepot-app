//! Provider state that a later turn must send back: today, Gemini's
//! per-call `extra_content`.
//!
//! **Memory only, bounded, scoped** (plan §9, Q4). An entry is keyed by
//! device, target fingerprint, upstream model and call id, so one
//! device's conversation can never supply another's signature, and a
//! target change (which bumps the fingerprint) starts clean (reviewed
//! E9). Nothing is written to disk: the state is a provider's private
//! reasoning token, and losing it costs one refused turn —
//! `continuation_state_lost` — which CC shows and the user recovers from
//! by starting a new conversation. A relay restart loses it by design.
//!
//! Bounds: an entry over [`Limits::per_entry`] is not stored; past
//! [`Limits::total`] bytes or [`Limits::entries`] entries the oldest goes
//! first; an entry older than [`Limits::retention`] is gone.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub device_id: String,
    pub fingerprint: String,
    pub model: String,
    pub call_id: String,
}

impl Key {
    fn size(&self) -> usize {
        self.device_id.len() + self.fingerprint.len() + self.model.len() + self.call_id.len()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub per_entry: usize,
    pub total: usize,
    pub entries: usize,
    pub retention: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_entry: super::MAX_EXTRA_CONTENT,
            total: 64 * 1024 * 1024,
            entries: 65_536,
            retention: Duration::from_secs(7 * 24 * 60 * 60),
        }
    }
}

struct Entry {
    value: Value,
    size: usize,
    at: Instant,
    seq: u64,
}

#[derive(Default)]
struct Inner {
    map: HashMap<Key, Entry>,
    /// Insertion order; a sequence number that no longer matches the
    /// map's entry is a replaced value, skipped on eviction.
    order: VecDeque<(u64, Key)>,
    total: usize,
    seq: u64,
}

impl Inner {
    fn remove(&mut self, key: &Key) {
        if let Some(e) = self.map.remove(key) {
            self.total -= e.size;
        }
    }

    fn evict_front(&mut self) {
        if let Some((seq, key)) = self.order.pop_front() {
            if self.map.get(&key).is_some_and(|e| e.seq == seq) {
                self.remove(&key);
            }
        }
    }
}

#[derive(Default)]
pub struct ContinuationStore {
    limits: Limits,
    inner: parking_lot::Mutex<Inner>,
}

impl ContinuationStore {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            inner: parking_lot::Mutex::default(),
        }
    }

    pub fn put(&self, key: Key, value: Value) -> bool {
        self.put_at(key, value, Instant::now())
    }

    pub fn get(&self, key: &Key) -> Option<Value> {
        self.get_at(key, Instant::now())
    }

    /// Store `value`; `false` when it is over the per-entry cap and was
    /// not kept (the next replay of that call is then refused). A value
    /// already held for `key` goes either way: an oversized replacement
    /// must not leave the old metadata to be replayed in its place.
    pub fn put_at(&self, key: Key, value: Value, now: Instant) -> bool {
        let size = key.size() + serde_json::to_vec(&value).map_or(usize::MAX, |v| v.len());
        let mut g = self.inner.lock();
        g.remove(&key);
        if size > self.limits.per_entry {
            return false;
        }
        g.seq += 1;
        let seq = g.seq;
        g.total += size;
        g.order.push_back((seq, key.clone()));
        g.map.insert(
            key,
            Entry {
                value,
                size,
                at: now,
                seq,
            },
        );
        while g.total > self.limits.total || g.map.len() > self.limits.entries {
            g.evict_front();
        }
        // Expired entries at the front go too, so retention also bounds
        // memory without a sweeper.
        while let Some((seq, key)) = g.order.front().cloned() {
            let stale = match g.map.get(&key) {
                Some(e) if e.seq == seq => now.duration_since(e.at) > self.limits.retention,
                _ => true,
            };
            if !stale {
                break;
            }
            g.evict_front();
        }
        // Replaced values leave dead order entries behind a live front;
        // compact before they outgrow the map.
        if g.order.len() > 2 * g.map.len() + 64 {
            let Inner { map, order, .. } = &mut *g;
            order.retain(|(seq, key)| map.get(key).is_some_and(|e| e.seq == *seq));
        }
        true
    }

    pub fn get_at(&self, key: &Key, now: Instant) -> Option<Value> {
        let mut g = self.inner.lock();
        let expired = now.duration_since(g.map.get(key)?.at) > self.limits.retention;
        if expired {
            g.remove(key);
            return None;
        }
        g.map.get(key).map(|e| e.value.clone())
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> usize {
        self.inner.lock().total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(device: &str, call: &str) -> Key {
        Key {
            device_id: device.into(),
            fingerprint: "fp".into(),
            model: "gemini".into(),
            call_id: call.into(),
        }
    }

    #[test]
    fn an_entry_is_scoped_to_device_target_model_and_call() {
        let s = ContinuationStore::default();
        assert!(s.put(key("d1", "c1"), json!({"sig": 1})));
        assert_eq!(s.get(&key("d1", "c1")), Some(json!({"sig": 1})));
        assert_eq!(s.get(&key("d2", "c1")), None);
        let mut other_model = key("d1", "c1");
        other_model.model = "other".into();
        assert_eq!(s.get(&other_model), None);
        let mut other_target = key("d1", "c1");
        other_target.fingerprint = "fp2".into();
        assert_eq!(s.get(&other_target), None);
    }

    #[test]
    fn an_oversized_entry_is_not_stored() {
        let s = ContinuationStore::new(Limits {
            per_entry: 64,
            ..Limits::default()
        });
        assert!(!s.put(key("d", "c"), json!({"sig": "x".repeat(100)})));
        assert!(s.is_empty());
    }

    #[test]
    fn an_oversized_replacement_removes_the_old_entry() {
        let s = ContinuationStore::new(Limits {
            per_entry: 64,
            ..Limits::default()
        });
        assert!(s.put(key("d", "c"), json!({"sig": 1})));
        assert!(!s.put(key("d", "c"), json!({"sig": "x".repeat(100)})));
        assert_eq!(s.get(&key("d", "c")), None, "no stale metadata to replay");
        assert_eq!(s.bytes(), 0);
    }

    #[test]
    fn the_oldest_entry_goes_first_when_the_total_is_reached() {
        let s = ContinuationStore::new(Limits {
            per_entry: 1024,
            total: 100,
            ..Limits::default()
        });
        for i in 0..10 {
            s.put(key("d", &format!("c{i}")), json!("0123456789"));
        }
        assert!(s.bytes() <= 100);
        assert_eq!(s.get(&key("d", "c0")), None);
        assert!(s.get(&key("d", "c9")).is_some());
    }

    #[test]
    fn replacing_a_value_counts_its_bytes_once() {
        let s = ContinuationStore::default();
        for _ in 0..1000 {
            s.put(key("d", "c"), json!("same"));
        }
        assert_eq!(s.len(), 1);
        assert_eq!(s.bytes(), key("d", "c").size() + 6);
    }

    #[test]
    fn replacing_one_value_behind_a_live_entry_keeps_the_order_bounded() {
        let s = ContinuationStore::default();
        s.put(key("d", "first"), json!(1));
        for i in 0..10_000 {
            s.put(key("d", "c"), json!(i));
        }
        assert_eq!(s.len(), 2);
        assert!(s.inner.lock().order.len() <= 2 * 2 + 64 + 1);
    }

    #[test]
    fn the_entry_count_is_capped() {
        let s = ContinuationStore::new(Limits {
            entries: 3,
            ..Limits::default()
        });
        for i in 0..10 {
            s.put(key("d", &format!("c{i}")), json!(1));
        }
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn an_entry_past_retention_is_gone() {
        let s = ContinuationStore::new(Limits {
            retention: Duration::from_secs(60),
            ..Limits::default()
        });
        let t0 = Instant::now();
        s.put_at(key("d", "c"), json!(1), t0);
        assert!(s
            .get_at(&key("d", "c"), t0 + Duration::from_secs(59))
            .is_some());
        assert!(s
            .get_at(&key("d", "c"), t0 + Duration::from_secs(61))
            .is_none());
        assert!(s.is_empty());
    }
}

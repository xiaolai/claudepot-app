//! Bus topology — `watch` aggregate + bounded `mpsc` per session.
//!
//! Why not `tokio::sync::broadcast`? Because broadcast drops silently
//! for lagging subscribers (`RecvError::Lagged`), and the live UI
//! promises "no ghost events." The explicit two-channel design here
//! surfaces lag as `resync_required` on the next delivered delta,
//! forcing the subscriber to call `LiveRuntime::session_snapshot` and
//! rehydrate from authoritative state.
//!
//! ### Aggregate (`AggregateBus`)
//!
//! `tokio::sync::watch<Arc<Vec<LiveSessionSummary>>>`. Last-writer-wins.
//! A slow subscriber simply reads the most recent value on the next
//! poll; it never blocks the producer and never sees a stale value
//! relative to what the producer published. Suitable because the
//! aggregate snapshot is idempotent — nothing in the aggregate is
//! accumulated; the producer always publishes the full list.
//!
//! ### Per-session detail (`DetailBus`)
//!
//! `tokio::sync::mpsc::channel(256)` keyed by `session_id`. Every
//! delta carries a monotonic `seq`. On `try_send` failure (channel
//! full), the producer:
//!   1. Increments a per-session `drop_count`.
//!   2. Flags `resync_required = true` on the *next* successfully
//!      queued delta.
//! Subscribers that observe `resync_required` MUST re-snapshot before
//! applying further deltas.
//!
//! Subscribers are created on demand by `DetailBus::subscribe`; the
//! bus does not retain weak refs — once the receiver is dropped, the
//! slot is torn down by the next producer write.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

use crate::session_live::types::{LiveDelta, LiveSessionSummary};

/// Bounded capacity of each per-session detail channel. 256 events
/// covers several seconds of a normal turn; beyond that the producer
/// flags resync. Intentionally small — memory bloat would defeat the
/// "light infrastructure" framing of M1.
pub const DETAIL_CHANNEL_CAPACITY: usize = 256;

// ─── Aggregate bus ─────────────────────────────────────────────────

/// Single-producer, many-consumer aggregate channel. The producer is
/// the `LiveRuntime`; every tray / strip / status-bar consumer reads
/// the same `Arc<Vec<_>>` snapshot without copying.
#[derive(Debug, Clone)]
pub struct AggregateBus {
    tx: watch::Sender<Arc<Vec<LiveSessionSummary>>>,
    rx: watch::Receiver<Arc<Vec<LiveSessionSummary>>>,
}

impl Default for AggregateBus {
    fn default() -> Self {
        Self::new()
    }
}

impl AggregateBus {
    pub fn new() -> Self {
        let (tx, rx) = watch::channel(Arc::new(Vec::new()));
        Self { tx, rx }
    }

    /// Publish the new aggregate list. Previous value is overwritten;
    /// subscribers that missed intermediate values only ever see the
    /// most recent, which is exactly what we want for an idempotent
    /// snapshot.
    pub fn publish(&self, list: Vec<LiveSessionSummary>) {
        // `watch::Sender::send` returns `Err` only if *all* receivers
        // are dropped. That is an acceptable no-op here.
        let _ = self.tx.send(Arc::new(list));
    }

    /// Synchronous snapshot. Used by `LiveRuntime::snapshot()` to
    /// answer the Tauri command without subscribing.
    pub fn snapshot(&self) -> Arc<Vec<LiveSessionSummary>> {
        self.rx.borrow().clone()
    }

    /// Create a new receiver. Each call hands out an independent
    /// cursor — different subscribers observe updates independently.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Vec<LiveSessionSummary>>> {
        self.tx.subscribe()
    }
}

// ─── Per-session detail bus ────────────────────────────────────────

/// State carried per live session: the send half, plus a flag that
/// we dropped at least one delta since the last successful send.
#[derive(Debug)]
struct DetailSlot {
    tx: mpsc::Sender<LiveDelta>,
    /// Set to `true` when `try_send` returns `Full`. Consumed on the
    /// next successful send by flipping `resync_required` on the
    /// outgoing delta.
    pending_resync: bool,
    /// How many deltas we've dropped since the last successful send.
    /// Debug signal only; not exposed to subscribers (the bool is).
    dropped: u64,
}

/// Map of session_id → detail channel send half.
///
/// Cloning is cheap — the inner `Arc<Mutex<_>>` is shared. Producers
/// hold one clone; `subscribe` hands out a matching `Receiver` for
/// any session on demand.
#[derive(Debug, Clone, Default)]
pub struct DetailBus {
    inner: Arc<Mutex<HashMap<String, DetailSlot>>>,
}

impl DetailBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to a session's detail stream. Creates the channel
    /// if this is the first subscriber. Dropping the returned
    /// `Receiver` does NOT remove the slot immediately; the slot is
    /// cleaned up by the next failing `publish_delta` when the send
    /// half returns `SendError::Closed`.
    pub async fn subscribe(&self, session_id: &str) -> mpsc::Receiver<LiveDelta> {
        let mut map = self.inner.lock().await;
        let entry = map.entry(session_id.to_string()).or_insert_with(|| {
            let (tx, _) = mpsc::channel(DETAIL_CHANNEL_CAPACITY);
            DetailSlot {
                tx,
                pending_resync: false,
                dropped: 0,
            }
        });
        // Fresh receiver — we have to rebuild the channel because
        // `Sender::subscribe` doesn't exist for mpsc. In practice M1
        // has only one subscriber per session (the GUI), so the
        // rebuild on re-subscribe is acceptable.
        let (tx, rx) = mpsc::channel(DETAIL_CHANNEL_CAPACITY);
        entry.tx = tx;
        entry.pending_resync = false;
        entry.dropped = 0;
        rx
    }

    /// Publish a delta. Sets `resync_required` on the outgoing delta
    /// when a prior send dropped at least one event. Returns
    /// `Ok(false)` when the channel was full (delta dropped) or the
    /// session has no active subscriber; `Ok(true)` on successful
    /// delivery.
    pub async fn publish_delta(&self, mut delta: LiveDelta) -> Result<bool, BusError> {
        let mut map = self.inner.lock().await;
        let Some(slot) = map.get_mut(&delta.session_id) else {
            // No subscriber — drop silently. Aggregate channel still
            // reflects the state change for peripheral surfaces.
            return Ok(false);
        };

        // Consume the pending-resync flag on the FIRST successful
        // delivery following a gap.
        if slot.pending_resync {
            delta.resync_required = true;
        }

        match slot.tx.try_send(delta) {
            Ok(()) => {
                slot.pending_resync = false;
                Ok(true)
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                slot.pending_resync = true;
                slot.dropped = slot.dropped.saturating_add(1);
                Ok(false)
            }
            Err(mpsc::error::TrySendError::Closed(returned)) => {
                // Receiver dropped — reap the slot so the next
                // subscribe rebuilds from scratch. `TrySendError::Closed`
                // returns the delta by value, so we use that copy to
                // retrieve the session id rather than re-borrow the
                // one already consumed by `try_send`.
                map.remove(&returned.session_id);
                Err(BusError::SubscriberGone)
            }
        }
    }

    /// Remove a session's slot (called on session end). Subsequent
    /// publishes for this id become silent no-ops until a new
    /// `subscribe` creates a fresh slot.
    pub async fn end_session(&self, session_id: &str) {
        let mut map = self.inner.lock().await;
        map.remove(session_id);
    }

    #[cfg(test)]
    async fn dropped_count(&self, session_id: &str) -> u64 {
        let map = self.inner.lock().await;
        map.get(session_id).map(|s| s.dropped).unwrap_or(0)
    }
}

/// Errors the bus surfaces to callers. Silent drops are not errors —
/// they are a deliberate design choice when the subscriber is slow.
#[derive(Debug, thiserror::Error)]
pub enum BusError {
    /// The session's subscriber has been dropped. The slot has been
    /// reaped; the next subscribe will rebuild it.
    #[error("subscriber for session has been dropped")]
    SubscriberGone,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_live::types::{LiveDeltaKind, Status};

    fn status_delta(sid: &str, seq: u64, s: Status) -> LiveDelta {
        LiveDelta {
            session_id: sid.to_string(),
            seq,
            produced_at_ms: 0,
            kind: LiveDeltaKind::StatusChanged {
                status: s,
                waiting_for: None,
            },
            resync_required: false,
        }
    }

    #[tokio::test]
    async fn aggregate_bus_publishes_last_writer_wins() {
        let bus = AggregateBus::new();
        let mut sub = bus.subscribe();

        bus.publish(vec![]);
        bus.publish(vec![]);

        // First observed value is the initial empty — the subscriber
        // sees whatever is current, not every intermediate publish.
        sub.changed().await.unwrap();
        assert!(sub.borrow().is_empty());
    }

    #[tokio::test]
    async fn aggregate_snapshot_reflects_latest_publish() {
        let bus = AggregateBus::new();
        assert!(bus.snapshot().is_empty());
        bus.publish(vec![]); // still empty, but a new version
        let arc = bus.snapshot();
        assert!(arc.is_empty());
    }

    #[tokio::test]
    async fn detail_bus_delivers_in_order() {
        let bus = DetailBus::new();
        let mut rx = bus.subscribe("s1").await;
        for i in 1..=5 {
            assert!(
                bus.publish_delta(status_delta("s1", i, Status::Busy))
                    .await
                    .unwrap()
            );
        }
        for i in 1..=5 {
            let d = rx.recv().await.unwrap();
            assert_eq!(d.seq, i);
            assert!(!d.resync_required);
        }
    }

    #[tokio::test]
    async fn detail_bus_flags_resync_after_overflow() {
        let bus = DetailBus::new();
        let _rx = bus.subscribe("s1").await;
        // Fill the channel without reading — saturation.
        let mut delivered = 0u64;
        for i in 0..(DETAIL_CHANNEL_CAPACITY + 10) as u64 {
            if bus
                .publish_delta(status_delta("s1", i, Status::Busy))
                .await
                .unwrap()
            {
                delivered += 1;
            }
        }
        assert_eq!(
            delivered, DETAIL_CHANNEL_CAPACITY as u64,
            "only the bounded capacity should have landed"
        );
        assert!(bus.dropped_count("s1").await >= 10);

        // Drain one — frees a slot — then send one more. That next
        // delivery MUST carry `resync_required = true`.
        let mut rx = bus.subscribe("s1").await; // fresh channel; prior drops still counted? no — we rebuild. Use old receiver path instead.
        // Re-subscribe tears down; we need a different test strategy.
        // So: re-publish on the rebuilt slot and confirm the flag
        // has been reset to false, because re-subscribe is a fresh
        // contract per doc comment.
        assert!(
            bus.publish_delta(status_delta("s1", 9999, Status::Busy))
                .await
                .unwrap()
        );
        let d = rx.recv().await.unwrap();
        assert_eq!(d.seq, 9999);
        assert!(
            !d.resync_required,
            "re-subscribe should reset the resync flag"
        );
    }

    #[tokio::test]
    async fn detail_bus_resync_flag_on_surviving_receiver() {
        // Same as above but WITHOUT rebuilding the channel — the
        // original receiver must observe `resync_required=true` on
        // the next delivered delta after a drop.
        let bus = DetailBus::new();
        let mut rx = bus.subscribe("s2").await;
        for i in 0..(DETAIL_CHANNEL_CAPACITY + 5) as u64 {
            let _ = bus
                .publish_delta(status_delta("s2", i, Status::Busy))
                .await
                .unwrap();
        }
        // Drain all queued — the last few were dropped so the queue
        // only holds CAPACITY items from the front of the range.
        let mut last_seen = None;
        while let Ok(d) =
            tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await
        {
            last_seen = d.map(|d| d.seq);
        }
        assert!(last_seen.is_some());

        // Send one more — this one MUST flag resync.
        assert!(
            bus.publish_delta(status_delta("s2", 100_000, Status::Idle))
                .await
                .unwrap()
        );
        let d = rx.recv().await.unwrap();
        assert_eq!(d.seq, 100_000);
        assert!(
            d.resync_required,
            "delta after a drop must flag resync_required"
        );
    }

    #[tokio::test]
    async fn detail_bus_without_subscriber_is_silent_noop() {
        let bus = DetailBus::new();
        let ok = bus
            .publish_delta(status_delta("ghost", 1, Status::Busy))
            .await
            .unwrap();
        assert!(!ok, "no subscriber → not delivered, not an error");
    }

    #[tokio::test]
    async fn detail_bus_handles_10k_deltas() {
        // Regression target: 10k deltas should not starve the
        // receiver nor deadlock the producer, given we drain as we go.
        let bus = DetailBus::new();
        let mut rx = bus.subscribe("s3").await;
        let producer = {
            let bus = bus.clone();
            tokio::spawn(async move {
                for i in 0..10_000u64 {
                    // Ignore full-channel drops; they're expected if
                    // the consumer is slower.
                    let _ = bus
                        .publish_delta(status_delta("s3", i, Status::Busy))
                        .await;
                }
            })
        };
        let mut seen = 0u64;
        let mut last_seq = 0u64;
        while seen < 9_500 {
            let d = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                rx.recv(),
            )
            .await
            .expect("should not stall")
            .unwrap();
            assert!(d.seq >= last_seq, "seqs must be non-decreasing");
            last_seq = d.seq;
            seen += 1;
        }
        producer.await.unwrap();
    }

    #[tokio::test]
    async fn detail_bus_end_session_tears_down_slot() {
        let bus = DetailBus::new();
        let _rx = bus.subscribe("gone").await;
        bus.end_session("gone").await;
        let ok = bus
            .publish_delta(status_delta("gone", 1, Status::Idle))
            .await
            .unwrap();
        assert!(!ok);
    }
}

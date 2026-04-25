//! `PricingCacheService` — process-wide cache + singleflight refresh
//! for the Anthropic pricing table.
//!
//! Why a service?
//!
//! Pre-D-3 the IPC layer (`commands_pricing.rs`) owned a
//! `static AtomicBool REFRESH_IN_FLIGHT` to keep many concurrent
//! `pricing_get` callers from spawning N parallel scrapes. That
//! singleflight covered the cache-miss path but **not** the explicit
//! `pricing_refresh` button: button-mash sent N concurrent fetches.
//!
//! This service moves both paths into one place:
//!
//! - **Cache slot** — `arc_swap::ArcSwapOption<PriceTable>` so any
//!   number of readers can `load_full()` an `Arc<PriceTable>` snapshot
//!   without contending on a lock.
//! - **Singleflight slot** — `tokio::sync::Mutex<Option<Arc<OnceCell>>>`.
//!   The first caller installs a fresh `OnceCell`, drops the mutex,
//!   and runs the fetch inside `cell.get_or_init(...)`. Concurrent
//!   callers find `Some(cell)` under the mutex, clone the `Arc`,
//!   release the mutex, and `await` the same cell. They all observe
//!   the same `Arc<PriceTable>` result. After the result lands, the
//!   first caller clears the slot so the next fetch starts fresh
//!   (failed fetches don't poison subsequent attempts).
//!
//! Callers never need to await the network on the read path:
//! `get_or_refresh_async` returns the current best snapshot
//! immediately and only kicks a background refresh when the cache is
//! `Bundled` (i.e. neither a fresh live fetch nor a recent cache file
//! is available).

use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};

use super::{bundled, load, write_cache, PriceSource, PriceTable};

/// How a `PricingCacheService` actually performs a network refresh.
/// Production wires this to the live HTML scraper in
/// [`super::fetch_live`]; tests inject a counted/scripted mock.
#[async_trait]
pub trait Fetcher: Send + Sync + 'static {
    /// Perform one refresh attempt. On success the service replaces
    /// its in-memory cache and persists to disk; on error the service
    /// surfaces a `Bundled` snapshot tagged with `last_fetch_error`.
    async fn fetch(&self) -> Result<PriceTable, String>;
}

/// Default production fetcher: hits Anthropic's public pricing page
/// via the existing best-effort scraper.
#[derive(Default)]
pub struct LiveFetcher;

#[async_trait]
impl Fetcher for LiveFetcher {
    async fn fetch(&self) -> Result<PriceTable, String> {
        super::fetch_live().await
    }
}

/// Process-wide cache + singleflight refresh.
///
/// Construct once at app startup with [`PricingCacheService::new`] and
/// register the returned `Arc<Self>` in the Tauri state container.
/// All accessors take `&Arc<Self>` so the singleflight slot is shared
/// across clones.
pub struct PricingCacheService {
    /// Best currently-available table. `None` means "haven't even
    /// loaded the on-disk cache yet"; the read path lazily seeds this
    /// from [`super::load`] on first call so app-start cost stays in
    /// the caller's tick rather than the constructor.
    cached: ArcSwapOption<PriceTable>,
    /// Singleflight slot. While a refresh is running, every caller
    /// (read-path background spawner *and* explicit `refresh_now`)
    /// joins the same `OnceCell`. After completion the slot is
    /// cleared so the next request triggers a fresh fetch.
    refresh: Mutex<Option<Arc<OnceCell<Arc<PriceTable>>>>>,
    fetcher: Arc<dyn Fetcher>,
}

impl PricingCacheService {
    /// Build a service backed by the live HTML scraper.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cached: ArcSwapOption::from(None),
            refresh: Mutex::new(None),
            fetcher: Arc::new(LiveFetcher),
        })
    }

    /// Build a service with a custom fetcher (test seam).
    #[doc(hidden)]
    pub fn with_fetcher(fetcher: Arc<dyn Fetcher>) -> Arc<Self> {
        Arc::new(Self {
            cached: ArcSwapOption::from(None),
            refresh: Mutex::new(None),
            fetcher,
        })
    }

    /// Seed the in-memory cache directly. Used by tests; production
    /// code should rely on `get_or_refresh_async` to lazily seed from
    /// the on-disk cache.
    #[doc(hidden)]
    pub fn seed_for_test(&self, table: Arc<PriceTable>) {
        self.cached.store(Some(table));
    }

    /// Returns the current best table snapshot. Never blocks on the
    /// network. If the in-memory slot is empty we lazily populate it
    /// from the on-disk cache (or bundled defaults). If the resolved
    /// source is `Bundled`, a background refresh is kicked off so the
    /// next call returns fresh numbers — but **this** call still
    /// returns the bundled snapshot immediately.
    pub fn get_or_refresh_async(self: &Arc<Self>) -> Arc<PriceTable> {
        let snapshot = self.cached.load_full().unwrap_or_else(|| {
            // Lazy-seed from disk → bundled. `load()` does its own
            // cache-file read with TTL guard, so this stays cheap.
            let fresh = Arc::new(load());
            // `compare_and_swap` would be ideal, but `ArcSwap` only
            // exposes `store`. The double-seed race is benign — both
            // values are equivalent best-effort snapshots; whichever
            // wins, subsequent readers converge.
            self.cached.store(Some(fresh.clone()));
            fresh
        });

        if matches!(snapshot.source, PriceSource::Bundled { .. }) {
            // Fire-and-forget. The spawned task joins or installs the
            // singleflight inside `refresh_now`, so concurrent callers
            // collapse to one fetch.
            let me = Arc::clone(self);
            tokio::spawn(async move {
                let _ = me.refresh_now().await;
            });
        }

        snapshot
    }

    /// Force a refresh right now and return the resulting table. If a
    /// refresh is already in flight, joins it instead of starting a
    /// second one. On fetch failure returns a `Bundled` snapshot
    /// tagged with `last_fetch_error` — never panics, never returns
    /// `Err`, never poisons the singleflight for the next caller.
    pub async fn refresh_now(self: &Arc<Self>) -> Arc<PriceTable> {
        // Briefly hold the singleflight mutex while we either join the
        // in-flight cell or install a fresh one. We must release the
        // mutex before awaiting the cell — otherwise concurrent
        // callers would serialize on the mutex instead of joining the
        // cell.
        let (cell, is_leader) = {
            let mut guard = self.refresh.lock().await;
            if let Some(existing) = guard.as_ref() {
                (Arc::clone(existing), false)
            } else {
                let cell: Arc<OnceCell<Arc<PriceTable>>> =
                    Arc::new(OnceCell::new());
                *guard = Some(Arc::clone(&cell));
                (cell, true)
            }
        };

        // `get_or_init` runs the closure exactly once per `OnceCell`;
        // every other awaiter blocks until the first completes, then
        // gets a clone of the result. This is the "Shared" semantics
        // the design doc names, implemented with one tokio primitive
        // already in our dep tree (no `futures-util::Shared`).
        let fetcher = Arc::clone(&self.fetcher);
        let result: Arc<PriceTable> = cell
            .get_or_init(|| async move {
                match fetcher.fetch().await {
                    Ok(fresh) => {
                        if let Err(e) = write_cache(&fresh) {
                            // Cache-write failure is non-fatal — the
                            // in-memory table is still usable, we
                            // just won't persist until next refresh.
                            tracing::warn!(
                                "pricing cache write failed: {e}"
                            );
                        }
                        Arc::new(fresh)
                    }
                    Err(e) => {
                        // Surface the error to the UI via the table's
                        // `last_fetch_error` field, but keep the
                        // numbers usable so the dashboard never goes
                        // blank on a transient network blip.
                        let mut fallback = bundled();
                        fallback.last_fetch_error = Some(e);
                        Arc::new(fallback)
                    }
                }
            })
            .await
            .clone();

        // Update the in-memory cache slot so subsequent
        // `get_or_refresh_async` reads pick up the fresh table.
        self.cached.store(Some(Arc::clone(&result)));

        // Only the leader clears the singleflight slot. Followers
        // already saw `Some(cell)` and don't own the lifecycle.
        // Without this guard a follower could clear the slot while a
        // *new* refresh started by another caller has already
        // installed its own cell — flushing someone else's work.
        if is_leader {
            let mut guard = self.refresh.lock().await;
            // Defensive: only clear if it's still our cell. If a
            // future refactor lets followers install cells too, this
            // pointer compare keeps us correct.
            if guard
                .as_ref()
                .map(|c| Arc::ptr_eq(c, &cell))
                .unwrap_or(false)
            {
                *guard = None;
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    /// Counts every call. Returns a fixed `Live` table so callers can
    /// distinguish a fetched result from `Bundled` defaults.
    struct CountingFetcher {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Fetcher for CountingFetcher {
        async fn fetch(&self) -> Result<PriceTable, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // Yield once so concurrent callers actually have a chance
            // to land in the singleflight rather than racing through.
            tokio::task::yield_now().await;
            Ok(PriceTable {
                models: BTreeMap::new(),
                source: PriceSource::Live {
                    url: "https://test.invalid/pricing".into(),
                    fetched_at_unix: 1_700_000_000,
                },
                last_fetch_error: None,
            })
        }
    }

    /// Returns `Err` once, then `Ok` forever after. Lets us test that
    /// a failed fetch doesn't poison the singleflight slot.
    struct FlakyFetcher {
        calls: AtomicUsize,
        // First call → Err, second+ → Ok. `StdMutex<Vec<...>>` would
        // also work; an atomic counter is simpler.
        outcomes: StdMutex<Vec<Result<PriceTable, String>>>,
    }

    #[async_trait]
    impl Fetcher for FlakyFetcher {
        async fn fetch(&self) -> Result<PriceTable, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            let mut outcomes = self.outcomes.lock().unwrap();
            if outcomes.is_empty() {
                Ok(PriceTable {
                    models: BTreeMap::new(),
                    source: PriceSource::Live {
                        url: "https://test.invalid/pricing".into(),
                        fetched_at_unix: 1_700_000_001,
                    },
                    last_fetch_error: None,
                })
            } else {
                outcomes.remove(0)
            }
        }
    }

    /// Always panics if called. Lets us assert "no network was
    /// touched" without inventing a counter pattern.
    struct ForbidFetcher;

    #[async_trait]
    impl Fetcher for ForbidFetcher {
        async fn fetch(&self) -> Result<PriceTable, String> {
            panic!("fetcher must not be called in this test");
        }
    }

    #[tokio::test]
    async fn concurrent_refreshes_yield_one_fetch() {
        let fetcher = Arc::new(CountingFetcher {
            calls: AtomicUsize::new(0),
        });
        let svc = PricingCacheService::with_fetcher(fetcher.clone());

        // 32 concurrent refreshers must all observe the same fetch.
        let mut handles = Vec::new();
        for _ in 0..32 {
            let svc = Arc::clone(&svc);
            handles.push(tokio::spawn(async move { svc.refresh_now().await }));
        }
        let mut results = Vec::with_capacity(32);
        for h in handles {
            results.push(h.await.unwrap());
        }

        assert_eq!(
            fetcher.calls.load(Ordering::SeqCst),
            1,
            "singleflight must collapse 32 concurrent refreshes into one fetch"
        );
        // Every awaiter must observe the same Arc payload — same data
        // and (since `OnceCell` clones the produced `Arc`) same
        // pointer identity for the leader; followers get the cloned
        // Arc which is `ptr_eq` to the leader's.
        let first = results.first().unwrap();
        for r in &results {
            assert!(Arc::ptr_eq(first, r));
        }
    }

    #[tokio::test]
    async fn cached_table_satisfies_get_without_fetch() {
        let svc = PricingCacheService::with_fetcher(Arc::new(ForbidFetcher));
        // Seed with a non-bundled table; `get_or_refresh_async` must
        // not spawn a refresh.
        let seeded = Arc::new(PriceTable {
            models: BTreeMap::new(),
            source: PriceSource::Live {
                url: "https://test.invalid/pricing".into(),
                fetched_at_unix: 1_700_000_002,
            },
            last_fetch_error: None,
        });
        svc.seed_for_test(Arc::clone(&seeded));

        let got = svc.get_or_refresh_async();
        assert!(Arc::ptr_eq(&got, &seeded));

        // Give any (incorrectly) spawned refresh task a chance to run
        // and panic via `ForbidFetcher`. If we return cleanly, the
        // contract held.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn failed_fetch_does_not_poison_singleflight() {
        let fetcher = Arc::new(FlakyFetcher {
            calls: AtomicUsize::new(0),
            outcomes: StdMutex::new(vec![Err("network down".into())]),
        });
        let svc = PricingCacheService::with_fetcher(fetcher.clone());

        // First refresh sees Err → service returns a bundled fallback
        // tagged with `last_fetch_error`.
        let first = svc.refresh_now().await;
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(first.source, PriceSource::Bundled { .. }));
        assert_eq!(
            first.last_fetch_error.as_deref(),
            Some("network down"),
            "failed fetch must surface the error message verbatim"
        );

        // Second refresh must spawn a *fresh* fetch (the singleflight
        // slot was cleared on completion regardless of outcome).
        let second = svc.refresh_now().await;
        assert_eq!(
            fetcher.calls.load(Ordering::SeqCst),
            2,
            "second refresh must trigger a new fetch — singleflight \
             slot must not retain the failed cell"
        );
        assert!(matches!(second.source, PriceSource::Live { .. }));
        assert!(second.last_fetch_error.is_none());
    }
}

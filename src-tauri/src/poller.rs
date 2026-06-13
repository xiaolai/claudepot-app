//! Shared scaffolding for the app's background pollers.
//!
//! Five watchers (`usage_watcher`, `usage_snapshot`,
//! `updates_watcher`, `service_status_watcher`, `cc_doctor_watcher`)
//! used to hand-roll the same `spawn → first-delay sleep → loop {
//! tick; sleep }` shape with three divergent interval idioms (fixed
//! const, read-after-tick helper, tick-returns-Duration). This
//! harness owns the loop; each watcher file keeps pure tick logic.
//!
//! The tick closure returns the duration to sleep before the next
//! tick — the most general of the three idioms (a fixed cadence is
//! just `async { tick().await; POLL_INTERVAL }`). The context `C` is
//! the `AppHandle` in production; tests pass `()`. A future per-tick
//! panic guard or shutdown hook belongs in [`poll_loop`], in exactly
//! one place.

use std::future::Future;
use std::time::Duration;

/// Spawn a stateless poller: sleep `first_delay`, then loop
/// `tick(ctx) → sleep(returned interval)` forever. The task lives
/// for the app's lifetime; tokio drops it on runtime shutdown.
pub fn spawn_poller<C, F, Fut>(ctx: C, name: &'static str, first_delay: Duration, mut tick: F)
where
    C: Clone + Send + 'static,
    F: FnMut(C) -> Fut + Send + 'static,
    Fut: Future<Output = Duration> + Send + 'static,
{
    spawn_poller_with_state(ctx, name, first_delay, (), move |ctx, ()| {
        let fut = tick(ctx);
        async move { ((), fut.await) }
    });
}

/// Spawn a poller whose tick threads owned state `S` through the
/// loop (functional state passing — no in-task mutex needed). The
/// tick returns `(next_state, next_interval)`.
pub fn spawn_poller_with_state<C, S, F, Fut>(
    ctx: C,
    name: &'static str,
    first_delay: Duration,
    state: S,
    tick: F,
) where
    C: Clone + Send + 'static,
    S: Send + 'static,
    F: FnMut(C, S) -> Fut + Send + 'static,
    Fut: Future<Output = (S, Duration)> + Send + 'static,
{
    tracing::debug!(poller = name, "spawning background poller");
    tauri::async_runtime::spawn(poll_loop(ctx, first_delay, state, tick));
}

/// The loop itself, separated from the spawn so tests can drive it
/// on a plain tokio runtime without a Tauri `AppHandle`.
async fn poll_loop<C, S, F, Fut>(ctx: C, first_delay: Duration, state: S, mut tick: F)
where
    C: Clone + Send + 'static,
    S: Send + 'static,
    F: FnMut(C, S) -> Fut + Send + 'static,
    Fut: Future<Output = (S, Duration)> + Send + 'static,
{
    tokio::time::sleep(first_delay).await;
    let mut state = state;
    loop {
        let (next_state, interval) = tick(ctx.clone(), state).await;
        state = next_state;
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// The loop must run the tick repeatedly, honoring the returned
    /// interval. Millisecond durations keep the test fast without a
    /// paused-clock test-util dependency.
    #[tokio::test]
    async fn test_poll_loop_ticks_repeatedly_after_first_delay() {
        let count = Arc::new(AtomicUsize::new(0));
        let count_for_tick = Arc::clone(&count);
        let handle = tokio::spawn(poll_loop(
            (),
            Duration::from_millis(1),
            (),
            move |(), ()| {
                let c = Arc::clone(&count_for_tick);
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    ((), Duration::from_millis(1))
                }
            },
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        assert!(
            count.load(Ordering::SeqCst) >= 2,
            "tick must run more than once (got {})",
            count.load(Ordering::SeqCst)
        );
    }

    /// State returned by one tick must be the state passed to the
    /// next — the functional state-passing contract usage_watcher
    /// relies on for its fired-set.
    #[tokio::test]
    async fn test_poll_loop_threads_state_between_ticks() {
        let observed_max = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&observed_max);
        let handle = tokio::spawn(poll_loop(
            (),
            Duration::from_millis(1),
            0usize,
            move |(), n: usize| {
                let observed = Arc::clone(&observed);
                async move {
                    observed.fetch_max(n, Ordering::SeqCst);
                    (n + 1, Duration::from_millis(1))
                }
            },
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        handle.abort();
        assert!(
            observed_max.load(Ordering::SeqCst) >= 1,
            "later ticks must observe state mutated by earlier ticks"
        );
    }
}

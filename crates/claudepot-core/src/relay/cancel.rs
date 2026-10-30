//! A cancellation signal shared by the server and an upstream handler.
//!
//! Revoking a device must end its in-flight streams, not only refuse its
//! next request, so every admitted request carries the device's token
//! and the response body stops when it fires. Built on `watch` rather
//! than pulling `tokio-util` in for one type.

use std::sync::Arc;

use tokio::sync::watch;

#[derive(Clone)]
pub struct CancelToken(Arc<watch::Sender<bool>>);

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }

    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Resolves once [`cancel`](Self::cancel) has been called, immediately
    /// if it already was.
    pub async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        // `wait_for` only errors when the sender is dropped, and `self`
        // holds it; treat that impossible case as "never cancelled".
        if rx.wait_for(|c| *c).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn cancelled_resolves_after_cancel_and_stays_resolved() {
        let t = CancelToken::new();
        assert!(!t.is_cancelled());
        let waiter = {
            let t = t.clone();
            tokio::spawn(async move { t.cancelled().await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        t.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .unwrap()
            .unwrap();
        assert!(t.is_cancelled());
        tokio::time::timeout(Duration::from_millis(50), t.cancelled())
            .await
            .unwrap();
    }
}

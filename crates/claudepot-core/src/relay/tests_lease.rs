//! Lease rules that span an await: cancellation, recovery, and the
//! identity a publication expects.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::mock::*;
use super::*;

fn expecting(account: &'static str) -> IdentityCheck {
    Arc::new(move |i: &Identity| i.account_uuid == account)
}

#[tokio::test]
async fn a_cancelled_renewal_whose_saves_all_fail_keeps_the_live_refresh_token() {
    let b = MockBackend::new("sk-ant-oat01-a", 60);
    b.failing_saves
        .store(SAVE_ATTEMPTS as usize, Ordering::SeqCst);
    let m = LeaseManager::new(b.clone());
    let cut = tokio::time::timeout(Duration::from_millis(1), m.current()).await;
    assert!(
        cut.is_err(),
        "the caller must have been dropped mid-renewal"
    );
    // Let the detached renewal finish its save retries.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-0");

    let l = m.current().await.unwrap();
    assert_eq!(l.access_token.expose(), "sk-ant-oat01-renewed-1");
    assert_eq!(b.refreshes(), 1, "the held renewal is reused");
    assert_eq!(
        b.blob.lock().claude_ai_oauth.refresh_token,
        "rt-1",
        "and saved at the next acquisition"
    );
}

#[tokio::test]
async fn a_caller_dropped_while_saving_the_held_credential_does_not_lose_it() {
    let b = MockBackend::new("sk-ant-oat01-a", 60);
    b.failing_saves
        .store(SAVE_ATTEMPTS as usize, Ordering::SeqCst);
    let m = LeaseManager::new(b.clone());
    let first = m.current().await.unwrap();
    assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-0");

    // The next acquisition saves the held blob first; drop it there.
    b.failing_saves.store(1, Ordering::SeqCst);
    b.save_delay_ms.store(50, Ordering::SeqCst);
    let cut = tokio::time::timeout(Duration::from_millis(10), m.invalidate(first.generation)).await;
    assert!(cut.is_err(), "the caller must have been dropped mid-save");
    b.save_delay_ms.store(0, Ordering::SeqCst);

    let next = m.invalidate(first.generation).await.unwrap();
    assert_eq!(next.access_token.expose(), "sk-ant-oat01-renewed-2");
    assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-2");
}

#[tokio::test]
async fn a_saved_renewal_supersedes_an_older_held_credential() {
    let b = MockBackend::new("sk-ant-oat01-a", 60);
    b.failing_saves
        .store(SAVE_ATTEMPTS as usize, Ordering::SeqCst);
    let m = LeaseManager::new(b.clone());
    let first = m.current().await.unwrap();
    // The held blob's own save fails once more; the renewal's save works.
    b.failing_saves.store(1, Ordering::SeqCst);
    let second = m.invalidate(first.generation).await.unwrap();
    assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-2");
    // Nothing stale is written back over it at the next renewal.
    let third = m.invalidate(second.generation).await.unwrap();
    assert_eq!(third.access_token.expose(), "sk-ant-oat01-renewed-3");
    assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-3");
}

#[tokio::test]
async fn a_first_identity_other_than_the_expected_one_stops_the_lease() {
    let b = MockBackend::new("sk-ant-oat01-a", 3600);
    b.failing_identities.store(1, Ordering::SeqCst);
    let m = LeaseManager::new(b.clone());
    assert_eq!(
        m.current().await.unwrap_err(),
        UpstreamError::CredentialUnavailable
    );
    m.expect_identity(expecting("acct")).await;

    *b.identity.lock() = identity("someone-else");
    assert_eq!(
        m.current().await.unwrap_err(),
        UpstreamError::IdentityChanged
    );
    *b.identity.lock() = identity("acct");
    assert_eq!(
        m.current().await.unwrap_err(),
        UpstreamError::IdentityChanged,
        "the stop is sticky"
    );
    assert_eq!(m.readiness().identity, None, "nothing was pinned");
}

#[tokio::test]
async fn the_expected_identity_is_pinned_when_it_is_the_one_found() {
    let b = MockBackend::new("sk-ant-oat01-a", 3600);
    let m = LeaseManager::new(b.clone());
    m.expect_identity(expecting("acct")).await;
    m.current().await.unwrap();
    assert_eq!(m.readiness().identity, Some(identity("acct")));
}

#[tokio::test]
async fn a_transient_failure_recovers_through_health_alone() {
    let b = MockBackend::new("sk-ant-oat01-a", 3600);
    b.failing_identities.store(1, Ordering::SeqCst);
    let m = LeaseManager::with_recovery(b.clone(), Duration::ZERO, REJECTION_HOLD);
    assert!(m.current().await.is_err());
    assert!(!m.readiness().ready());

    assert!(m.claim_recovery(Instant::now()));
    assert!(!m.claim_recovery(Instant::now()), "one recovery at a time");
    m.recover().await;
    assert!(m.readiness().ready());
    assert_eq!(m.readiness().identity, Some(identity("acct")));
    assert!(!m.claim_recovery(Instant::now()), "nothing to recover");
}

#[tokio::test]
async fn recovery_waits_for_its_interval_and_never_lifts_a_sticky_stop() {
    let b = MockBackend::new("sk-ant-oat01-a", 3600);
    b.failing_identities.store(1, Ordering::SeqCst);
    let m = LeaseManager::new(b.clone());
    assert!(m.current().await.is_err());
    let now = Instant::now();
    assert!(!m.claim_recovery(now), "not inside the interval");
    assert!(m.claim_recovery(now + REPROBE_AFTER));
    m.recover().await;
    assert!(m.readiness().ready());

    b.cc_owns.store(true, Ordering::SeqCst);
    let stopped = LeaseManager::with_recovery(b.clone(), Duration::ZERO, Duration::ZERO);
    assert!(stopped.current().await.is_err());
    assert!(!stopped.claim_recovery(Instant::now() + REJECTION_HOLD));
}

#[tokio::test]
async fn a_rejection_is_retried_only_after_the_hold() {
    let b = MockBackend::new("sk-ant-oat01-a", 3600);
    let m = LeaseManager::with_recovery(b.clone(), Duration::ZERO, Duration::from_secs(60));
    m.current().await.unwrap();
    m.rejected();
    let now = Instant::now();
    assert!(!m.claim_recovery(now));
    assert!(m.claim_recovery(now + Duration::from_secs(61)));
    m.recover().await;
    assert!(m.readiness().ready());
    assert_eq!(b.refreshes(), 0, "a fresh token needs no refresh");
}

#[tokio::test]
async fn queued_rejections_of_one_generation_share_a_failed_refresh() {
    let b = MockBackend::new("sk-ant-oat01-a", 3600);
    let m = Arc::new(LeaseManager::with_recovery(
        b.clone(),
        Duration::from_millis(200),
        REJECTION_HOLD,
    ));
    let first_gen = m.current().await.unwrap().generation;
    // The provider refuses the refresh token: every renewal fails.
    *b.live_refresh.lock() = "rt-elsewhere".into();
    let tasks: Vec<_> = (0..16)
        .map(|_| {
            let m = m.clone();
            tokio::spawn(async move { m.invalidate(first_gen).await })
        })
        .collect();
    for t in tasks {
        assert_eq!(
            t.await.unwrap().unwrap_err(),
            UpstreamError::CredentialUnavailable
        );
    }
    assert_eq!(
        b.refresh_attempts.load(Ordering::SeqCst),
        1,
        "one refresh per rejected generation, even when it fails"
    );

    // Bounded: once the interval passes, the generation may try again.
    tokio::time::sleep(Duration::from_millis(250)).await;
    *b.live_refresh.lock() = "rt-0".into();
    let l = m.invalidate(first_gen).await.unwrap();
    assert_eq!(l.access_token.expose(), "sk-ant-oat01-renewed-1");
    assert_eq!(b.refresh_attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn an_unsaved_renewal_never_overwrites_a_re_login() {
    let b = MockBackend::new("sk-ant-oat01-a", 60);
    b.failing_saves
        .store(SAVE_ATTEMPTS as usize, Ordering::SeqCst);
    let m = LeaseManager::new(b.clone());
    let first = m.current().await.unwrap();
    assert_eq!(first.access_token.expose(), "sk-ant-oat01-renewed-1");
    assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-0");

    // The user signs in again between acquisitions.
    *b.blob.lock() = blob("sk-ant-oat01-relogin", "rt-login", 3600);
    *b.live_refresh.lock() = "rt-login".into();

    let next = m.invalidate(first.generation).await.unwrap();
    assert_eq!(next.access_token.expose(), "sk-ant-oat01-relogin");
    assert_eq!(
        b.blob.lock().claude_ai_oauth.refresh_token,
        "rt-login",
        "the re-login stays in the store"
    );
    assert_eq!(b.refreshes(), 1, "the re-login's token needed no refresh");
}

#[tokio::test]
async fn a_chain_of_unsaved_renewals_is_still_saved_over_its_own_predecessor() {
    let b = MockBackend::new("sk-ant-oat01-a", 60);
    b.failing_saves
        .store(SAVE_ATTEMPTS as usize * 2 + 1, Ordering::SeqCst);
    let m = LeaseManager::new(b.clone());
    // First renewal: all saves fail, held.
    let first = m.current().await.unwrap();
    // Second: the held blob's save fails, it renews again, all saves fail.
    let second = m.invalidate(first.generation).await.unwrap();
    assert_eq!(second.access_token.expose(), "sk-ant-oat01-renewed-2");
    assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-0");
    // Third: the store still holds the lineage's start, so the held blob
    // is saved, not dropped as if someone had signed in.
    let third = m.invalidate(second.generation).await.unwrap();
    assert_eq!(third.access_token.expose(), "sk-ant-oat01-renewed-3");
    assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-3");
}

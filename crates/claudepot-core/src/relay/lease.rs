//! The subscription upstream's access token, leased for a long-running
//! server.
//!
//! Not `launcher::get_access_token`: that function's active-account
//! branch force-refreshes Claude Code's own login, an exemption justified
//! for a user-initiated launch and wrong for a server that renews for
//! hours. It also returns a bare `String`, with no expiry and no way to
//! say "this token was rejected".
//!
//! ## Rules
//!
//! - **Never the CC-active account.** Checked under the credential lock
//!   at start and before every renewal. P0b measured why: renewing an
//!   account from outside rotated its refresh token, invalidated CC's
//!   unexpired access token, and CC emptied its login. Once seen, the
//!   conflict is sticky — the lease stops issuing until restarted.
//! - **One renewal per rejection.** An upstream 401 calls
//!   [`LeaseManager::invalidate`] with the generation it used; only the
//!   first caller for that generation renews, and it first re-reads the
//!   store in case another process already did. A renewal that fails is
//!   remembered for its generation: callers queued behind it get its
//!   error, not a refresh of their own, until [`REPROBE_AFTER`] passes.
//! - **Identity is the provider's**, `account_uuid` + `org_uuid` from the
//!   profile endpoint, pinned at the first acquisition and re-derived at
//!   every renewal. A change is `identity_changed`, sticky. When the
//!   start probe could not confirm the published identity, the first
//!   identity pinned later must match the publication
//!   ([`LeaseManager::expect_identity`]); a different one is
//!   `identity_changed` too, never silently pinned.
//! - **A transient failure recovers without a request.** Health calls
//!   [`LeaseManager::claim_recovery`] and runs [`LeaseManager::recover`]
//!   in the background, at most once per [`REPROBE_AFTER`] (a rejection:
//!   [`REJECTION_HOLD`]). Sticky stops never recover.
//! - A renewal's result is saved before its identity is checked: the
//!   refresh token came from this slot, so its successor belongs to the
//!   same lineage whatever it turns out to authenticate as, and dropping
//!   it would end that login.
//! - A renewal whose save failed is held, and saved at the next
//!   acquisition **only if the store still holds the credential it was
//!   renewed from**. Anything else there was written since — a re-login —
//!   and is newer than the held blob, which is dropped rather than written
//!   over it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use uuid::Uuid;

use super::passthrough::{REJECTION_HOLD, REPROBE_AFTER};
use super::secret::Secret;
use super::upstream::UpstreamError;
use crate::blob::CredentialBlob;
use crate::cli_backend::swap::{self, DefaultRefresher, TokenRefresher};
use crate::cli_backend::CliPlatform;
use crate::oauth::refresh::{build_blob, TokenResponse};
use crate::services::account_service::{DefaultProfileFetcher, ProfileFetcher};

/// Renew this long before the token's own expiry.
pub const RENEW_MARGIN_SECS: i64 = 300;

/// Saves of a renewed credential tried before it is held in memory.
const SAVE_ATTEMPTS: u32 = 3;
const SAVE_RETRY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub account_uuid: String,
    pub org_uuid: String,
}

#[derive(Debug, Clone)]
pub struct Lease {
    pub access_token: Secret,
    /// Milliseconds since the epoch, as CC stores it.
    pub expires_at: i64,
    /// Local counter, bumped each time a different token is adopted.
    pub generation: u64,
}

impl Lease {
    fn is_expiring(&self) -> bool {
        self.expires_at < chrono::Utc::now().timestamp_millis() + RENEW_MARGIN_SECS * 1000
    }
}

/// Held for as long as the cross-process credential lock must be.
pub type LockGuard = Box<dyn Send>;

/// The I/O a lease needs, behind a trait so its rules are testable
/// without a keychain or a network.
#[async_trait::async_trait]
pub trait LeaseBackend: Send + Sync {
    async fn lock(&self) -> Result<LockGuard, UpstreamError>;
    async fn load(&self) -> Result<CredentialBlob, UpstreamError>;
    async fn save(&self, blob_json: &str) -> Result<(), UpstreamError>;
    /// Is Claude Code on this machine signed in as this account? Must
    /// fail closed: "cannot tell" is an error, never `false`.
    async fn cc_owns(&self, slot: &CredentialBlob) -> Result<bool, UpstreamError>;
    async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, UpstreamError>;
    async fn identity(&self, access_token: &str) -> Result<Identity, UpstreamError>;
}

/// What the first pinned identity must satisfy. Set when a publication
/// was accepted for an identity this process could not confirm at start:
/// the first identity it does confirm must be that one, or a different
/// account would serve under the accepted fingerprint and generation.
pub type IdentityCheck = Arc<dyn Fn(&Identity) -> bool + Send + Sync>;

#[derive(Default)]
struct State {
    lease: Option<Lease>,
    identity: Option<Identity>,
    stopped: Option<UpstreamError>,
    next_generation: u64,
    expected: Option<IdentityCheck>,
    /// The generation whose renewal last failed, the error, and when.
    failed_renewal: Option<(u64, UpstreamError, Instant)>,
}

/// A renewed blob whose save failed. Its refresh token is the only live
/// one — the stored one was spent — so it is saved first and used in place
/// of the store until a save succeeds. Shared with the detached renewal so
/// that a caller dropped mid-renewal cannot lose it.
type Unsaved = Arc<parking_lot::Mutex<Option<Held>>>;

#[derive(Clone)]
struct Held {
    json: String,
    /// The refresh token the store held when this lineage left it: the
    /// store still holding it is what makes writing `json` over it safe.
    stored_refresh: Secret,
}

/// The last acquisition's result, readable without waiting on a renewal
/// in progress — health must not stall behind a slow refresh.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Readiness {
    /// The sticky stop reason, if the lease has stopped.
    pub stopped: Option<UpstreamError>,
    /// The last acquisition's error; cleared by the next success.
    pub last_error: Option<UpstreamError>,
    /// The provider identity pinned at the first acquisition.
    pub identity: Option<Identity>,
}

impl Readiness {
    /// Nothing known to be wrong. A lease never yet acquired is ready:
    /// the first request acquires it, and refusing to score a server
    /// until someone has used it would leave it unused.
    pub fn ready(&self) -> bool {
        self.stopped.is_none() && self.last_error.is_none()
    }
}

/// [`Readiness`] plus what decides when health may retry an acquisition.
#[derive(Default)]
struct Health {
    readiness: Readiness,
    last_attempt: Option<Instant>,
    rejected_at: Option<Instant>,
    probing: bool,
}

pub struct LeaseManager {
    backend: Arc<dyn LeaseBackend>,
    state: Mutex<State>,
    unsaved: Unsaved,
    health: parking_lot::Mutex<Health>,
    reprobe_after: Duration,
    rejection_hold: Duration,
}

impl LeaseManager {
    pub fn new(backend: Arc<dyn LeaseBackend>) -> Self {
        Self::with_recovery(backend, REPROBE_AFTER, REJECTION_HOLD)
    }

    /// [`Self::new`] with the recovery intervals given, for tests.
    pub fn with_recovery(
        backend: Arc<dyn LeaseBackend>,
        reprobe_after: Duration,
        rejection_hold: Duration,
    ) -> Self {
        Self {
            backend,
            state: Mutex::new(State::default()),
            unsaved: Arc::default(),
            health: parking_lot::Mutex::new(Health::default()),
            reprobe_after,
            rejection_hold,
        }
    }

    pub fn readiness(&self) -> Readiness {
        self.health.lock().readiness.clone()
    }

    /// Require the first identity this lease pins to pass `check`. No
    /// effect once an identity is pinned: from then on any change is
    /// `identity_changed` anyway.
    pub async fn expect_identity(&self, check: IdentityCheck) {
        let mut st = self.state.lock().await;
        if st.identity.is_none() {
            st.expected = Some(check);
        }
    }

    fn note(&self, st: &State, result: &Result<Lease, UpstreamError>) {
        let mut h = self.health.lock();
        h.readiness = Readiness {
            stopped: st.stopped,
            last_error: result.as_ref().err().copied(),
            identity: st.identity.clone(),
        };
        h.last_attempt = Some(Instant::now());
    }

    /// A token good for at least [`RENEW_MARGIN_SECS`], renewing first if
    /// the held one is closer to expiry than that.
    pub async fn current(&self) -> Result<Lease, UpstreamError> {
        let mut st = self.state.lock().await;
        if let Some(e) = st.stopped {
            return Err(e);
        }
        if let Some(l) = &st.lease {
            if !l.is_expiring() {
                return Ok(l.clone());
            }
        }
        let r = self.acquire(&mut st, None).await;
        self.note(&st, &r);
        r
    }

    /// The upstream rejected the token of `generation`. Renews once per
    /// generation; later callers for the same generation get the
    /// renewed lease without another refresh.
    pub async fn invalidate(&self, generation: u64) -> Result<Lease, UpstreamError> {
        let mut st = self.state.lock().await;
        if let Some(e) = st.stopped {
            return Err(e);
        }
        let rejected = match &st.lease {
            Some(l) if l.generation != generation => return Ok(l.clone()),
            Some(l) => Some(l.access_token.clone()),
            None => None,
        };
        if let Some((g, e, at)) = st.failed_renewal {
            if g == generation && at.elapsed() < self.reprobe_after {
                return Err(e);
            }
        }
        let r = self.acquire(&mut st, rejected.as_ref()).await;
        let still_rejected = st.lease.as_ref().map(|l| l.generation) == Some(generation);
        st.failed_renewal = match &r {
            Err(e) if rejected.is_some() && still_rejected => {
                Some((generation, *e, Instant::now()))
            }
            _ => None,
        };
        self.note(&st, &r);
        r
    }

    /// The upstream refused even the renewed token: health reports the
    /// credential as rejected until a request is accepted again, or a
    /// recovery after [`REJECTION_HOLD`] re-acquires it.
    pub fn rejected(&self) {
        let mut h = self.health.lock();
        h.readiness.last_error = Some(UpstreamError::CredentialRejected);
        h.rejected_at = Some(Instant::now());
    }

    /// A request was accepted, so a rejection recorded by
    /// [`Self::rejected`] no longer holds. Other errors stay until the next
    /// acquisition replaces them.
    pub fn accepted(&self) {
        let mut h = self.health.lock();
        if h.readiness.last_error == Some(UpstreamError::CredentialRejected) {
            h.readiness.last_error = None;
        }
    }

    /// Claim the one background recovery: only while a non-sticky error
    /// stands, none is running, and the last attempt (for a rejection, the
    /// rejection) is older than its interval. Without it a transient
    /// failure would keep the upstream unready forever: clients route
    /// around an unready upstream, so no request would arrive to clear it.
    pub fn claim_recovery(&self, now: Instant) -> bool {
        let mut h = self.health.lock();
        if h.readiness.stopped.is_some() || h.probing {
            return false;
        }
        let since = match h.readiness.last_error {
            None => return false,
            Some(UpstreamError::CredentialRejected) => (h.rejected_at, self.rejection_hold),
            Some(_) => (h.last_attempt, self.reprobe_after),
        };
        let due = since
            .0
            .is_none_or(|t| now.saturating_duration_since(t) >= since.1);
        if due {
            h.probing = true;
        }
        due
    }

    /// The recovery [`Self::claim_recovery`] claimed: one acquisition,
    /// under every rule a request's would be.
    pub async fn recover(&self) {
        {
            let mut st = self.state.lock().await;
            if st.stopped.is_none() {
                let r = self.acquire(&mut st, None).await;
                self.note(&st, &r);
            }
        }
        self.health.lock().probing = false;
    }

    /// The sticky stop reason, if any — for the status event.
    pub async fn stopped(&self) -> Option<UpstreamError> {
        self.state.lock().await.stopped
    }

    async fn acquire(
        &self,
        st: &mut State,
        rejected: Option<&Secret>,
    ) -> Result<Lease, UpstreamError> {
        let guard = self.backend.lock().await?;
        let slot = self.load_slot().await?;
        if self.backend.cc_owns(&slot).await? {
            tracing::warn!("relay lease stopped: account is Claude Code's active login");
            return Err(stop(st, UpstreamError::OwnerConflict));
        }
        if slot.is_signed_out() {
            return Err(UpstreamError::CredentialUnavailable);
        }

        let stored = &slot.claude_ai_oauth;
        let stored_usable = !slot.is_expired(RENEW_MARGIN_SECS)
            && rejected.is_none_or(|r| r.expose() != stored.access_token);
        let (token, expires_at) = if stored_usable {
            // Fresh enough, and not the token just rejected: another
            // process (or an earlier renewal) already did the work.
            (Secret::new(stored.access_token.clone()), stored.expires_at)
        } else {
            if !slot.can_refresh() {
                return Err(UpstreamError::CredentialUnavailable);
            }
            self.renew(guard, slot.clone()).await?
        };

        let identity = self.backend.identity(token.expose()).await?;
        pin_identity(st, identity)?;
        Ok(adopt(st, token, expires_at))
    }

    /// The slot to work from: a held unsaved blob — saved first if the
    /// store still holds its predecessor — or the store. The held blob is
    /// cloned, never taken, so a caller dropped during the save cannot drop
    /// the only live refresh token. The credential lock is held, so nothing
    /// that takes it changes the store meanwhile; a re-login between
    /// acquisitions shows up as a store that moved on.
    async fn load_slot(&self) -> Result<CredentialBlob, UpstreamError> {
        let held = self.unsaved.lock().clone();
        let Some(held) = held else {
            return self.backend.load().await;
        };
        let parsed = || {
            CredentialBlob::from_json(&held.json).map_err(|_| UpstreamError::CredentialUnavailable)
        };
        let stored = match self.backend.load().await {
            Ok(s) => s,
            // The store cannot be seen, so it cannot be checked: use the
            // held blob, and leave the store alone.
            Err(_) => return parsed(),
        };
        if stored.claude_ai_oauth.refresh_token != held.stored_refresh.expose() {
            // A credential written since the renewal: newer than ours.
            tracing::warn!(
                "relay lease dropped an unsaved renewal: the stored credential changed since"
            );
            *self.unsaved.lock() = None;
            return Ok(stored);
        }
        if self.backend.save(&held.json).await.is_ok() {
            *self.unsaved.lock() = None;
        }
        parsed()
    }

    /// Refresh from `prior` and save the successor. The refresh spends the
    /// stored refresh token, so it, the save, and recording a failed save
    /// all run detached, holding the credential lock: a caller dropped
    /// mid-renewal (the client went away) must not drop the only live
    /// refresh token with it.
    async fn renew(
        &self,
        guard: LockGuard,
        prior: CredentialBlob,
    ) -> Result<(Secret, i64), UpstreamError> {
        let backend = self.backend.clone();
        let unsaved = self.unsaved.clone();
        // A held blob's predecessor is still what the store holds: `prior`
        // is that blob, never written, so the lineage's anchor stays.
        let stored_refresh = self
            .unsaved
            .lock()
            .as_ref()
            .map(|h| h.stored_refresh.clone())
            .unwrap_or_else(|| Secret::new(prior.claude_ai_oauth.refresh_token.clone()));
        let renewal = tokio::spawn(async move {
            let resp = backend
                .refresh(&prior.claude_ai_oauth.refresh_token)
                .await?;
            let json = build_blob(&resp, Some(&prior));
            let saved = save_with_retries(backend.as_ref(), &json).await;
            *unsaved.lock() = if saved {
                None
            } else {
                tracing::error!(
                    "relay lease renewed but the credential could not be saved; keeping it in memory and saving it at the next renewal"
                );
                Some(Held {
                    json: json.clone(),
                    stored_refresh,
                })
            };
            // Released only once the successor is saved or held.
            drop(guard);
            Ok::<_, UpstreamError>(json)
        });
        let json = renewal
            .await
            .map_err(|_| UpstreamError::CredentialUnavailable)??;
        let blob =
            CredentialBlob::from_json(&json).map_err(|_| UpstreamError::CredentialUnavailable)?;
        tracing::debug!("relay lease renewed");
        Ok((
            Secret::new(blob.claude_ai_oauth.access_token.clone()),
            blob.claude_ai_oauth.expires_at,
        ))
    }
}

/// Stop the lease for good with `e`.
fn stop(st: &mut State, e: UpstreamError) -> UpstreamError {
    st.stopped = Some(e);
    st.lease = None;
    e
}

/// Check `identity` against the pinned one, or pin it — if it is the one
/// a publication expects.
fn pin_identity(st: &mut State, identity: Identity) -> Result<(), UpstreamError> {
    match &st.identity {
        Some(pinned) if *pinned != identity => {
            tracing::warn!("relay lease stopped: account identity changed");
            Err(stop(st, UpstreamError::IdentityChanged))
        }
        Some(_) => Ok(()),
        None if st.expected.as_ref().is_some_and(|ok| !ok(&identity)) => {
            tracing::warn!(
                "relay lease stopped: account identity is not the one its publication was accepted for"
            );
            Err(stop(st, UpstreamError::IdentityChanged))
        }
        None => {
            st.identity = Some(identity);
            Ok(())
        }
    }
}

/// Hold `token`, keeping the generation when it is the token already held.
fn adopt(st: &mut State, token: Secret, expires_at: i64) -> Lease {
    let same_token = st.lease.as_ref().is_some_and(|l| l.access_token == token);
    let generation = if same_token {
        st.lease.as_ref().map_or(0, |l| l.generation)
    } else {
        st.next_generation += 1;
        st.next_generation
    };
    let lease = Lease {
        access_token: token,
        expires_at,
        generation,
    };
    st.lease = Some(lease.clone());
    lease
}

async fn save_with_retries(backend: &dyn LeaseBackend, json: &str) -> bool {
    for attempt in 0..SAVE_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(SAVE_RETRY * attempt).await;
        }
        if backend.save(json).await.is_ok() {
            return true;
        }
    }
    false
}

/// The production backend: Claudepot's private slot for one account,
/// the swap lock, and Claude Code's credential store as the owner check.
pub struct StoreBackend {
    pub account_id: Uuid,
    pub email: String,
    pub platform: Box<dyn CliPlatform>,
    pub claude_json: Option<std::path::PathBuf>,
}

impl StoreBackend {
    pub fn new(account_id: Uuid, email: String) -> Self {
        Self {
            account_id,
            email,
            platform: crate::cli_backend::create_platform(),
            claude_json: crate::cli_backend::claude_json::default_path(),
        }
    }
}

#[async_trait::async_trait]
impl LeaseBackend for StoreBackend {
    async fn lock(&self) -> Result<LockGuard, UpstreamError> {
        // A blocking flock: off the runtime threads.
        let file = tokio::task::spawn_blocking(swap::acquire_swap_lock)
            .await
            .map_err(|_| UpstreamError::CredentialUnavailable)?
            .map_err(|_| UpstreamError::CredentialUnavailable)?;
        Ok(Box::new(file))
    }

    async fn load(&self) -> Result<CredentialBlob, UpstreamError> {
        let s = swap::load_private(self.account_id)
            .await
            .map_err(|_| UpstreamError::CredentialUnavailable)?;
        CredentialBlob::from_json(&s).map_err(|_| UpstreamError::CredentialUnavailable)
    }

    async fn save(&self, blob_json: &str) -> Result<(), UpstreamError> {
        swap::save_private(self.account_id, blob_json)
            .await
            .map_err(|_| UpstreamError::CredentialUnavailable)
    }

    async fn cc_owns(&self, slot: &CredentialBlob) -> Result<bool, UpstreamError> {
        match crate::launcher::cc_state(
            self.platform.as_ref(),
            slot,
            &self.email,
            &DefaultProfileFetcher,
            self.claude_json.as_deref(),
        )
        .await
        {
            Ok(crate::launcher::CcCredentialState::Same(_)) => Ok(true),
            Ok(crate::launcher::CcCredentialState::DifferentOrEmpty) => Ok(false),
            Err(_) => Err(UpstreamError::CredentialUnavailable),
        }
    }

    async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, UpstreamError> {
        DefaultRefresher
            .refresh(refresh_token)
            .await
            .map_err(|_| UpstreamError::CredentialUnavailable)
    }

    async fn identity(&self, access_token: &str) -> Result<Identity, UpstreamError> {
        let p = DefaultProfileFetcher
            .fetch(access_token)
            .await
            .map_err(|_| UpstreamError::CredentialUnavailable)?;
        Ok(Identity {
            account_uuid: p.account_uuid,
            org_uuid: p.org_uuid,
        })
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! An in-memory backend that counts refreshes; shared with the
    //! server tests.
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    pub struct MockBackend {
        pub blob: parking_lot::Mutex<CredentialBlob>,
        pub refreshes: AtomicUsize,
        /// Refresh calls, successful or not.
        pub refresh_attempts: AtomicUsize,
        pub cc_owns: AtomicBool,
        pub identity: parking_lot::Mutex<Identity>,
        pub lock: Arc<Mutex<()>>,
        /// Prefix for the access tokens refreshes mint.
        pub token_prefix: String,
        /// The one refresh token the provider would accept now.
        pub live_refresh: parking_lot::Mutex<String>,
        /// Saves to fail before saves succeed again.
        pub failing_saves: AtomicUsize,
        /// Identity lookups to fail before they succeed again.
        pub failing_identities: AtomicUsize,
        /// Each save first waits this many milliseconds.
        pub save_delay_ms: AtomicUsize,
    }

    pub fn blob(access: &str, refresh: &str, expires_in_secs: i64) -> CredentialBlob {
        let json = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": access,
                "refreshToken": refresh,
                "expiresAt": chrono::Utc::now().timestamp_millis() + expires_in_secs * 1000,
                "scopes": ["user:inference"],
            }
        });
        CredentialBlob::from_json(&json.to_string()).unwrap()
    }

    pub fn identity(a: &str) -> Identity {
        Identity {
            account_uuid: a.into(),
            org_uuid: "org".into(),
        }
    }

    impl MockBackend {
        pub fn new(access: &str, expires_in_secs: i64) -> Arc<Self> {
            Arc::new(Self {
                blob: parking_lot::Mutex::new(blob(access, "rt-0", expires_in_secs)),
                refreshes: AtomicUsize::new(0),
                refresh_attempts: AtomicUsize::new(0),
                cc_owns: AtomicBool::new(false),
                identity: parking_lot::Mutex::new(identity("acct")),
                lock: Arc::new(Mutex::new(())),
                token_prefix: "sk-ant-oat01-renewed".into(),
                live_refresh: parking_lot::Mutex::new("rt-0".into()),
                failing_saves: AtomicUsize::new(0),
                failing_identities: AtomicUsize::new(0),
                save_delay_ms: AtomicUsize::new(0),
            })
        }

        pub fn refreshes(&self) -> usize {
            self.refreshes.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl LeaseBackend for MockBackend {
        async fn lock(&self) -> Result<LockGuard, UpstreamError> {
            Ok(Box::new(self.lock.clone().lock_owned().await))
        }
        async fn load(&self) -> Result<CredentialBlob, UpstreamError> {
            Ok(self.blob.lock().clone())
        }
        async fn save(&self, blob_json: &str) -> Result<(), UpstreamError> {
            let delay = self.save_delay_ms.load(Ordering::SeqCst) as u64;
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            let failing = self.failing_saves.load(Ordering::SeqCst);
            if failing > 0 {
                self.failing_saves.store(failing - 1, Ordering::SeqCst);
                return Err(UpstreamError::CredentialUnavailable);
            }
            *self.blob.lock() = CredentialBlob::from_json(blob_json).unwrap();
            Ok(())
        }
        async fn cc_owns(&self, _: &CredentialBlob) -> Result<bool, UpstreamError> {
            Ok(self.cc_owns.load(Ordering::SeqCst))
        }
        async fn refresh(&self, rt: &str) -> Result<TokenResponse, UpstreamError> {
            self.refresh_attempts.fetch_add(1, Ordering::SeqCst);
            // Single-use, like the real endpoint: a spent token fails.
            if rt != *self.live_refresh.lock() {
                return Err(UpstreamError::CredentialUnavailable);
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            let n = self.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
            *self.live_refresh.lock() = format!("rt-{n}");
            Ok(TokenResponse {
                access_token: format!("{}-{n}", self.token_prefix),
                refresh_token: format!("rt-{n}"),
                expires_in: 3600,
                scope: None,
                token_type: None,
                refresh_token_expires_in: None,
            })
        }
        async fn identity(&self, _: &str) -> Result<Identity, UpstreamError> {
            let failing = self.failing_identities.load(Ordering::SeqCst);
            if failing > 0 {
                self.failing_identities.store(failing - 1, Ordering::SeqCst);
                return Err(UpstreamError::CredentialUnavailable);
            }
            Ok(self.identity.lock().clone())
        }
    }
}

#[cfg(test)]
#[path = "tests_lease.rs"]
mod tests_lease;

#[cfg(test)]
mod tests {
    use super::mock::*;
    use super::*;
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn a_fresh_stored_token_is_used_without_refreshing() {
        let b = MockBackend::new("sk-ant-oat01-a", 3600);
        let m = LeaseManager::new(b.clone());
        let l = m.current().await.unwrap();
        assert_eq!(l.access_token.expose(), "sk-ant-oat01-a");
        assert_eq!(b.refreshes(), 0);
    }

    #[tokio::test]
    async fn a_token_inside_the_margin_is_renewed_and_saved() {
        let b = MockBackend::new("sk-ant-oat01-a", 60);
        let m = LeaseManager::new(b.clone());
        let l = m.current().await.unwrap();
        assert_eq!(l.access_token.expose(), "sk-ant-oat01-renewed-1");
        assert_eq!(b.refreshes(), 1);
        assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-1");
    }

    #[tokio::test]
    async fn a_renewal_outlives_a_caller_dropped_mid_refresh() {
        let b = MockBackend::new("sk-ant-oat01-a", 60);
        let m = LeaseManager::new(b.clone());
        // The mock refresh takes 5 ms; give up on it after 1 ms.
        let cut = tokio::time::timeout(std::time::Duration::from_millis(1), m.current()).await;
        assert!(
            cut.is_err(),
            "the caller must have been dropped mid-renewal"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-1");
        let l = m.current().await.unwrap();
        assert_eq!(l.access_token.expose(), "sk-ant-oat01-renewed-1");
        assert_eq!(
            b.refreshes(),
            1,
            "the saved renewal is reused, not repeated"
        );
    }

    #[tokio::test]
    async fn a_failed_save_keeps_the_renewed_credential_for_the_next_renewal() {
        let b = MockBackend::new("sk-ant-oat01-a", 60);
        b.failing_saves
            .store(SAVE_ATTEMPTS as usize, Ordering::SeqCst);
        let m = LeaseManager::new(b.clone());
        let first = m.current().await.unwrap();
        assert_eq!(first.access_token.expose(), "sk-ant-oat01-renewed-1");
        assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-0");
        // The next renewal saves the held blob first, then renews from
        // its live refresh token rather than the spent stored one.
        let next = m.invalidate(first.generation).await.unwrap();
        assert_eq!(next.access_token.expose(), "sk-ant-oat01-renewed-2");
        assert_eq!(b.blob.lock().claude_ai_oauth.refresh_token, "rt-2");
    }

    #[tokio::test]
    async fn thirty_two_concurrent_rejections_of_one_generation_refresh_once() {
        let b = MockBackend::new("sk-ant-oat01-a", 3600);
        let m = Arc::new(LeaseManager::new(b.clone()));
        let first_gen = m.current().await.unwrap().generation;
        let tasks: Vec<_> = (0..32)
            .map(|_| {
                let m = m.clone();
                tokio::spawn(async move { m.invalidate(first_gen).await })
            })
            .collect();
        for t in tasks {
            let l = t.await.unwrap().unwrap();
            assert_eq!(l.access_token.expose(), "sk-ant-oat01-renewed-1");
            assert_ne!(l.generation, first_gen);
        }
        assert_eq!(b.refreshes(), 1);
    }

    #[tokio::test]
    async fn a_rejection_adopts_a_token_another_process_already_renewed() {
        let b = MockBackend::new("sk-ant-oat01-a", 3600);
        let m = LeaseManager::new(b.clone());
        let first_gen = m.current().await.unwrap().generation;
        *b.blob.lock() = blob("sk-ant-oat01-other", "rt-x", 3600);
        let l = m.invalidate(first_gen).await.unwrap();
        assert_eq!(l.access_token.expose(), "sk-ant-oat01-other");
        assert_eq!(b.refreshes(), 0);
    }

    #[tokio::test]
    async fn the_cc_active_account_is_never_refreshed_and_the_stop_is_sticky() {
        let b = MockBackend::new("sk-ant-oat01-a", 60);
        b.cc_owns.store(true, Ordering::SeqCst);
        let m = LeaseManager::new(b.clone());
        assert_eq!(m.current().await.unwrap_err(), UpstreamError::OwnerConflict);
        b.cc_owns.store(false, Ordering::SeqCst);
        assert_eq!(m.current().await.unwrap_err(), UpstreamError::OwnerConflict);
        assert_eq!(b.refreshes(), 0);
    }

    #[tokio::test]
    async fn becoming_cc_active_later_stops_the_next_renewal() {
        let b = MockBackend::new("sk-ant-oat01-a", 3600);
        let m = LeaseManager::new(b.clone());
        let first_gen = m.current().await.unwrap().generation;
        b.cc_owns.store(true, Ordering::SeqCst);
        assert_eq!(
            m.invalidate(first_gen).await.unwrap_err(),
            UpstreamError::OwnerConflict
        );
        assert_eq!(b.refreshes(), 0);
    }

    #[tokio::test]
    async fn an_identity_change_at_renewal_stops_serving() {
        let b = MockBackend::new("sk-ant-oat01-a", 3600);
        let m = LeaseManager::new(b.clone());
        let first_gen = m.current().await.unwrap().generation;
        *b.identity.lock() = identity("someone-else");
        assert_eq!(
            m.invalidate(first_gen).await.unwrap_err(),
            UpstreamError::IdentityChanged
        );
        assert_eq!(
            m.current().await.unwrap_err(),
            UpstreamError::IdentityChanged
        );
    }

    #[tokio::test]
    async fn a_signed_out_slot_is_unavailable_not_refreshed() {
        let b = MockBackend::new("", 0);
        b.blob.lock().claude_ai_oauth.refresh_token.clear();
        let m = LeaseManager::new(b.clone());
        assert_eq!(
            m.current().await.unwrap_err(),
            UpstreamError::CredentialUnavailable
        );
        assert_eq!(b.refreshes(), 0);
    }
}

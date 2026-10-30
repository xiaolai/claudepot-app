//! The `PassThrough` upstream: a published gateway route (plan §9).
//!
//! Anthropic bytes in, Anthropic bytes out, like the subscription
//! handler — with two differences, and only these:
//!
//! - **`model` is rewritten from the route.** A client asks for
//!   `claude-sonnet-4-5`; DeepSeek knows `deepseek-chat`. The client's
//!   model is kept only when it already names one of the route's models;
//!   otherwise a haiku-tier request goes to the route's small/fast model
//!   and everything else to its primary model — the same two slots the
//!   route's wrapper fills with `ANTHROPIC_MODEL` and
//!   `ANTHROPIC_SMALL_FAST_MODEL`. The body is re-encoded to do it, so
//!   top-level key order is not preserved; nothing inside a value moves.
//! - **The credential is the route's key**, read through
//!   [`super::route_secret::RouteKey`] on every request — `routes.json`
//!   for a plaintext key, the keychain otherwise — so a key rotated while
//!   the relay serves is used by the next request. It is sent as
//!   `Authorization: Bearer`, as CC's CLI sends `ANTHROPIC_AUTH_TOKEN`
//!   whatever the route's Desktop auth scheme says. No OAuth beta is
//!   added: that is an Anthropic-subscription header.
//!
//! The origin is the route's `base_url`, fixed when the handler is built;
//! an edit to the route reaches a serving relay only through a restart,
//! which recomputes the target digest and makes every device accept the
//! change (`acceptance_required`) before anything reaches the new target.
//! A route upstream is never priced: `PriceBook` is keyed by model name,
//! and a provider's model name says nothing about its price.
//!
//! **Credential health recovers on its own.** A failed read or an upstream
//! `401` is recorded in [`CredentialHealth`] and health reports it; while
//! it stands, `status()` re-reads the credential in the background at most
//! once per [`REPROBE_AFTER`]. Without that, a keychain that was locked
//! for a moment would keep the upstream unready forever: clients route
//! around an unready upstream, so no request would arrive to clear it. A
//! rejected credential clears when the key changes, when the upstream
//! accepts it, or after [`REJECTION_HOLD`].
//!
//! **Results apply in the order their reads began.** Every read takes a
//! [`ReadTicket`] when it starts; a read, or a rejection of what a read
//! returned, that began before the newest result already applied is
//! stale. Without that, a probe of the old key finishing after a request
//! read the new key and saw it rejected would clear the rejection.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{header, HeaderMap, StatusCode};
use sha2::{Digest, Sha256};

use super::cancel::CancelToken;
use super::forward::{
    allowlisted_headers, bearer, build_request, execute, relay_client, Cooldown, Timeouts,
};
use super::health::UpstreamStatus;
use super::identity::{CredentialIdentity, TargetKind, TargetSpec};
use super::route_secret::{KeychainReader, RouteKey, RouteSecretError};
use super::secret::Secret;
use super::upstream::{
    AdmittedRequest, Capabilities, Operation, OutcomeFuture, RequestRefusal, TargetIdentity,
    UpstreamError, UpstreamHandler, UpstreamResponse,
};
use crate::routes::{Route, RouteProvider, SecretField};

/// The model the route answers `requested` with.
pub fn map_model<'a>(route: &'a Route, requested: Option<&str>) -> &'a str {
    if let Some(r) = requested {
        if let Some(own) = route_models(route).find(|m| *m == r) {
            return own;
        }
        if r.to_ascii_lowercase().contains("haiku") {
            return route.small_fast_model.as_deref().unwrap_or(&route.model);
        }
    }
    &route.model
}

fn route_models(route: &Route) -> impl Iterator<Item = &str> {
    std::iter::once(route.model.as_str())
        .chain(route.small_fast_model.as_deref())
        .chain(route.additional_models.iter().map(String::as_str))
}

/// What decides where a route's requests go and who pays: kind, origin,
/// the model mapping and the key's *identity* — where the key is kept,
/// never the key. So rotating a key in place keeps the target, and moving
/// it (plaintext ↔ keychain) or editing the origin or a model changes it.
pub fn route_target_spec(route: &Route) -> Result<TargetSpec, RouteSecretError> {
    let RouteProvider::Gateway(cfg) = &route.provider else {
        return Err(RouteSecretError::NotRelayCapable(route.id));
    };
    // Two key spaces that cannot collide: the fallback slots are bare
    // (`*`, `haiku`) and every model kept as itself is `=`-prefixed. Shared
    // keys let a route model literally named `haiku` overwrite the haiku
    // slot, so moving that slot left the digest unchanged.
    let mut model_map = BTreeMap::new();
    model_map.insert("*".to_owned(), route.model.clone());
    model_map.insert(
        "haiku".to_owned(),
        route
            .small_fast_model
            .clone()
            .unwrap_or_else(|| route.model.clone()),
    );
    for m in route_models(route) {
        model_map.insert(format!("={m}"), m.to_owned());
    }
    let key_id = if cfg.use_keychain {
        format!(
            "keychain:{}",
            crate::routes::keychain_account(route.id, SecretField::GatewayApiKey)
        )
    } else {
        format!("plaintext:{}", route.id)
    };
    Ok(TargetSpec {
        kind: TargetKind::PassThrough,
        origin: origin_of(&cfg.base_url),
        model_map,
        credential: CredentialIdentity::Key {
            key_id_digest: hex::encode(Sha256::digest(key_id.as_bytes())),
        },
        protocol: None,
    })
}

fn origin_of(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_owned()
}

/// Rewrite the body's top-level `model`. A body that is not a JSON
/// object is refused before dispatch rather than forwarded unmapped.
fn rewrite_model(body: &[u8], route: &Route) -> Result<Vec<u8>, UpstreamError> {
    let refuse = UpstreamError::Refused(RequestRefusal::ContentUnsupported);
    let mut v: serde_json::Value = serde_json::from_slice(body).map_err(|_| refuse)?;
    let obj = v.as_object_mut().ok_or(refuse)?;
    let mapped = map_model(route, obj.get("model").and_then(|m| m.as_str())).to_owned();
    obj.insert("model".to_owned(), serde_json::Value::String(mapped));
    serde_json::to_vec(&v).map_err(|_| refuse)
}

/// How long a recorded credential failure stands before `status()`
/// re-reads the credential.
pub const REPROBE_AFTER: Duration = Duration::from_secs(30);
/// How long an upstream rejection of an unchanged credential stands
/// before a re-read clears it, so a provider's transient `401` does not
/// keep the upstream unready until a restart.
pub const REJECTION_HOLD: Duration = Duration::from_secs(300);

/// What a credential read found: its failure, or a digest of the
/// credential — never the credential.
pub(crate) type CredentialRead = Result<[u8; 32], UpstreamError>;

pub(crate) fn credential_digest(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

/// A route handler's credential state, as health reports it. Shared by the
/// pass-through and the translated handler; see the module docs.
pub(crate) struct CredentialHealth {
    reprobe_after: Duration,
    rejection_hold: Duration,
    state: parking_lot::Mutex<HealthState>,
}

#[derive(Default)]
struct HealthState {
    error: Option<UpstreamError>,
    /// The digest of the credential the upstream rejected, and when.
    rejected: Option<([u8; 32], Instant)>,
    last_read: Option<Instant>,
    probing: bool,
    /// The last ticket issued.
    issued: u64,
    /// The ticket of the newest result applied, and the digest that read
    /// found (`None` for a failed read).
    applied: u64,
    applied_digest: Option<[u8; 32]>,
}

/// When a credential read began, relative to the others; see the module
/// docs. Taken with [`CredentialHealth::begin`] before the read starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ReadTicket(u64);

impl Default for CredentialHealth {
    fn default() -> Self {
        Self::new(REPROBE_AFTER, REJECTION_HOLD)
    }
}

impl CredentialHealth {
    pub(crate) fn new(reprobe_after: Duration, rejection_hold: Duration) -> Self {
        Self {
            reprobe_after,
            rejection_hold,
            state: parking_lot::Mutex::new(HealthState::default()),
        }
    }

    pub(crate) fn error(&self) -> Option<UpstreamError> {
        self.state.lock().error
    }

    /// A ticket for a read about to start.
    pub(crate) fn begin(&self) -> ReadTicket {
        let mut s = self.state.lock();
        s.issued += 1;
        ReadTicket(s.issued)
    }

    /// The read with `ticket` finished. A failure stands until a read
    /// succeeds; a success clears everything except a rejection of this
    /// same credential that is younger than the hold. A read that began
    /// before the newest applied result is stale and changes nothing.
    pub(crate) fn read(&self, ticket: ReadTicket, r: CredentialRead, now: Instant) {
        let mut s = self.state.lock();
        if ticket.0 < s.applied {
            return;
        }
        s.applied = ticket.0;
        s.applied_digest = r.as_ref().ok().copied();
        s.last_read = Some(now);
        match r {
            Err(e) => {
                s.error = Some(e);
                s.rejected = None;
            }
            Ok(digest) => {
                let held = s.rejected.is_some_and(|(d, at)| {
                    d == digest && now.saturating_duration_since(at) < self.rejection_hold
                });
                if !held {
                    s.error = None;
                    s.rejected = None;
                }
            }
        }
    }

    /// The upstream answered `401` to the credential with `digest`, which
    /// the read with `ticket` returned. Stale when a newer read already
    /// found a different credential: that one has not been rejected.
    pub(crate) fn rejected(&self, ticket: ReadTicket, digest: [u8; 32], now: Instant) {
        let mut s = self.state.lock();
        if ticket.0 < s.applied && s.applied_digest != Some(digest) {
            return;
        }
        s.applied = s.applied.max(ticket.0);
        s.error = Some(UpstreamError::CredentialRejected);
        s.rejected = Some((digest, now));
    }

    /// The upstream accepted the credential with `digest`. Only a
    /// rejection of that same credential is cleared: a read failure
    /// standing now was recorded after this request's own read, and a
    /// rejection of a different credential — one another request read
    /// later — is newer news. Both stand.
    pub(crate) fn accepted_credential(&self, digest: [u8; 32]) {
        let mut s = self.state.lock();
        if s.rejected.is_some_and(|(d, _)| d == digest) {
            s.error = None;
            s.rejected = None;
        }
    }

    /// Claim the one background re-read: only while a failure stands,
    /// none is running, and the last read is older than the interval.
    pub(crate) fn claim_probe(&self, now: Instant) -> bool {
        let mut s = self.state.lock();
        let due = s
            .last_read
            .is_none_or(|t| now.saturating_duration_since(t) >= self.reprobe_after);
        if s.error.is_none() || s.probing || !due {
            return false;
        }
        s.probing = true;
        true
    }

    /// The claimed re-read with `ticket` finished with `r`.
    pub(crate) fn probed(&self, ticket: ReadTicket, r: CredentialRead, now: Instant) {
        self.read(ticket, r, now);
        self.state.lock().probing = false;
    }

    /// What `status()` reports, after starting a re-read with `probe` if
    /// one is due. The re-read runs in the background: health never waits
    /// on a keychain or a Codex app-server, and the next poll sees it.
    pub(crate) fn status<F>(
        self: &Arc<Self>,
        cooldown: Option<Duration>,
        probe: F,
    ) -> UpstreamStatus
    where
        F: std::future::Future<Output = CredentialRead> + Send + 'static,
    {
        if self.claim_probe(Instant::now()) {
            let health = self.clone();
            let ticket = self.begin();
            tokio::spawn(async move {
                let r = probe.await;
                health.probed(ticket, r, Instant::now());
            });
        }
        let credential_error = self.error();
        UpstreamStatus {
            credential_ready: credential_error.is_none(),
            credential_error,
            cooldown,
            ..UpstreamStatus::default()
        }
    }
}

/// Read `key` for a request or a probe, logging a failure without the key.
async fn read_key(key: &RouteKey) -> Result<Option<Secret>, UpstreamError> {
    key.read().await.map_err(|e| {
        tracing::warn!(route = %key.route_id(), error = %e, "relay route secret unavailable");
        UpstreamError::CredentialUnavailable
    })
}

/// The digest [`CredentialHealth`] tracks for a route key; an empty key
/// (a local server that takes any token) digests as the empty string.
pub(crate) fn key_digest(key: &Option<Secret>) -> [u8; 32] {
    credential_digest(key.as_ref().map_or("", |k| k.expose()))
}

/// A route key read, reduced to what health needs.
pub(crate) async fn probe_key(key: RouteKey) -> CredentialRead {
    read_key(&key).await.map(|k| key_digest(&k))
}

pub struct PassThroughHandler {
    target: TargetIdentity,
    caps: Capabilities,
    route: Route,
    origin: String,
    client: reqwest::Client,
    timeouts: Timeouts,
    key: RouteKey,
    cooldown: Cooldown,
    health: Arc<CredentialHealth>,
}

impl PassThroughHandler {
    /// The production handler: the OS keychain, and `routes_file` (the
    /// serving data dir's `routes.json`) re-read on every request.
    pub fn new(
        target: TargetIdentity,
        route: Route,
        routes_file: std::path::PathBuf,
    ) -> reqwest::Result<Self> {
        let timeouts = Timeouts::default();
        let key = RouteKey::production(route.clone(), routes_file);
        Ok(Self::with_key(
            target,
            route,
            relay_client(&timeouts)?,
            timeouts,
            key,
        ))
    }

    /// Everything injectable, for tests against a loopback gateway. A
    /// plaintext key is the given route's; see [`Self::with_key`] for one
    /// re-read from a route store.
    pub fn with_client(
        target: TargetIdentity,
        route: Route,
        client: reqwest::Client,
        timeouts: Timeouts,
        keychain: Arc<dyn KeychainReader + Send + Sync>,
    ) -> Self {
        let key = RouteKey::new(route.clone(), keychain);
        Self::with_key(target, route, client, timeouts, key)
    }

    /// [`Self::with_client`] with the key source given whole.
    pub fn with_key(
        target: TargetIdentity,
        route: Route,
        client: reqwest::Client,
        timeouts: Timeouts,
        key: RouteKey,
    ) -> Self {
        let origin = match &route.provider {
            RouteProvider::Gateway(cfg) => origin_of(&cfg.base_url),
            _ => String::new(),
        };
        Self {
            target,
            caps: Capabilities {
                operations: [Operation::Messages, Operation::CountTokens].into(),
            },
            route,
            origin,
            client,
            timeouts,
            key,
            cooldown: Cooldown::default(),
            health: Arc::default(),
        }
    }

    /// Read the key once, as a start check; health reports the result
    /// until the next read.
    pub async fn check_credential(&self) -> Result<(), UpstreamError> {
        self.secret().await.map(|_| ())
    }

    /// The key, and the ticket its read took.
    async fn secret(&self) -> Result<(Option<Secret>, ReadTicket), UpstreamError> {
        let ticket = self.health.begin();
        let r = read_key(&self.key).await;
        self.health.read(
            ticket,
            r.as_ref().map(key_digest).map_err(|e| *e),
            Instant::now(),
        );
        r.map(|k| (k, ticket))
    }

    fn request(
        &self,
        req: &AdmittedRequest,
        body: Vec<u8>,
        key: Option<&Secret>,
    ) -> Result<reqwest::Request, UpstreamError> {
        let mut headers: HeaderMap = allowlisted_headers(&req.headers);
        if let Some(k) = key {
            headers.insert(header::AUTHORIZATION, bearer(k.expose())?);
        }
        build_request(&self.client, &self.origin, req, headers, body)
    }
}

#[async_trait::async_trait]
impl UpstreamHandler for PassThroughHandler {
    fn target(&self) -> &TargetIdentity {
        &self.target
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    async fn status(&self) -> UpstreamStatus {
        self.health
            .status(self.cooldown.remaining(), probe_key(self.key.clone()))
    }

    async fn handle(
        &self,
        req: AdmittedRequest,
        _cancel: CancelToken,
    ) -> Result<(UpstreamResponse, OutcomeFuture), UpstreamError> {
        let body = rewrite_model(&req.body.bytes, &self.route)?;
        let (key, ticket) = self.secret().await?;
        let request = self.request(&req, body, key.as_ref())?;
        let started = Instant::now();
        let resp = execute(&self.client, request, self.timeouts.response_header).await?;
        let status = resp.status();
        // A route key has no renewal, so there is nothing to retry with.
        if status == StatusCode::UNAUTHORIZED {
            self.health
                .rejected(ticket, key_digest(&key), Instant::now());
            return Err(UpstreamError::CredentialRejected);
        }
        self.health.accepted_credential(key_digest(&key));
        if status == StatusCode::TOO_MANY_REQUESTS {
            self.cooldown.start(resp.headers());
        }
        Ok(super::forward::stream_response(
            resp,
            started,
            req.operation,
        ))
    }
}

#[cfg(test)]
#[path = "tests_passthrough.rs"]
mod tests;

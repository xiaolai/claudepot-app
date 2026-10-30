//! The forward handler: Anthropic bytes in, Anthropic bytes out.
//!
//! P1's only upstream kind is `Subscription`: the request body is sent
//! unchanged with the leased OAuth token, and the response is streamed
//! back unchanged — no re-encoding, no buffering of the response.
//!
//! ## Boundary
//!
//! - The origin is a constant per upstream, never derived from the
//!   request. Redirects are disabled; a 3xx is `upstream_redirect`.
//! - Request headers cross through an allowlist (P0's observed set).
//!   The client's `Authorization` is the relay device token and is
//!   never forwarded; `x-api-key` is never forwarded either.
//! - The credential header is marked sensitive.
//!
//! ## Timeouts (plan §4.3)
//!
//! Connect 15 s; 120 s for response headers; a **resetting** 10-minute
//! read-idle on the body; no total deadline, because a long thinking
//! turn streams for as long as it needs.

use std::time::{Duration, Instant};

use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use futures_util::StreamExt;
use tokio::sync::oneshot;

use super::cancel::CancelToken;
use super::health::{OwnerState, UpstreamStatus, UtilizationSource};
use super::lease::LeaseManager;
use super::sse::{Mode, UsageTap};
use super::upstream::{
    AdmittedRequest, Capabilities, Operation, Outcome, OutcomeFuture, OutcomeKind, ProviderUsage,
    TargetIdentity, UpstreamError, UpstreamHandler, UpstreamResponse,
};
use std::sync::Arc;

pub const ANTHROPIC_ORIGIN: &str = "https://api.anthropic.com";
pub const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Request headers that may reach the upstream.
const FORWARDED: &[&str] = &[
    "anthropic-version",
    "anthropic-beta",
    "content-type",
    "accept",
    "user-agent",
    "x-app",
    "x-claude-code-session-id",
];
const FORWARDED_PREFIX: &str = "x-stainless-";

/// Response headers that describe this hop, not the payload.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
    "set-cookie",
];

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub response_header: Duration,
    pub read_idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            response_header: Duration::from_secs(120),
            read_idle: Duration::from_secs(600),
        }
    }
}

/// The pooled client for relay traffic. Not `oauth::http_client()`,
/// whose 15 s total timeout would cut a long turn off.
pub fn relay_client(t: &Timeouts) -> reqwest::Result<reqwest::Client> {
    let builder = base_builder(t);
    crate::proxy::apply(builder, &crate::proxy::detect()).build()
}

/// The builder every relay client starts from. No `.timeout()` — a
/// total deadline is exactly what a streamed turn must not have.
pub(crate) fn base_builder(t: &Timeouts) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(t.connect)
        .read_timeout(t.read_idle)
}

pub struct SubscriptionHandler {
    target: TargetIdentity,
    caps: Capabilities,
    lease: Arc<LeaseManager>,
    client: reqwest::Client,
    origin: String,
    timeouts: Timeouts,
    /// Until when the provider asked us to back off (its last 429).
    cooldown: Cooldown,
    utilization: Option<UtilizationSource>,
}

/// A 429 without a usable `retry-after` cools down for this long, and
/// none for longer than [`MAX_COOLDOWN`].
pub const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);
pub const MAX_COOLDOWN: Duration = Duration::from_secs(60 * 60);

pub(crate) fn cooldown_for(headers: &HeaderMap) -> Duration {
    headers
        .get(header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(DEFAULT_COOLDOWN, Duration::from_secs)
        .min(MAX_COOLDOWN)
}

/// Until when an upstream asked to be left alone (a 429's
/// `Retry-After`), shared by every handler that honours one.
#[derive(Debug, Default)]
pub(crate) struct Cooldown(parking_lot::Mutex<Option<Instant>>);

impl Cooldown {
    /// Time left, clearing an expired cooldown.
    pub(crate) fn remaining(&self) -> Option<Duration> {
        let mut c = self.0.lock();
        let left = c.and_then(|until| until.checked_duration_since(Instant::now()));
        if left.is_none() {
            *c = None;
        }
        left
    }

    /// Start one from a 429's headers.
    pub(crate) fn start(&self, headers: &HeaderMap) {
        *self.0.lock() = Some(Instant::now() + cooldown_for(headers));
    }
}

impl SubscriptionHandler {
    pub fn new(target: TargetIdentity, lease: Arc<LeaseManager>) -> reqwest::Result<Self> {
        let timeouts = Timeouts::default();
        Ok(Self::with_client(
            target,
            lease,
            relay_client(&timeouts)?,
            ANTHROPIC_ORIGIN.to_owned(),
            timeouts,
        ))
    }

    /// Everything injectable, for tests that point the handler at a
    /// loopback upstream.
    pub fn with_client(
        target: TargetIdentity,
        lease: Arc<LeaseManager>,
        client: reqwest::Client,
        origin: String,
        timeouts: Timeouts,
    ) -> Self {
        Self {
            target,
            caps: Capabilities {
                operations: [Operation::Messages, Operation::CountTokens].into(),
            },
            lease,
            client,
            origin,
            timeouts,
            cooldown: Cooldown::default(),
            utilization: None,
        }
    }

    pub fn with_utilization(mut self, source: UtilizationSource) -> Self {
        self.utilization = Some(source);
        self
    }

    fn request(
        &self,
        req: &AdmittedRequest,
        token: &str,
    ) -> Result<reqwest::Request, UpstreamError> {
        build_request(
            &self.client,
            &self.origin,
            req,
            outbound_headers(&req.headers, token)?,
            req.body.bytes.clone(),
        )
    }
}

/// `POST <origin><operation path>[?beta=true]` with `headers` and `body`.
pub(crate) fn build_request(
    client: &reqwest::Client,
    origin: &str,
    req: &AdmittedRequest,
    headers: HeaderMap,
    body: impl Into<reqwest::Body>,
) -> Result<reqwest::Request, UpstreamError> {
    let mut url = format!("{origin}{}", req.operation.path());
    if req.beta {
        url.push_str("?beta=true");
    }
    client
        .post(url)
        .headers(headers)
        .body(body)
        .build()
        .map_err(|_| UpstreamError::Transport)
}

/// Send `request`, waiting at most `response_header` for the headers. A
/// redirect is `upstream_redirect`: the origin is fixed, never followed.
pub(crate) async fn execute(
    client: &reqwest::Client,
    request: reqwest::Request,
    response_header: Duration,
) -> Result<reqwest::Response, UpstreamError> {
    let resp = match tokio::time::timeout(response_header, client.execute(request)).await {
        Err(_) => return Err(UpstreamError::Timeout),
        Ok(Err(e)) if e.is_timeout() => return Err(UpstreamError::Timeout),
        Ok(Err(_)) => return Err(UpstreamError::Transport),
        Ok(Ok(r)) => r,
    };
    if resp.status().is_redirection() {
        return Err(UpstreamError::Redirect);
    }
    Ok(resp)
}

/// The client headers that may cross to any upstream — nothing else,
/// so the device token and `x-api-key` never do.
pub(crate) fn allowlisted_headers(client: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in client {
        let n = name.as_str();
        if FORWARDED.contains(&n) || n.starts_with(FORWARDED_PREFIX) {
            out.append(name.clone(), value.clone());
        }
    }
    out
}

/// `Authorization: Bearer <token>`, marked sensitive.
pub(crate) fn bearer(token: &str) -> Result<HeaderValue, UpstreamError> {
    let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| UpstreamError::CredentialUnavailable)?;
    auth.set_sensitive(true);
    Ok(auth)
}

/// The allowlisted client headers plus the upstream credential.
pub(crate) fn outbound_headers(
    client: &HeaderMap,
    token: &str,
) -> Result<HeaderMap, UpstreamError> {
    let mut out = allowlisted_headers(client);
    // The OAuth beta must be present, once, beside whatever CC asked for.
    let mut betas: Vec<String> = out
        .get_all("anthropic-beta")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if !betas.iter().any(|b| b == OAUTH_BETA) {
        betas.push(OAUTH_BETA.to_owned());
    }
    out.remove("anthropic-beta");
    out.insert(
        HeaderName::from_static("anthropic-beta"),
        HeaderValue::from_str(&betas.join(",")).map_err(|_| UpstreamError::Transport)?,
    );
    out.insert(header::AUTHORIZATION, bearer(token)?);
    Ok(out)
}

/// The upstream's response headers minus those describing this hop: the
/// fixed set, and every header its `Connection` nominates (RFC 9110 §7.6.1).
pub(crate) fn inbound_headers(upstream: &HeaderMap) -> HeaderMap {
    let nominated: Vec<String> = upstream
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let mut out = HeaderMap::new();
    for (name, value) in upstream {
        let n = name.as_str();
        if !HOP_BY_HOP.contains(&n) && !nominated.iter().any(|c| c == n) {
            out.append(name.clone(), value.clone());
        }
    }
    out
}

#[async_trait::async_trait]
impl UpstreamHandler for SubscriptionHandler {
    fn target(&self) -> &TargetIdentity {
        &self.target
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    async fn status(&self) -> UpstreamStatus {
        // A transient acquisition failure is retried here, in the
        // background: clients route around an unready upstream, so no
        // request would arrive to clear it. The next poll sees the result.
        if self.lease.claim_recovery(Instant::now()) {
            let lease = self.lease.clone();
            tokio::spawn(async move { lease.recover().await });
        }
        let r = self.lease.readiness();
        let cooldown = self.cooldown.remaining();
        UpstreamStatus {
            credential_ready: r.ready(),
            credential_error: r.stopped.or(r.last_error),
            owner_state: Some(match r.stopped {
                Some(UpstreamError::OwnerConflict) => OwnerState::Conflict,
                _ => OwnerState::Clear,
            }),
            utilization: self
                .utilization
                .as_ref()
                .and_then(|f| f(chrono::Utc::now())),
            cooldown,
            account: r.identity.map(|i| (i.account_uuid, i.org_uuid)),
        }
    }

    async fn handle(
        &self,
        req: AdmittedRequest,
        _cancel: CancelToken,
    ) -> Result<(UpstreamResponse, OutcomeFuture), UpstreamError> {
        // Cancellation before headers is the caller dropping this future,
        // which drops the in-flight reqwest request with it.
        let mut lease = self.lease.current().await?;
        let mut retried = false;
        loop {
            let started = Instant::now();
            let request = self.request(&req, lease.access_token.expose())?;
            let resp = execute(&self.client, request, self.timeouts.response_header).await?;
            let status = resp.status();
            if status == StatusCode::UNAUTHORIZED {
                // Nothing has reached the client yet, and the body is
                // buffered, so one retry with a renewed token is safe.
                if retried {
                    self.lease.rejected();
                    return Err(UpstreamError::CredentialRejected);
                }
                retried = true;
                drop(resp);
                lease = self.lease.invalidate(lease.generation).await?;
                continue;
            }
            self.lease.accepted();
            if status == StatusCode::TOO_MANY_REQUESTS {
                self.cooldown.start(resp.headers());
            }
            return Ok(stream_response(resp, started, req.operation));
        }
    }
}

/// How a stream ended, before the tap's view refines it.
#[derive(Clone, Copy)]
enum End {
    /// Upstream EOF.
    Eof,
    /// A transport error, an idle timeout, or the body dropped first.
    Cut,
}

/// Resolves the outcome exactly once: at end of stream, at an error, or
/// — if the body is dropped first, i.e. the client went away — on drop.
struct Reporter {
    tx: Option<oneshot::Sender<Outcome>>,
    outcome: Outcome,
    started: Instant,
    success: bool,
    tap: Option<UsageTap>,
    native: Option<NativeSlot>,
}

impl Reporter {
    fn feed(&mut self, chunk: &[u8]) {
        if self.outcome.ttfb.is_none() {
            self.outcome.ttfb = Some(self.started.elapsed());
        }
        self.outcome.bytes += chunk.len() as u64;
        if let Some(tap) = self.tap.as_mut() {
            tap.feed(chunk);
        }
    }

    fn finish(&mut self, end: End) {
        let Some(tx) = self.tx.take() else {
            return;
        };
        let (mode, tapped) = match self.tap.take() {
            Some(tap) => (tap.mode(), tap.finish()),
            None => (Mode::Off, Default::default()),
        };
        self.outcome.kind = match (self.success, end) {
            (false, _) => OutcomeKind::Failed,
            (true, End::Cut) => OutcomeKind::Partial,
            (true, End::Eof) => tapped.success_kind(mode),
        };
        self.outcome.usage = tapped.usage;
        self.outcome.native_usage = self.native.as_ref().and_then(|n| *n.lock());
        self.outcome.model = tapped.model;
        self.outcome.error = tapped.error;
        let _ = tx.send(self.outcome.clone());
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        self.finish(End::Cut);
    }
}

pub(crate) fn stream_response(
    resp: reqwest::Response,
    started: Instant,
    operation: Operation,
) -> (UpstreamResponse, OutcomeFuture) {
    let status = resp.status();
    let headers = inbound_headers(resp.headers());
    let mode = match operation {
        // Not inference: nothing to account.
        Operation::CountTokens => Mode::Off,
        _ => Mode::for_content_type(
            resp.headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
        ),
    };
    let request_id = resp
        .headers()
        .get("request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    report_stream(
        resp.bytes_stream(),
        Reported {
            status,
            headers,
            request_id,
            mode,
            started,
            native: None,
        },
    )
}

/// What [`report_stream`] needs to know about a response besides its body.
pub(crate) struct Reported {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub request_id: Option<String>,
    /// How the body is read for usage: a translated handler passes
    /// `Sse` for the Anthropic stream it produces.
    pub mode: Mode,
    pub started: Instant,
    /// Where a translated handler leaves the provider's own counters by
    /// the time the body ends; `None` for a forwarded body.
    pub native: Option<NativeSlot>,
}

/// Filled by the translate pipeline, read once when the outcome resolves.
pub(crate) type NativeSlot = Arc<parking_lot::Mutex<Option<ProviderUsage>>>;

/// Any Anthropic-shaped body as an [`UpstreamResponse`] whose outcome
/// resolves once — at the end, at an error, or when the client drops it.
/// The forward handlers pass the provider's bytes; the translate handler
/// passes the Anthropic stream it writes, so both are accounted by the
/// same tap and the ledger never parses a provider's format.
pub(crate) fn report_stream<S, E>(inner: S, r: Reported) -> (UpstreamResponse, OutcomeFuture)
where
    S: futures_util::Stream<Item = Result<axum::body::Bytes, E>> + Send + 'static,
    E: Send + 'static,
{
    let Reported {
        status,
        headers,
        request_id,
        mode,
        started,
        native,
    } = r;
    let (tx, rx) = oneshot::channel();
    let reporter = Reporter {
        tx: Some(tx),
        outcome: Outcome {
            kind: OutcomeKind::Partial,
            status: Some(status.as_u16()),
            ttfb: None,
            upstream_request_id: request_id,
            bytes: 0,
            usage: None,
            native_usage: None,
            model: None,
            error: None,
        },
        started,
        success: status.is_success(),
        tap: Some(UsageTap::new(mode)),
        native,
    };
    let body = futures_util::stream::unfold(
        (Box::pin(inner), reporter),
        move |(mut inner, mut rep)| async move {
            match inner.next().await {
                Some(Ok(chunk)) => {
                    rep.feed(&chunk);
                    Some((Ok(chunk), (inner, rep)))
                }
                Some(Err(_)) => {
                    // An idle timeout or a reset mid-stream. The error
                    // text stays here; the client sees the stream abort.
                    rep.finish(End::Cut);
                    Some((
                        Err(std::io::Error::other("upstream stream failed")),
                        (inner, rep),
                    ))
                }
                None => {
                    rep.finish(End::Eof);
                    None
                }
            }
        },
    )
    .boxed();
    (
        UpstreamResponse {
            status,
            headers,
            body,
        },
        rx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_honours_retry_after_seconds_within_the_cap() {
        let with = |v: &str| {
            let mut h = HeaderMap::new();
            h.insert("retry-after", v.parse().unwrap());
            cooldown_for(&h)
        };
        assert_eq!(cooldown_for(&HeaderMap::new()), DEFAULT_COOLDOWN);
        assert_eq!(with("7"), Duration::from_secs(7));
        assert_eq!(with("999999"), MAX_COOLDOWN);
        // The HTTP-date form and junk fall back to the default.
        assert_eq!(with("Wed, 21 Oct 2026 07:28:00 GMT"), DEFAULT_COOLDOWN);
        assert_eq!(with("-5"), DEFAULT_COOLDOWN);
    }

    #[test]
    fn only_allowlisted_headers_cross_and_the_device_token_never_does() {
        let mut h = HeaderMap::new();
        h.insert("authorization", "Bearer cprl_device".parse().unwrap());
        h.insert("x-api-key", "sk-ant-api03-x".parse().unwrap());
        h.insert("cookie", "a=b".parse().unwrap());
        h.insert("anthropic-version", "2023-06-01".parse().unwrap());
        h.insert("x-stainless-os", "MacOS".parse().unwrap());
        h.insert("x-claude-code-session-id", "s".parse().unwrap());
        let out = outbound_headers(&h, "sk-ant-oat01-lease").unwrap();
        assert!(out.get("x-api-key").is_none());
        assert!(out.get("cookie").is_none());
        assert_eq!(out["authorization"], "Bearer sk-ant-oat01-lease");
        assert!(out["authorization"].is_sensitive());
        assert_eq!(out["anthropic-version"], "2023-06-01");
        assert_eq!(out["x-stainless-os"], "MacOS");
        assert_eq!(out["x-claude-code-session-id"], "s");
        assert_eq!(out["anthropic-beta"], OAUTH_BETA);
    }

    #[test]
    fn the_oauth_beta_is_merged_once_with_the_clients_betas() {
        let mut h = HeaderMap::new();
        h.insert(
            "anthropic-beta",
            "claude-code-20250219,oauth-2025-04-20".parse().unwrap(),
        );
        h.append(
            "anthropic-beta",
            "interleaved-thinking-2025-05-14".parse().unwrap(),
        );
        let out = outbound_headers(&h, "t").unwrap();
        assert_eq!(
            out["anthropic-beta"],
            "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14"
        );
    }

    #[test]
    fn hop_by_hop_and_cookie_headers_do_not_come_back() {
        let mut h = HeaderMap::new();
        h.insert("set-cookie", "x".parse().unwrap());
        h.insert("transfer-encoding", "chunked".parse().unwrap());
        h.insert("content-length", "5".parse().unwrap());
        h.insert("content-type", "text/event-stream".parse().unwrap());
        h.insert("request-id", "req_1".parse().unwrap());
        let out = inbound_headers(&h);
        assert_eq!(out.len(), 2);
        assert_eq!(out["content-type"], "text/event-stream");
    }

    #[tokio::test]
    async fn status_polling_alone_recovers_a_transiently_failed_lease() {
        use super::super::identity::TargetKind;
        use super::super::lease::mock::MockBackend;
        use std::sync::atomic::Ordering;

        let b = MockBackend::new("sk-ant-oat01-a", 3600);
        b.failing_identities.store(1, Ordering::SeqCst);
        let lease = Arc::new(LeaseManager::with_recovery(
            b.clone(),
            Duration::ZERO,
            Duration::ZERO,
        ));
        assert!(lease.current().await.is_err());
        let t = Timeouts::default();
        let h = SubscriptionHandler::with_client(
            TargetIdentity {
                id: super::super::upstream::UpstreamId::parse("sub").unwrap(),
                fingerprint: "fp".into(),
                generation: 1,
                kind: TargetKind::Subscription,
            },
            lease,
            base_builder(&t).no_proxy().build().unwrap(),
            "http://127.0.0.1:9".into(),
            t,
        );
        assert!(!h.status().await.credential_ready);
        let mut ready = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            if h.status().await.credential_ready {
                ready = true;
                break;
            }
        }
        assert!(ready, "no request was needed to recover");
    }

    #[test]
    fn headers_the_connection_header_nominates_do_not_come_back() {
        let mut h = HeaderMap::new();
        h.insert("connection", "X-Hop, close".parse().unwrap());
        h.append("connection", "x-other".parse().unwrap());
        h.insert("x-hop", "1".parse().unwrap());
        h.insert("x-other", "2".parse().unwrap());
        h.insert("close", "3".parse().unwrap());
        h.insert("request-id", "req_1".parse().unwrap());
        let out = inbound_headers(&h);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out["request-id"], "req_1");
    }
}

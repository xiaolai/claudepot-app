//! The relay listener and its request pipeline.
//!
//! `parse → auth → admission → grant → body → handler → stream`, in
//! one axum fallback so the order is visible in one function. Hello is
//! answered before auth, because CC sends it with no credential; every
//! other refusal comes *after* auth, so an unauthenticated caller learns
//! nothing about which paths or upstreams exist.
//!
//! Admission permits and the device's cancel token move into the
//! response body: a permit is released when the stream ends or is
//! dropped, not when headers are sent, and revoking a device ends its
//! in-flight streams.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::StreamExt;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use hyper_util::service::TowerToHyperService;
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_rustls::TlsAcceptor;

use super::cancel::CancelToken;
use super::devices::{DeviceRecord, DeviceRegistry, Grant};
use super::identity;
use super::pairing::{PairError, Pairing};
use super::refusal::Refusal;
use super::serve_lock::ServeLock;
use super::target::{self, Parsed, Target, UpstreamId, UpstreamSel};
use super::upstream::{
    AdmittedRequest, BufferedBody, Operation, Outcome, OutcomeKind, TargetIdentity,
    UpstreamHandler, UpstreamResponse,
};

/// The outcome of a request that ended before any response header, named
/// by its fixed refusal code, never by upstream text.
fn before_headers(kind: OutcomeKind, refusal: Refusal) -> Outcome {
    Outcome {
        kind,
        status: None,
        ttfb: None,
        upstream_request_id: None,
        bytes: 0,
        usage: None,
        native_usage: None,
        model: None,
        error: Some(refusal.code().to_owned()),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub per_device: usize,
    pub global: usize,
    pub max_body: usize,
    pub body_budget: usize,
    /// A request body that sends nothing for this long is abandoned.
    pub body_idle: Duration,
    pub tls_handshake: Duration,
    /// How long a connection may keep finishing its in-flight response
    /// after shutdown before it is cut. Stop waits at most this long.
    pub shutdown_drain: Duration,
    /// Open connections, authenticated or not. Admission limits count
    /// requests, so without this an idle socket costs a task and a
    /// descriptor that nothing bounds. At the cap the listener stops
    /// accepting; the kernel backlog holds the rest.
    pub max_connections: usize,
    /// A connection must deliver a complete request head within this,
    /// including between keep-alive requests, or it is closed.
    pub header_read: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            per_device: 8,
            global: 32,
            max_body: 32 * 1024 * 1024,
            body_budget: 256 * 1024 * 1024,
            body_idle: Duration::from_secs(30),
            tls_handshake: Duration::from_secs(10),
            shutdown_drain: Duration::from_secs(5),
            max_connections: 256,
            header_read: Duration::from_secs(30),
        }
    }
}

/// The listener's shutdown signal, carried on every request it serves so
/// the pipeline can refuse to dispatch once Stop has been asked for.
#[derive(Clone)]
struct Stopping(CancelToken);

/// What the ledger (P3) receives for every dispatched request.
#[derive(Debug, Clone)]
pub struct OutcomeRecord {
    pub device_id: String,
    pub target: TargetIdentity,
    pub operation: Operation,
    /// The body's top-level `model`, as the client asked for it — a mapped
    /// route answers with a different one, recorded in `outcome.model`.
    pub requested_model: Option<String>,
    pub outcome: Outcome,
}

pub type OutcomeSink = Arc<dyn Fn(OutcomeRecord) + Send + Sync>;

/// A control request body is a few short strings.
const MAX_CONTROL_BODY: usize = 4 * 1024;
const MAX_DEVICE_NAME: usize = 64;

/// What the pairing endpoints serve. Present only on a TLS listener:
/// pairing returns a bearer token, which never crosses plaintext. Every
/// served relay is TLS (`listen::check`), so in practice it is always set.
pub struct ControlPlane {
    pub server_id: String,
    pub ca_pem: String,
    pub ca_fingerprint: String,
    pub pairing: Pairing,
}

pub struct RelayState {
    devices: DeviceRegistry,
    /// Swappable while serving: republishing a changed target replaces
    /// its handler under a new generation, and the next request from a
    /// device that accepted the old one is refused.
    upstreams: parking_lot::RwLock<HashMap<UpstreamId, Arc<dyn UpstreamHandler>>>,
    /// `relay-config.json`'s epoch; a grant from another epoch is stale.
    epoch: String,
    /// What bare `/v1/…` means: the server's subscription upstream.
    default_upstream: Option<UpstreamId>,
    limits: Limits,
    global: Arc<Semaphore>,
    per_device: Mutex<HashMap<String, Arc<Semaphore>>>,
    budget: Arc<Semaphore>,
    sink: Option<OutcomeSink>,
    control: Option<Arc<ControlPlane>>,
    /// `relay-config.json`'s `server_id`, reported by health.
    server_id: String,
    /// Latency samples for health; fed for every outcome, sink or not.
    stats: Arc<super::ledger::Stats>,
    /// The `TrustError` code of the last failed leaf renewal while
    /// serving; cleared by the next one that succeeds.
    trust_fault: parking_lot::Mutex<Option<&'static str>>,
    /// The serve lock of the data dir `devices` persists to. It goes with
    /// the last reference to this state, because any holder can still
    /// write that file — not with the listener.
    _serve_lock: Option<ServeLock>,
}

impl RelayState {
    pub fn new(devices: DeviceRegistry, limits: Limits) -> Self {
        Self {
            devices,
            upstreams: parking_lot::RwLock::new(HashMap::new()),
            epoch: String::new(),
            default_upstream: None,
            global: Arc::new(Semaphore::new(limits.global)),
            per_device: Mutex::new(HashMap::new()),
            budget: Arc::new(Semaphore::new(limits.body_budget)),
            limits,
            sink: None,
            control: None,
            server_id: String::new(),
            stats: Arc::new(super::ledger::Stats::default()),
            trust_fault: parking_lot::Mutex::new(None),
            _serve_lock: None,
        }
    }

    pub fn with_serve_lock(mut self, lock: ServeLock) -> Self {
        self._serve_lock = Some(lock);
        self
    }

    pub fn trust_fault(&self) -> Option<&'static str> {
        *self.trust_fault.lock()
    }

    pub(crate) fn set_trust_fault(&self, code: Option<&'static str>) {
        *self.trust_fault.lock() = code;
    }

    pub fn with_server_id(mut self, id: impl Into<String>) -> Self {
        self.server_id = id.into();
        self
    }

    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn stats(&self) -> &Arc<super::ledger::Stats> {
        &self.stats
    }

    pub fn default_upstream(&self) -> Option<&UpstreamId> {
        self.default_upstream.as_ref()
    }

    pub(crate) fn handlers(&self) -> Vec<Arc<dyn UpstreamHandler>> {
        self.upstreams.read().values().cloned().collect()
    }

    /// Requests holding a global slot right now (streams in flight).
    pub fn active_streams(&self) -> usize {
        self.limits
            .global
            .saturating_sub(self.global.available_permits())
    }

    /// Requests `device_id` could start now without `relay_over_capacity`.
    pub fn capacity_free(&self, device_id: &str) -> usize {
        self.device_semaphore(device_id)
            .available_permits()
            .min(self.global.available_permits())
    }

    pub fn with_upstream(mut self, handler: Arc<dyn UpstreamHandler>, default: bool) -> Self {
        let id = handler.target().id.clone();
        if default {
            self.default_upstream = Some(id.clone());
        }
        self.upstreams.get_mut().insert(id, handler);
        self
    }

    pub fn with_control(mut self, control: Arc<ControlPlane>) -> Self {
        self.control = Some(control);
        self
    }

    pub fn control(&self) -> Option<&Arc<ControlPlane>> {
        self.control.as_ref()
    }

    pub fn with_epoch(mut self, epoch: impl Into<String>) -> Self {
        self.epoch = epoch.into();
        self
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    pub fn devices(&self) -> &DeviceRegistry {
        &self.devices
    }

    /// Install or replace an upstream's handler while serving.
    pub fn replace_upstream(&self, handler: Arc<dyn UpstreamHandler>) {
        let id = handler.target().id.clone();
        self.upstreams.write().insert(id, handler);
    }

    /// Stop serving an upstream; requests for it become ungranted.
    pub fn remove_upstream(&self, id: &UpstreamId) {
        self.upstreams.write().remove(id);
    }

    /// The current identity of every served upstream.
    pub fn targets(&self) -> Vec<TargetIdentity> {
        let mut v: Vec<_> = self
            .upstreams
            .read()
            .values()
            .map(|h| h.target().clone())
            .collect();
        v.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        v
    }

    pub fn target(&self, id: &UpstreamId) -> Option<TargetIdentity> {
        self.upstreams.read().get(id).map(|h| h.target().clone())
    }

    pub fn with_sink(mut self, sink: OutcomeSink) -> Self {
        self.sink = Some(sink);
        self
    }

    fn device_semaphore(&self, device_id: &str) -> Arc<Semaphore> {
        self.per_device
            .lock()
            .entry(device_id.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(self.limits.per_device)))
            .clone()
    }

    /// The one place a parsed target meets a grant. Dispatch uses the
    /// handler this returns, so nothing checks one and calls another.
    fn resolve(
        &self,
        device: &DeviceRecord,
        target: &Target,
    ) -> Result<Arc<dyn UpstreamHandler>, Refusal> {
        let id = match &target.upstream {
            UpstreamSel::Default => self
                .default_upstream
                .as_ref()
                .ok_or(Refusal::UpstreamNotGranted)?,
            UpstreamSel::Named(id) => id,
        };
        let handler = self
            .upstreams
            .read()
            .get(id)
            .cloned()
            .ok_or(Refusal::UpstreamNotGranted)?;
        let Some(grant) = device.grant_for(id) else {
            // A snapshot grant: a later publication is refused outright
            // unless the device opted into being offered it.
            return Err(if device.include_future {
                Refusal::AcceptanceRequired
            } else {
                Refusal::UpstreamNotGranted
            });
        };
        if !grant.covers(handler.target().generation, &self.epoch) {
            return Err(Refusal::AcceptanceRequired);
        }
        if !handler.capabilities().accepts(target.operation) {
            return Err(Refusal::OperationUnsupported);
        }
        Ok(handler)
    }
}

pub fn router(state: Arc<RelayState>) -> Router {
    Router::new().fallback(handle).with_state(state)
}

/// The top-level `model` of a request body, when it is a plain
/// identifier. Anything else is dropped rather than recorded: the field is
/// client-supplied and the ledger is not a place for arbitrary text.
fn requested_model(body: &[u8]) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Probe {
        model: Option<String>,
    }
    super::sse::model_id(&serde_json::from_slice::<Probe>(body).ok()?.model?)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

async fn handle(State(s): State<Arc<RelayState>>, req: Request) -> Response {
    match pipeline(s, req).await {
        Ok(r) => r,
        Err(refusal) => {
            tracing::debug!(code = refusal.code(), "relay refused a request");
            refusal.into_response()
        }
    }
}

/// Resolves once the listener serving this request is shutting down;
/// never, for a request that did not come through [`serve`].
async fn stop_requested(stopping: Option<&Stopping>) {
    match stopping {
        Some(s) => s.0.cancelled().await,
        None => std::future::pending().await,
    }
}

fn is_stopping(stopping: Option<&Stopping>) -> bool {
    stopping.is_some_and(|s| s.0.is_cancelled())
}

/// The request pipeline, in order. Each step either refuses or hands the
/// next one what it needs; nothing past `authenticate` runs for a caller
/// without a live token.
async fn pipeline(s: Arc<RelayState>, req: Request) -> Result<Response, Refusal> {
    // Once Stop is asked for, nothing new is dispatched. 503 because it
    // is transient: CC retries it, and a restarted relay answers.
    let stopping = req.extensions().get::<Stopping>().cloned();
    if is_stopping(stopping.as_ref()) {
        return Err(Refusal::OverCapacity);
    }
    let parsed = target::parse(req.method(), req.uri());
    // 1. Endpoints answered before auth: hello, ca, pair, health.
    let req = match public(&s, &parsed, req).await {
        Public::Answered(r) => return r,
        Public::Not(req) => req,
    };
    // 2. Auth.
    let (device, cancel) = bearer(req.headers())
        .and_then(|t| s.devices.authenticate(t))
        .ok_or(Refusal::Unauthenticated)?;
    // 3. The authenticated control endpoint, or an API target.
    let target = match parsed {
        Ok(Parsed::Api(t)) => t,
        Ok(Parsed::Control(name))
            if name == "accept" && req.method() == axum::http::Method::POST =>
        {
            return accept(&s, &device, req).await;
        }
        Ok(Parsed::Control(_)) | Ok(Parsed::Hello) | Err(_) => return Err(Refusal::NotFound),
    };
    // 4. Admission, 5. grant, 6. body, 7. dispatch and accounting.
    let held = admit(&s, &device.id)?;
    let handler = s.resolve(&device, &target)?;
    let (admitted, requested_model) =
        read_admitted(&s, &device, &target, req, &cancel, stopping.as_ref()).await?;
    dispatch(&s, handler, admitted, requested_model, cancel, held).await
}

/// What the pre-auth step did with a request.
enum Public {
    Answered(Result<Response, Refusal>),
    /// Not a public endpoint: on to auth.
    Not(Request),
}

async fn public(s: &RelayState, parsed: &Result<Parsed, target::NotFound>, req: Request) -> Public {
    if let Ok(Parsed::Hello) = parsed {
        return Public::Answered(Ok(StatusCode::OK.into_response()));
    }
    let Ok(Parsed::Control(name)) = parsed else {
        return Public::Not(req);
    };
    let get = req.method() == axum::http::Method::GET;
    let post = req.method() == axum::http::Method::POST;
    // The two pre-auth control endpoints. Both answer only on a TLS
    // listener; `ca` is public material and `pair` is gated by the code.
    match (name.as_str(), &s.control) {
        ("ca", Some(c)) if get => Public::Answered(Ok(json_response(serde_json::json!({
            "ca_pem": c.ca_pem,
            "fingerprint": c.ca_fingerprint,
            "server_id": c.server_id,
        })))),
        ("pair", Some(c)) if post => {
            let c = c.clone();
            Public::Answered(pair(s, &c, req).await)
        }
        // Health answers everyone, but only a paired device learns
        // anything beyond liveness — and a bad token is not told it is bad.
        ("health", _) if get => Public::Answered(health(s, req.headers()).await),
        _ => Public::Not(req),
    }
}

/// A slot for the device and one for the server, held by the response
/// body until it ends.
fn admit(s: &RelayState, device_id: &str) -> Result<Held, Refusal> {
    let device_permit = s
        .device_semaphore(device_id)
        .try_acquire_owned()
        .map_err(|_| Refusal::OverCapacity)?;
    let global_permit = s
        .global
        .clone()
        .try_acquire_owned()
        .map_err(|_| Refusal::OverCapacity)?;
    Ok(Held {
        _device: device_permit,
        _global: global_permit,
    })
}

/// Buffer the body of an admitted, granted request.
async fn read_admitted(
    s: &RelayState,
    device: &DeviceRecord,
    target: &Target,
    req: Request,
    cancel: &CancelToken,
    stopping: Option<&Stopping>,
) -> Result<(AdmittedRequest, Option<String>), Refusal> {
    let (parts, body) = req.into_parts();
    // A revoked device, or a stopping listener, gives its slots and body
    // budget back now — not when a drip-fed upload finally ends.
    let body = tokio::select! {
        r = read_body(s, &parts.headers, body) => r?,
        () = cancel.cancelled() => return Err(Refusal::Unauthenticated),
        () = stop_requested(stopping) => return Err(Refusal::OverCapacity),
    };
    if is_stopping(stopping) {
        return Err(Refusal::OverCapacity);
    }
    let requested_model = requested_model(&body.bytes);
    let admitted = AdmittedRequest {
        operation: target.operation,
        device_id: device.id.clone(),
        beta: target.beta,
        headers: parts.headers,
        body,
    };
    Ok((admitted, requested_model))
}

/// Hand the request to its upstream and record the outcome exactly once:
/// here if it fails before headers, otherwise when the stream ends.
async fn dispatch(
    s: &RelayState,
    handler: Arc<dyn UpstreamHandler>,
    admitted: AdmittedRequest,
    requested_model: Option<String>,
    cancel: CancelToken,
    held: Held,
) -> Result<Response, Refusal> {
    let emit = outcome_emitter(
        s,
        handler.target().clone(),
        admitted.device_id.clone(),
        admitted.operation,
        requested_model,
    );

    let (upstream, outcome) = tokio::select! {
        r = handler.handle(admitted, cancel.clone()) => match r {
            Ok(r) => r,
            // No response headers ever arrived, so no outcome future
            // exists to resolve: record the failure here, by its fixed
            // code, or the ledger and health never see it.
            Err(e) => {
                let refusal = Refusal::from(e);
                emit(before_headers(OutcomeKind::Failed, refusal));
                return Err(refusal);
            }
        },
        // Revoked while waiting for headers: dropping the handler future
        // drops the upstream request. Cancelled, so partial, as a revoke
        // mid-stream is.
        _ = cancel.cancelled() => {
            emit(before_headers(OutcomeKind::Partial, Refusal::Unauthenticated));
            return Err(Refusal::Unauthenticated);
        }
    };

    tokio::spawn(async move {
        if let Ok(outcome) = outcome.await {
            emit(outcome);
        }
    });

    Ok(revocable_response(upstream, cancel, held))
}

/// `GET /relay/health`: liveness for anyone; the report only for a
/// paired device — and a bad token is not told it is bad.
async fn health(s: &RelayState, headers: &HeaderMap) -> Result<Response, Refusal> {
    let Some((device, _)) = bearer(headers).and_then(|t| s.devices.authenticate(t)) else {
        return Ok(json_response(serde_json::json!({ "ok": true })));
    };
    let salt = headers
        .get(super::health::SALT_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(super::identity::parse_salt)
        .ok_or(Refusal::BadRequest)?;
    let report = super::health::report(s, &device, &salt).await;
    Ok(json_response(
        serde_json::to_value(report).map_err(|_| Refusal::StoreUnavailable)?,
    ))
}

/// Records one request's outcome to the stats and the sink, exactly once.
fn outcome_emitter(
    s: &RelayState,
    record_target: TargetIdentity,
    device_id: String,
    operation: Operation,
    requested_model: Option<String>,
) -> impl FnOnce(Outcome) + Send + 'static {
    let sink = s.sink.clone();
    let stats = s.stats.clone();
    move |outcome: Outcome| {
        let rec = OutcomeRecord {
            device_id,
            target: record_target,
            operation,
            requested_model,
            outcome,
        };
        stats.record(&rec, std::time::Instant::now());
        if let Some(sink) = sink {
            sink(rec);
        }
    }
}

/// The client's response: the upstream's status, headers and body, with
/// the admission permits held until the body ends and a revoke ending it
/// mid-stream.
fn revocable_response(upstream: UpstreamResponse, cancel: CancelToken, held: Held) -> Response {
    let body = futures_util::stream::unfold(
        (upstream.body, cancel, held),
        |(mut inner, cancel, held)| async move {
            if cancel.is_cancelled() {
                return None;
            }
            tokio::select! {
                next = inner.next() => next.map(|item| (item, (inner, cancel, held))),
                _ = cancel.cancelled() => Some((
                    Err(std::io::Error::other("device revoked")),
                    (inner, cancel, held),
                )),
            }
        },
    );
    let mut resp = Response::new(Body::from_stream(body));
    *resp.status_mut() = upstream.status;
    *resp.headers_mut() = upstream.headers;
    resp
}

fn json_response(v: serde_json::Value) -> Response {
    let mut resp = (StatusCode::OK, v.to_string()).into_response();
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    h.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    resp
}

async fn control_body<T: serde::de::DeserializeOwned>(
    s: &RelayState,
    req: Request,
) -> Result<T, Refusal> {
    let body = req.into_body();
    let bytes = tokio::time::timeout(
        s.limits.body_idle,
        axum::body::to_bytes(body, MAX_CONTROL_BODY),
    )
    .await
    .map_err(|_| Refusal::BodyTimeout)?
    .map_err(|_| Refusal::BodyTooLarge)?;
    serde_json::from_slice(&bytes).map_err(|_| Refusal::BadRequest)
}

#[derive(serde::Deserialize)]
struct PairRequest {
    code: String,
    device_name: String,
    client_salt: String,
}

#[derive(serde::Deserialize)]
struct AcceptRequest {
    upstream: UpstreamId,
    /// `None` asks for the current fingerprint without recording
    /// anything, so the client can show it before the user agrees.
    #[serde(default)]
    expected_fingerprint: Option<String>,
    client_salt: String,
}

fn valid_device_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.chars().count() <= MAX_DEVICE_NAME && !n.chars().any(char::is_control)
}

/// Grants are a snapshot of what is published right now (plan §6).
async fn pair(s: &RelayState, c: &ControlPlane, req: Request) -> Result<Response, Refusal> {
    let body: PairRequest = control_body(s, req).await?;
    let salt = identity::parse_salt(&body.client_salt).ok_or(Refusal::BadRequest)?;
    if !valid_device_name(&body.device_name) {
        return Err(Refusal::BadRequest);
    }
    let redeemed = c
        .pairing
        .redeem(&body.code, std::time::Instant::now())
        .map_err(|e| match e {
            PairError::Unavailable => Refusal::PairingUnavailable,
            PairError::Wrong => Refusal::PairingCodeInvalid,
        })?;
    let targets = s.targets();
    let grants = targets
        .iter()
        .map(|t| Grant {
            upstream_id: t.id.clone(),
            accepted_generation: t.generation,
            epoch: s.epoch.clone(),
        })
        .collect();
    let issued = s
        .devices
        .enroll(body.device_name.trim(), grants, redeemed.include_future)
        .map_err(|e| {
            tracing::error!(error = %e, "relay pairing could not record the device");
            Refusal::StoreUnavailable
        })?;
    let upstreams: Vec<_> = targets
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "fingerprint": identity::client_fingerprint(&salt, &t.fingerprint),
                "generation": t.generation,
            })
        })
        .collect();
    Ok(json_response(serde_json::json!({
        "device_id": issued.device_id,
        "token": issued.plaintext.expose(),
        "server_id": c.server_id,
        "default_upstream": s.default_upstream,
        "upstreams": upstreams,
    })))
}

/// Record a device's acceptance of an upstream's current target. Only a
/// device that already holds a grant for it, or opted into future
/// publications, may accept: otherwise a snapshot grant would be one
/// request away from any new publication.
async fn accept(s: &RelayState, device: &DeviceRecord, req: Request) -> Result<Response, Refusal> {
    let body: AcceptRequest = control_body(s, req).await?;
    let salt = identity::parse_salt(&body.client_salt).ok_or(Refusal::BadRequest)?;
    if device.grant_for(&body.upstream).is_none() && !device.include_future {
        return Err(Refusal::UpstreamNotGranted);
    }
    let current = s
        .target(&body.upstream)
        .ok_or(Refusal::UpstreamNotGranted)?;
    let fingerprint = identity::client_fingerprint(&salt, &current.fingerprint);
    let Some(expected) = body.expected_fingerprint else {
        // A query records nothing, but says whether this device's grant
        // already covers the current target — the same test `admit`
        // applies — so a client asking about an unchanged target is not
        // told it changed.
        let covered = device
            .grant_for(&current.id)
            .is_some_and(|g| g.covers(current.generation, &s.epoch));
        return Ok(json_response(serde_json::json!({
            "upstream": current.id,
            "fingerprint": fingerprint,
            "generation": current.generation,
            "accepted": covered,
        })));
    };
    if !crate::remote::ct::constant_time_eq(expected.as_bytes(), fingerprint.as_bytes()) {
        return Err(Refusal::FingerprintMismatch);
    }
    let recorded = s
        .devices
        .accept(
            &device.id,
            Grant {
                upstream_id: current.id.clone(),
                accepted_generation: current.generation,
                epoch: s.epoch.clone(),
            },
        )
        .map_err(|e| {
            tracing::error!(error = %e, "relay could not record an acceptance");
            Refusal::StoreUnavailable
        })?;
    if !recorded {
        return Err(Refusal::Unauthenticated);
    }
    Ok(json_response(serde_json::json!({
        "upstream": current.id,
        "fingerprint": fingerprint,
        "generation": current.generation,
        "accepted": true,
    })))
}

/// Admission permits, owned by the response body.
struct Held {
    _device: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

async fn read_body(
    s: &RelayState,
    headers: &HeaderMap,
    body: Body,
) -> Result<BufferedBody, Refusal> {
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if declared.is_some_and(|n| n > s.limits.max_body) {
        return Err(Refusal::BodyTooLarge);
    }
    // Reserve what the body may grow to; an undeclared length reserves
    // the cap, which is what makes the aggregate budget a real bound.
    let reserve = declared.unwrap_or(s.limits.max_body).max(1);
    let permits = u32::try_from(reserve).map_err(|_| Refusal::BodyTooLarge)?;
    let budget = s
        .budget
        .clone()
        .try_acquire_many_owned(permits)
        .map_err(|_| Refusal::OverCapacity)?;

    let mut stream = body.into_data_stream();
    let mut buf = Vec::with_capacity(declared.unwrap_or(0));
    loop {
        let next = tokio::time::timeout(s.limits.body_idle, stream.next())
            .await
            .map_err(|_| Refusal::BodyTimeout)?;
        match next {
            None => break,
            Some(Err(_)) => return Err(Refusal::BodyUnreadable),
            Some(Ok(chunk)) => {
                if buf.len() + chunk.len() > reserve.min(s.limits.max_body) {
                    return Err(Refusal::BodyTooLarge);
                }
                buf.extend_from_slice(&chunk);
            }
        }
    }
    Ok(BufferedBody {
        bytes: Bytes::from(buf),
        _budget: Some(budget),
    })
}

/// Serve `router` on `listener` until `shutdown` fires, then drain. With
/// an acceptor every connection must finish its TLS handshake within
/// [`Limits::tls_handshake`].
///
/// **Returns only once every connection has closed.** Stopping the
/// accept loop alone is not stopping: a keep-alive connection would go on
/// carrying billable requests after the caller reported "stopped" and
/// released the serve lock. On shutdown each connection is told to close
/// gracefully — an idle one closes at once, one mid-response gets
/// [`Limits::shutdown_drain`] to finish and is then cut, which drops its
/// upstream request. Requests that arrive meanwhile are refused before
/// dispatch (`Stopping`).
pub async fn serve(
    listener: TcpListener,
    router: Router,
    tls: Option<TlsAcceptor>,
    limits: Limits,
    shutdown: CancelToken,
) {
    let app = router.layer(axum::Extension(Stopping(shutdown.clone())));
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    let mut conns = tokio::task::JoinSet::new();
    loop {
        // A slot first, then a socket: at the cap nothing is accepted, so
        // an idle flood waits in the backlog instead of holding tasks.
        let slot = tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            // Reap finished connections so the set does not grow.
            Some(_) = conns.join_next(), if !conns.is_empty() => continue,
            s = slots.clone().acquire_owned() => match s {
                Ok(s) => s,
                Err(_) => break,
            },
        };
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
            r = listener.accept() => {
                if let Ok((stream, _peer)) = r {
                    let conn = connection(
                        stream,
                        app.clone(),
                        tls.clone(),
                        limits,
                        shutdown.clone(),
                    );
                    conns.spawn(async move {
                        let _slot = slot;
                        conn.await;
                    });
                }
            }
        }
    }
    drop(listener);
    // Every connection bounds its own drain; this is the backstop should
    // one not, so that `serve` returning always means "nothing serves".
    let backstop = limits.shutdown_drain + Duration::from_secs(1);
    let drained = tokio::time::timeout(backstop, async {
        while conns.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        conns.shutdown().await;
    }
}

async fn connection(
    stream: tokio::net::TcpStream,
    app: Router,
    tls: Option<TlsAcceptor>,
    limits: Limits,
    shutdown: CancelToken,
) {
    match tls {
        Some(acceptor) => {
            let handshake = tokio::time::timeout(limits.tls_handshake, acceptor.accept(stream));
            let tls_stream = tokio::select! {
                r = handshake => match r {
                    Ok(Ok(s)) => s,
                    _ => return,
                },
                () = shutdown.cancelled() => return,
            };
            drive(tls_stream, app, limits, &shutdown).await;
        }
        None => drive(stream, app, limits, &shutdown).await,
    }
}

/// Serve one connection until it ends, or until shutdown plus the drain.
async fn drive<T>(io: T, app: Router, limits: Limits, shutdown: &CancelToken)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // HTTP/1 only, which is all the relay builds: the auto builder's
    // version sniffing otherwise waits for preface bytes with no deadline,
    // so a silent socket would never reach the header timeout below.
    let mut builder = ConnBuilder::new(TokioExecutor::new()).http1_only();
    // Without a timer hyper enforces no header deadline at all.
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read);
    let conn = builder.serve_connection(TokioIo::new(io), TowerToHyperService::new(app));
    tokio::pin!(conn);
    tokio::select! {
        _ = conn.as_mut() => return,
        () = shutdown.cancelled() => {}
    }
    conn.as_mut().graceful_shutdown();
    // Dropping `conn` past the bound drops the response body, which
    // releases its admission permits and cancels the upstream request.
    let _ = tokio::time::timeout(limits.shutdown_drain, conn).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const HELLO: &[u8] = b"HEAD /api/hello HTTP/1.1\r\nHost: relay\r\n\r\n";

    async fn plain_relay(
        limits: Limits,
    ) -> (
        std::net::SocketAddr,
        CancelToken,
        tokio::task::JoinHandle<()>,
    ) {
        let state = Arc::new(RelayState::new(DeviceRegistry::new(), limits));
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let shutdown = CancelToken::new();
        let task = tokio::spawn(serve(l, router(state), None, limits, shutdown.clone()));
        (addr, shutdown, task)
    }

    /// Bytes read before EOF, or `None` if the peer neither answered nor
    /// closed within `wait`.
    async fn read_some(s: &mut tokio::net::TcpStream, wait: Duration) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 256];
        match tokio::time::timeout(wait, s.read(&mut buf)).await {
            Ok(Ok(n)) => Some(buf[..n].to_vec()),
            Ok(Err(_)) => Some(Vec::new()),
            Err(_) => None,
        }
    }

    #[tokio::test]
    async fn a_connection_that_never_sends_a_full_head_is_closed_at_the_deadline() {
        let limits = Limits {
            header_read: Duration::from_millis(200),
            ..Limits::default()
        };
        let (addr, shutdown, task) = plain_relay(limits).await;
        let mut silent = tokio::net::TcpStream::connect(addr).await.unwrap();
        let mut partial = tokio::net::TcpStream::connect(addr).await.unwrap();
        partial
            .write_all(b"HEAD /api/hello HTTP/1.1\r\n")
            .await
            .unwrap();
        for s in [&mut silent, &mut partial] {
            let got = read_some(s, Duration::from_secs(3)).await;
            assert!(
                got.as_ref()
                    .is_some_and(|b| !b.starts_with(b"HTTP/1.1 200")),
                "an incomplete head must be cut, got {got:?}"
            );
        }
        shutdown.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn the_listener_accepts_no_connection_past_the_cap() {
        let limits = Limits {
            max_connections: 2,
            ..Limits::default()
        };
        let (addr, shutdown, task) = plain_relay(limits).await;
        let idle_a = tokio::net::TcpStream::connect(addr).await.unwrap();
        let _idle_b = tokio::net::TcpStream::connect(addr).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut third = tokio::net::TcpStream::connect(addr).await.unwrap();
        third.write_all(HELLO).await.unwrap();
        assert_eq!(
            read_some(&mut third, Duration::from_millis(300)).await,
            None,
            "a third connection was served past a cap of two"
        );
        // A slot frees, and the waiting connection is served.
        drop(idle_a);
        let got = read_some(&mut third, Duration::from_secs(3)).await.unwrap();
        assert!(got.starts_with(b"HTTP/1.1 200"), "{got:?}");
        shutdown.cancel();
        task.await.unwrap();
    }

    #[test]
    fn requested_model_keeps_only_a_plain_identifier() {
        assert_eq!(
            requested_model(br#"{"model":"claude-sonnet-4-5","max_tokens":1}"#).as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(requested_model(br#"{"max_tokens":1}"#), None);
        assert_eq!(requested_model(br#"{"model":"a b\nc"}"#), None);
        assert_eq!(requested_model(br#"{"model":7}"#), None);
        assert_eq!(requested_model(b"not json"), None);
        assert_eq!(
            requested_model(format!(r#"{{"model":"{}"}}"#, "m".repeat(129)).as_bytes()),
            None
        );
    }
}

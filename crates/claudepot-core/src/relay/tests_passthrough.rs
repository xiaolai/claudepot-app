//! Pass-through suite (plan P9): a real relay listener in front of a
//! loopback "DeepSeek", so what is asserted is what crosses a socket.

use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use parking_lot::Mutex;
use tokio::net::TcpListener;
use uuid::Uuid;

use super::*;
use crate::relay::cancel::CancelToken;
use crate::relay::devices::{DeviceRegistry, Grant};
use crate::relay::forward::base_builder;
use crate::relay::ledger::LedgerLine;
use crate::relay::server::{self, Limits, OutcomeRecord, RelayState};
use crate::relay::target::UpstreamId;
use crate::routes::{AuthScheme, GatewayConfig};

const ROUTE_KEY: &str = "sk-route-CANARYdeepseekKEY";

// ---------------------------------------------------------------------
// The mock gateway
// ---------------------------------------------------------------------

struct Seen {
    path: String,
    headers: HeaderMap,
    body: serde_json::Value,
}

struct Gateway {
    status: AtomicU16,
    hits: AtomicUsize,
    seen: Mutex<Vec<Seen>>,
}

const SSE: &str = concat!(
    "event: message_start\n",
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"deepseek-chat\",\"usage\":{\"input_tokens\":12,\"output_tokens\":1}}}\n\n",
    "event: message_delta\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
    "event: message_stop\n",
    "data: {\"type\":\"message_stop\"}\n\n",
);

async fn gateway_handler(
    State(g): State<Arc<Gateway>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    g.hits.fetch_add(1, Ordering::SeqCst);
    g.seen.lock().push(Seen {
        path: uri.path().to_owned(),
        headers,
        body: serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    });
    let status = g.status.load(Ordering::SeqCst);
    if status != 200 {
        return (StatusCode::from_u16(status).unwrap(), "{}").into_response();
    }
    ([("content-type", "text/event-stream")], SSE).into_response()
}

async fn spawn_gateway(status: u16) -> (String, Arc<Gateway>) {
    let g = Arc::new(Gateway {
        status: AtomicU16::new(status),
        hits: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .fallback(gateway_handler)
        .with_state(g.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), g)
}

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

fn route(base_url: &str, use_keychain: bool) -> Route {
    Route {
        id: Uuid::from_u128(0xd5),
        name: "DeepSeek".into(),
        provider: RouteProvider::Gateway(GatewayConfig {
            base_url: format!("{base_url}/anthropic/"),
            api_key: if use_keychain {
                String::new()
            } else {
                ROUTE_KEY.into()
            },
            auth_scheme: AuthScheme::Bearer,
            enable_tool_search: false,
            use_keychain,
        }),
        model: "deepseek-chat".into(),
        small_fast_model: Some("deepseek-lite".into()),
        additional_models: vec!["deepseek-reasoner".into()],
        wrapper_name: "claude-deepseek".into(),
        deployment_organization_uuid: Uuid::nil(),
        active_on_desktop: false,
        installed_on_cli: true,
        is_private_cloud: false,
        capabilities_override: None,
    }
}

/// A keychain that holds `secret` for every route, or nothing.
struct FakeKeychain(Option<&'static str>);
impl KeychainReader for FakeKeychain {
    fn read(&self, _route: crate::routes::RouteId) -> Result<Option<Secret>, RouteSecretError> {
        Ok(self.0.map(Secret::new))
    }
}

fn route_id() -> UpstreamId {
    UpstreamId::parse("deepseek").unwrap()
}

fn target(r: &Route) -> TargetIdentity {
    TargetIdentity {
        id: route_id(),
        fingerprint: route_target_spec(r).unwrap().digest(),
        generation: 1,
        kind: TargetKind::PassThrough,
    }
}

struct Relay {
    addr: std::net::SocketAddr,
    token: String,
    outcomes: Arc<Mutex<Vec<OutcomeRecord>>>,
    shutdown: CancelToken,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// A keychain whose answer the test can change, counting reads.
struct SwitchKeychain {
    secret: Mutex<Result<Option<&'static str>, RouteSecretError>>,
    reads: AtomicUsize,
}

impl SwitchKeychain {
    fn locked() -> Arc<Self> {
        Arc::new(Self {
            secret: Mutex::new(Err(RouteSecretError::Locked)),
            reads: AtomicUsize::new(0),
        })
    }

    fn holding(secret: &'static str) -> Arc<Self> {
        Arc::new(Self {
            secret: Mutex::new(Ok(Some(secret))),
            reads: AtomicUsize::new(0),
        })
    }

    fn set(&self, secret: Result<Option<&'static str>, RouteSecretError>) {
        *self.secret.lock() = secret;
    }
}

impl KeychainReader for SwitchKeychain {
    fn read(&self, _route: crate::routes::RouteId) -> Result<Option<Secret>, RouteSecretError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        match &*self.secret.lock() {
            Ok(s) => Ok(s.map(Secret::new)),
            Err(e) => Err(match e {
                RouteSecretError::Locked => RouteSecretError::Locked,
                other => RouteSecretError::Backend(other.to_string()),
            }),
        }
    }
}

fn handler_with(r: Route, key: RouteKey) -> PassThroughHandler {
    let timeouts = Timeouts::default();
    let client = base_builder(&timeouts).no_proxy().build().unwrap();
    PassThroughHandler::with_key(target(&r), r, client, timeouts, key)
}

/// Poll health until `want` holds, giving a background probe time to run.
async fn status_until(
    h: &PassThroughHandler,
    want: impl Fn(&UpstreamStatus) -> bool,
) -> UpstreamStatus {
    for _ in 0..200 {
        let s = h.status().await;
        if want(&s) {
            return s;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    h.status().await
}

async fn spawn_relay(r: Route, keychain: FakeKeychain) -> Relay {
    let timeouts = Timeouts::default();
    let client = base_builder(&timeouts).no_proxy().build().unwrap();
    let handler =
        PassThroughHandler::with_client(target(&r), r, client, timeouts, Arc::new(keychain));
    spawn_relay_for(Arc::new(handler)).await
}

async fn spawn_relay_for(handler: Arc<PassThroughHandler>) -> Relay {
    let devices = DeviceRegistry::new();
    let issued = devices
        .issue(
            "laptop",
            vec![Grant {
                upstream_id: route_id(),
                accepted_generation: 1,
                epoch: String::new(),
            }],
        )
        .unwrap();
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let sink = outcomes.clone();
    let limits = Limits::default();
    let state = RelayState::new(devices, limits)
        .with_upstream(handler, true)
        .with_sink(Arc::new(move |o| sink.lock().push(o)));
    let router = server::router(Arc::new(state));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let shutdown = CancelToken::new();
    tokio::spawn(server::serve(l, router, None, limits, shutdown.clone()));
    Relay {
        addr,
        token: issued.plaintext.expose().to_owned(),
        outcomes,
        shutdown,
    }
}

impl Relay {
    async fn send(&self, path: &str, body: &str) -> reqwest::Response {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}{path}", self.addr))
            .bearer_auth(&self.token)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "interleaved-thinking-2025-05-14")
            .header("content-type", "application/json")
            .body(body.to_owned())
            .send()
            .await
            .unwrap()
    }

    /// The outcome sink runs after the stream ends; give it a moment.
    async fn outcome(&self) -> OutcomeRecord {
        for _ in 0..200 {
            if let Some(o) = self.outcomes.lock().first().cloned() {
                return o;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("no outcome recorded");
    }
}

async fn error_code(resp: reqwest::Response) -> (u16, String) {
    let status = resp.status().as_u16();
    let v: serde_json::Value = resp.json().await.unwrap();
    (status, v["error"]["message"].as_str().unwrap().to_owned())
}

// ---------------------------------------------------------------------
// Pure mapping
// ---------------------------------------------------------------------

#[test]
fn a_claude_model_maps_to_the_route_and_a_route_model_is_kept() {
    let r = route("http://x", false);
    assert_eq!(map_model(&r, Some("claude-sonnet-4-5")), "deepseek-chat");
    assert_eq!(
        map_model(&r, Some("claude-3-5-HAIKU-latest")),
        "deepseek-lite"
    );
    assert_eq!(
        map_model(&r, Some("deepseek-reasoner")),
        "deepseek-reasoner"
    );
    assert_eq!(map_model(&r, None), "deepseek-chat");
    let mut no_small = r.clone();
    no_small.small_fast_model = None;
    assert_eq!(
        map_model(&no_small, Some("claude-haiku-4-5")),
        "deepseek-chat"
    );
}

#[test]
fn the_target_moves_with_origin_model_and_key_location_but_not_the_key() {
    let base = route("http://x", false);
    let digest = |r: &Route| route_target_spec(r).unwrap().digest();
    let d0 = digest(&base);

    let mut rotated = base.clone();
    if let RouteProvider::Gateway(c) = &mut rotated.provider {
        c.api_key = "sk-route-rotated".into();
    }
    assert_eq!(
        digest(&rotated),
        d0,
        "rotating the key in place keeps the target"
    );

    let mut trailing = base.clone();
    if let RouteProvider::Gateway(c) = &mut trailing.provider {
        c.base_url = "http://x/anthropic".into();
    }
    assert_eq!(
        digest(&trailing),
        d0,
        "a trailing slash is not a new origin"
    );

    let mut origin = base.clone();
    if let RouteProvider::Gateway(c) = &mut origin.provider {
        c.base_url = "http://y/anthropic/".into();
    }
    let mut model = base.clone();
    model.model = "deepseek-v4".into();
    let moved = route("http://x", true);
    for (what, r) in [
        ("origin", origin),
        ("model", model),
        ("key location", moved),
    ] {
        assert_ne!(digest(&r), d0, "{what} must change the target");
    }

    let spec = route_target_spec(&base).unwrap();
    let rendered = format!("{spec:?}");
    assert!(
        !rendered.contains(ROUTE_KEY),
        "the spec never holds the key"
    );
}

#[test]
fn a_route_model_named_like_a_fallback_slot_does_not_hide_a_slot_change() {
    // `haiku` is a literal route model, so it must not share a key with
    // the haiku-tier slot: moving that slot changes where
    // `claude-haiku-4-5` goes, and so must change the target.
    let mut a = route("http://x", false);
    a.additional_models = vec!["haiku".into(), "fast-a".into(), "fast-b".into()];
    a.small_fast_model = Some("fast-a".into());
    let mut b = a.clone();
    b.small_fast_model = Some("fast-b".into());
    assert_ne!(
        map_model(&a, Some("claude-haiku-4-5")),
        map_model(&b, Some("claude-haiku-4-5"))
    );
    assert_ne!(
        route_target_spec(&a).unwrap().digest(),
        route_target_spec(&b).unwrap().digest(),
        "a changed haiku slot is a new target"
    );
}

#[test]
fn a_route_that_is_not_a_gateway_is_not_relay_capable() {
    let mut r = route("http://x", false);
    r.provider = RouteProvider::Vertex(crate::routes::VertexConfig {
        project_id: "p".into(),
        region: None,
        base_url: None,
        skip_gcp_auth: false,
    });
    assert!(matches!(
        route_target_spec(&r),
        Err(RouteSecretError::NotRelayCapable(_))
    ));
}

// ---------------------------------------------------------------------
// Through the socket
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_mapped_request_reaches_the_gateway_with_its_key_and_is_recorded_unpriced() {
    let (origin, g) = spawn_gateway(200).await;
    let relay = spawn_relay(route(&origin, false), FakeKeychain(None)).await;
    let resp = relay
        .send(
            "/u/deepseek/v1/messages?beta=true",
            r#"{"model":"claude-sonnet-4-5","max_tokens":8,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
        )
        .await;
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(text.contains("deepseek-chat"), "the stream is forwarded");

    let seen = std::mem::take(&mut *g.seen.lock());
    assert_eq!(seen.len(), 1);
    let s = &seen[0];
    assert_eq!(s.path, "/anthropic/v1/messages");
    assert_eq!(s.body["model"], "deepseek-chat");
    assert_eq!(s.body["max_tokens"], 8, "nothing else in the body moves");
    assert_eq!(s.body["messages"][0]["content"], "hi");
    let auth = s.headers["authorization"].to_str().unwrap();
    assert_eq!(auth, format!("Bearer {ROUTE_KEY}"));
    assert!(
        !auth.contains(&relay.token),
        "the device token never leaves"
    );
    for (name, value) in s.headers.iter() {
        let v = value.to_str().unwrap_or("");
        assert!(!v.contains(&relay.token), "device token leaked in {name}");
        assert!(
            !v.contains("oauth-2025-04-20"),
            "no subscription beta on {name}"
        );
    }
    assert_eq!(
        s.headers["anthropic-beta"], "interleaved-thinking-2025-05-14",
        "the client's own betas pass through"
    );

    let rec = relay.outcome().await;
    assert_eq!(rec.target.kind, TargetKind::PassThrough);
    let line = LedgerLine::from_record(
        &rec,
        chrono::Utc::now(),
        &crate::pricing::PriceBook::bundled_only(),
    );
    assert_eq!(line.requested_model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(line.model.as_deref(), Some("deepseek-chat"));
    assert!(line.cost.is_none(), "a route upstream is never priced");
    assert!(line.usage.is_some(), "usage is still counted");
}

#[tokio::test]
async fn a_keychain_route_reads_its_key_from_the_keychain() {
    let (origin, g) = spawn_gateway(200).await;
    let relay = spawn_relay(route(&origin, true), FakeKeychain(Some("sk-from-keychain"))).await;
    let resp = relay
        .send(
            "/u/deepseek/v1/messages",
            r#"{"model":"claude-haiku-4-5","max_tokens":1}"#,
        )
        .await;
    assert_eq!(resp.status(), 200);
    let seen = g.seen.lock();
    assert_eq!(seen[0].headers["authorization"], "Bearer sk-from-keychain");
    assert_eq!(seen[0].body["model"], "deepseek-lite");
}

#[tokio::test]
async fn a_missing_keychain_secret_is_credential_unavailable_and_never_dispatched() {
    let (origin, g) = spawn_gateway(200).await;
    let relay = spawn_relay(route(&origin, true), FakeKeychain(None)).await;
    let resp = relay
        .send(
            "/u/deepseek/v1/messages",
            r#"{"model":"claude-sonnet-4-5","max_tokens":1}"#,
        )
        .await;
    assert_eq!(
        error_code(resp).await,
        (403, "credential_unavailable".into())
    );
    assert_eq!(g.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_body_that_is_not_a_json_object_is_refused_before_dispatch() {
    let (origin, g) = spawn_gateway(200).await;
    let relay = spawn_relay(route(&origin, false), FakeKeychain(None)).await;
    for body in ["not json", "[1,2]"] {
        let resp = relay.send("/u/deepseek/v1/messages", body).await;
        assert_eq!(
            error_code(resp).await,
            (400, "content_unsupported".into()),
            "{body}"
        );
    }
    assert_eq!(g.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_rejected_route_key_is_a_403_with_one_upstream_attempt() {
    let (origin, g) = spawn_gateway(401).await;
    let relay = spawn_relay(route(&origin, false), FakeKeychain(None)).await;
    let resp = relay
        .send(
            "/u/deepseek/v1/messages",
            r#"{"model":"claude-sonnet-4-5","max_tokens":1}"#,
        )
        .await;
    assert_eq!(
        error_code(resp).await,
        (403, "upstream_credential_rejected".into())
    );
    assert_eq!(
        g.hits.load(Ordering::SeqCst),
        1,
        "a route key has no renewal"
    );
}

#[tokio::test]
async fn health_reports_a_missing_route_key_after_the_start_check() {
    let r = route("http://127.0.0.1:9", true);
    let timeouts = Timeouts::default();
    let client = base_builder(&timeouts).no_proxy().build().unwrap();
    let h = PassThroughHandler::with_client(
        target(&r),
        r,
        client,
        timeouts,
        Arc::new(FakeKeychain(None)),
    );
    assert!(h.status().await.credential_ready, "unknown until checked");
    assert_eq!(
        h.check_credential().await,
        Err(UpstreamError::CredentialUnavailable)
    );
    let s = h.status().await;
    assert!(!s.credential_ready);
    assert_eq!(
        s.credential_error,
        Some(UpstreamError::CredentialUnavailable)
    );
}

// ---------------------------------------------------------------------
// A serving relay follows the route's key, and its health recovers
// ---------------------------------------------------------------------

const BODY_MIN: &str = r#"{"model":"claude-sonnet-4-5","max_tokens":1}"#;

fn set_key(r: &mut Route, key: &str) {
    if let RouteProvider::Gateway(c) = &mut r.provider {
        c.api_key = key.into();
    }
}

#[tokio::test]
async fn a_plaintext_key_rotated_in_routes_json_reaches_the_next_request_without_a_restart() {
    let (origin, g) = spawn_gateway(200).await;
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("routes.json");
    let mut store = crate::routes::RouteStore::open_at(file.clone()).unwrap();
    let approved = store.add(route(&origin, false)).unwrap();
    let key = RouteKey::new(approved.clone(), Arc::new(FakeKeychain(None))).with_routes_file(file);
    let relay = spawn_relay_for(Arc::new(handler_with(approved.clone(), key))).await;

    assert_eq!(
        relay
            .send("/u/deepseek/v1/messages", BODY_MIN)
            .await
            .status(),
        200
    );
    let mut rotated = approved.clone();
    set_key(&mut rotated, "sk-route-ROTATED");
    store.update(rotated).unwrap();
    assert_eq!(
        relay
            .send("/u/deepseek/v1/messages", BODY_MIN)
            .await
            .status(),
        200
    );

    let seen = g.seen.lock();
    assert_eq!(
        seen[0].headers["authorization"],
        format!("Bearer {ROUTE_KEY}")
    );
    assert_eq!(seen[1].headers["authorization"], "Bearer sk-route-ROTATED");
}

#[tokio::test]
async fn a_route_retargeted_while_serving_is_refused_and_its_new_key_is_never_sent() {
    let (origin, g) = spawn_gateway(200).await;
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("routes.json");
    let mut store = crate::routes::RouteStore::open_at(file.clone()).unwrap();
    let approved = store.add(route(&origin, false)).unwrap();
    let key = RouteKey::new(approved.clone(), Arc::new(FakeKeychain(None))).with_routes_file(file);
    let relay = spawn_relay_for(Arc::new(handler_with(approved.clone(), key))).await;

    let mut moved = approved.clone();
    if let RouteProvider::Gateway(c) = &mut moved.provider {
        c.base_url = "https://elsewhere.invalid/anthropic".into();
        c.api_key = "sk-route-FOR-ELSEWHERE".into();
    }
    store.update(moved).unwrap();
    let resp = relay.send("/u/deepseek/v1/messages", BODY_MIN).await;
    assert_eq!(
        error_code(resp).await,
        (403, "credential_unavailable".into())
    );
    assert_eq!(g.hits.load(Ordering::SeqCst), 0, "nothing was dispatched");
}

#[tokio::test]
async fn health_recovers_from_a_failed_read_without_a_request() {
    let kc = SwitchKeychain::locked();
    let r = route("http://127.0.0.1:9", true);
    let mut h = handler_with(r.clone(), RouteKey::new(r, kc.clone()));
    h.health = Arc::new(CredentialHealth::new(Duration::ZERO, REJECTION_HOLD));
    assert_eq!(
        h.check_credential().await,
        Err(UpstreamError::CredentialUnavailable)
    );
    assert!(!h.status().await.credential_ready);

    kc.set(Ok(Some("sk-unlocked")));
    let s = status_until(&h, |s| s.credential_ready).await;
    assert!(s.credential_ready, "a re-read cleared the failure: {s:?}");
    assert_eq!(s.credential_error, None);
}

#[tokio::test]
async fn a_failed_read_is_reprobed_at_most_once_per_interval() {
    let kc = SwitchKeychain::locked();
    let r = route("http://127.0.0.1:9", true);
    let h = handler_with(r.clone(), RouteKey::new(r, kc.clone()));
    assert!(h.check_credential().await.is_err());
    for _ in 0..20 {
        assert!(!h.status().await.credential_ready);
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        kc.reads.load(Ordering::SeqCst),
        1,
        "no re-read inside the interval: health polls never hammer the keychain"
    );
}

#[tokio::test]
async fn an_upstream_rejection_is_reported_until_the_key_is_repaired() {
    let (origin, g) = spawn_gateway(401).await;
    let kc = SwitchKeychain::holding("sk-revoked");
    let r = route(&origin, true);
    let mut h = handler_with(r.clone(), RouteKey::new(r, kc.clone()));
    h.health = Arc::new(CredentialHealth::new(Duration::ZERO, REJECTION_HOLD));
    let h = Arc::new(h);
    let relay = spawn_relay_for(h.clone()).await;

    let resp = relay.send("/u/deepseek/v1/messages", BODY_MIN).await;
    assert_eq!(
        error_code(resp).await,
        (403, "upstream_credential_rejected".into())
    );
    let s = h.status().await;
    assert!(!s.credential_ready);
    assert_eq!(s.credential_error, Some(UpstreamError::CredentialRejected));

    // Re-reading the same key does not clear the rejection.
    for _ in 0..5 {
        h.status().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        h.status().await.credential_error,
        Some(UpstreamError::CredentialRejected),
        "the unchanged key stays rejected"
    );

    // The key is replaced in the keychain: health sees it without a request.
    kc.set(Ok(Some("sk-replaced")));
    let s = status_until(&h, |s| s.credential_ready).await;
    assert!(s.credential_ready, "{s:?}");
    assert_eq!(g.hits.load(Ordering::SeqCst), 1, "recovery sent no request");

    g.status.store(200, Ordering::SeqCst);
    assert_eq!(
        relay
            .send("/u/deepseek/v1/messages", BODY_MIN)
            .await
            .status(),
        200
    );
    assert!(h.status().await.credential_ready);
}

#[test]
fn a_rejection_of_an_unchanged_key_expires_after_the_hold() {
    let health = CredentialHealth::new(Duration::ZERO, Duration::from_secs(60));
    let t0 = Instant::now();
    let k = credential_digest("sk-same");
    let t = health.begin();
    health.rejected(t, k, t0);
    health.read(health.begin(), Ok(k), t0 + Duration::from_secs(59));
    assert_eq!(health.error(), Some(UpstreamError::CredentialRejected));
    health.read(health.begin(), Ok(k), t0 + Duration::from_secs(61));
    assert_eq!(health.error(), None);
}

#[test]
fn an_older_acceptance_does_not_clear_a_newer_credential_failure() {
    let health = CredentialHealth::new(Duration::ZERO, REJECTION_HOLD);
    let t0 = Instant::now();
    let k = credential_digest("sk-old");
    // Request A read the key; then the key went missing for request B.
    health.read(health.begin(), Ok(k), t0);
    health.read(
        health.begin(),
        Err(UpstreamError::CredentialUnavailable),
        t0,
    );
    // A's upstream answer arrives last.
    health.accepted_credential(k);
    assert_eq!(health.error(), Some(UpstreamError::CredentialUnavailable));
    assert!(
        health.claim_probe(t0),
        "health keeps re-reading while the failure stands"
    );
}

#[test]
fn an_acceptance_clears_only_a_rejection_of_its_own_credential() {
    let health = CredentialHealth::new(Duration::ZERO, REJECTION_HOLD);
    let t0 = Instant::now();
    let (old, new) = (credential_digest("sk-old"), credential_digest("sk-new"));
    health.rejected(health.begin(), new, t0);
    health.accepted_credential(old);
    assert_eq!(health.error(), Some(UpstreamError::CredentialRejected));
    health.accepted_credential(new);
    assert_eq!(health.error(), None);
}

#[test]
fn a_probe_of_the_old_key_finishing_late_does_not_clear_a_newer_rejection() {
    let health = CredentialHealth::new(Duration::ZERO, REJECTION_HOLD);
    let t0 = Instant::now();
    let (old, new) = (credential_digest("sk-old"), credential_digest("sk-new"));
    // A probe starts reading while the old key is still in place...
    let probe = health.begin();
    // ...the key rotates; a request reads the new key and is refused.
    let request = health.begin();
    health.read(request, Ok(new), t0);
    health.rejected(request, new, t0);
    // The probe's read of the old key lands last.
    health.probed(probe, Ok(old), t0);
    assert_eq!(
        health.error(),
        Some(UpstreamError::CredentialRejected),
        "a read that began before the rejection is stale"
    );
    // A read that begins after it and finds a changed key still clears it.
    health.read(health.begin(), Ok(old), t0);
    assert_eq!(health.error(), None);
}

#[test]
fn a_late_rejection_of_a_key_a_newer_read_replaced_does_not_stand() {
    let health = CredentialHealth::new(Duration::ZERO, REJECTION_HOLD);
    let t0 = Instant::now();
    let (old, new) = (credential_digest("sk-old"), credential_digest("sk-new"));
    let slow = health.begin();
    health.read(slow, Ok(old), t0);
    // The key rotates and a newer request reads the new one.
    health.read(health.begin(), Ok(new), t0);
    // The slow request's 401 for the old key arrives after.
    health.rejected(slow, old, t0);
    assert_eq!(health.error(), None, "the current key was never rejected");
    // A late 401 for the key a newer read still holds does stand.
    let again = health.begin();
    health.read(again, Ok(new), t0);
    health.read(health.begin(), Ok(new), t0);
    health.rejected(again, new, t0);
    assert_eq!(health.error(), Some(UpstreamError::CredentialRejected));
}

// ---------------------------------------------------------------------
// Publishing and serving through `service`
// ---------------------------------------------------------------------

mod service_level {
    use super::*;
    use crate::relay::service::{self, ServiceError};
    use crate::routes::RouteStore;

    fn data_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        service::set_bind(d.path(), "127.0.0.1", Some(0), None).unwrap();
        d
    }

    fn store(d: &std::path::Path) -> RouteStore {
        RouteStore::open_at(d.join(service::ROUTES_FILENAME)).unwrap()
    }

    async fn serve(d: &std::path::Path) -> service::Served {
        service::serve_with_routes(
            d,
            |_| None,
            |target, r, _, _| {
                let t = Timeouts::default();
                let client = base_builder(&t).no_proxy().build().ok()?;
                Some(service::RouteHandler::PassThrough(
                    PassThroughHandler::with_client(
                        target,
                        r.clone(),
                        client,
                        t,
                        Arc::new(FakeKeychain(None)),
                    ),
                ))
            },
        )
        .await
        .unwrap()
    }

    /// Plain HTTP over the serving relay's own state, so the test needs
    /// no TLS client: what is under test is the grant check, not TLS. The
    /// handle is the listener's task: it holds that state, and with it the
    /// serve lock, until it has ended.
    async fn plain_listener(served: &service::Served) -> (Relay, tokio::task::JoinHandle<()>) {
        let state = served.running.state.clone();
        let issued = state
            .devices()
            .issue(
                "laptop",
                vec![Grant {
                    upstream_id: route_id(),
                    accepted_generation: 1,
                    epoch: state.epoch().to_owned(),
                }],
            )
            .unwrap();
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let shutdown = CancelToken::new();
        let limits = Limits::default();
        let task = tokio::spawn(server::serve(
            l,
            server::router(state),
            None,
            limits,
            shutdown.clone(),
        ));
        let relay = Relay {
            addr,
            token: issued.plaintext.expose().to_owned(),
            outcomes: Arc::new(Mutex::new(Vec::new())),
            shutdown,
        };
        (relay, task)
    }

    /// Stop both listeners over the served state and wait for them, so the
    /// serve lock is free for what follows.
    async fn stop_all(
        relay: Relay,
        listener: tokio::task::JoinHandle<()>,
        mut served: service::Served,
    ) {
        drop(relay);
        listener.await.unwrap();
        served.running.stop_and_drain().await;
    }

    const BODY: &str = r#"{"model":"claude-sonnet-4-5","max_tokens":1}"#;

    #[tokio::test]
    async fn an_edited_route_is_republished_and_the_old_grant_must_accept_again() {
        let d = data_dir();
        let (origin_a, a) = spawn_gateway(200).await;
        let added = store(d.path()).add(route(&origin_a, false)).unwrap();

        let (id, g1) = service::publish_route(d.path(), &added, None, None)
            .await
            .unwrap();
        assert_eq!((id.as_str(), g1), ("deepseek", 1));

        let served = serve(d.path()).await;
        assert!(served.republished.is_empty());
        assert!(served.warnings.is_empty(), "{:?}", served.warnings);
        let (relay, listener) = plain_listener(&served).await;
        assert_eq!(
            relay.send("/u/deepseek/v1/messages", BODY).await.status(),
            200
        );
        assert_eq!(a.hits.load(Ordering::SeqCst), 1);
        stop_all(relay, listener, served).await;

        // Point the route somewhere else while the relay is stopped.
        let (origin_b, b) = spawn_gateway(200).await;
        let mut edited = added.clone();
        if let RouteProvider::Gateway(c) = &mut edited.provider {
            c.base_url = format!("{origin_b}/anthropic");
        }
        store(d.path()).update(edited).unwrap();

        let served = serve(d.path()).await;
        assert_eq!(served.republished, vec![route_id()]);
        let (relay, listener) = plain_listener(&served).await;
        let resp = relay.send("/u/deepseek/v1/messages", BODY).await;
        assert_eq!(error_code(resp).await, (403, "acceptance_required".into()));
        assert_eq!(
            b.hits.load(Ordering::SeqCst),
            0,
            "nothing reached the new target"
        );
        stop_all(relay, listener, served).await;

        let s = service::status(d.path(), &[], chrono::Utc::now()).unwrap();
        let p = &s.publications[0];
        assert_eq!(p.generation, 2);
        assert_eq!(p.route_id, Some(added.id));
        assert_eq!(p.route_name.as_deref(), Some("DeepSeek"));
        assert_eq!(p.account_id, None);
    }

    #[tokio::test]
    async fn a_keychain_route_without_its_key_serves_with_a_warning() {
        let d = data_dir();
        let added = store(d.path())
            .add(route("http://127.0.0.1:9", false))
            .unwrap();
        service::publish_route(d.path(), &added, None, None)
            .await
            .unwrap();
        // Moved to the keychain afterwards, whose key the fake does not hold.
        let mut moved = added.clone();
        if let RouteProvider::Gateway(c) = &mut moved.provider {
            c.use_keychain = true;
            c.api_key.clear();
        }
        store(d.path()).update(moved).unwrap();

        let served = serve(d.path()).await;
        assert_eq!(served.republished, vec![route_id()]);
        assert_eq!(served.warnings.len(), 1);
        assert_eq!(served.warnings[0].code, "credential_unavailable");
    }

    #[tokio::test]
    async fn a_route_publication_whose_route_is_gone_is_reported_not_served() {
        let d = data_dir();
        let added = store(d.path())
            .add(route("http://127.0.0.1:9", false))
            .unwrap();
        service::publish_route(d.path(), &added, None, None)
            .await
            .unwrap();
        store(d.path()).remove(added.id).unwrap();

        let served = serve(d.path()).await;
        assert_eq!(served.warnings[0].code, "route_missing");
    }

    #[tokio::test]
    async fn a_route_that_is_not_a_gateway_cannot_be_published() {
        let d = data_dir();
        let mut r = route("http://127.0.0.1:9", false);
        r.provider = RouteProvider::Vertex(crate::routes::VertexConfig {
            project_id: "p".into(),
            region: None,
            base_url: None,
            skip_gcp_auth: false,
        });
        let err = service::publish_route(d.path(), &r, None, None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ServiceError::RouteSecret {
                error: RouteSecretError::NotRelayCapable(_),
                ..
            }
        ));
    }

    #[test]
    fn a_route_is_found_by_exact_id_name_or_wrapper_and_never_by_guess() {
        let a = route("http://x", false);
        let mut b = route("http://x", false);
        b.id = Uuid::from_u128(0xb0);
        b.name = "Kimi".into();
        b.wrapper_name = "claude-kimi".into();
        let routes = vec![a.clone(), b.clone()];
        for q in ["DeepSeek", "claude-deepseek", &a.id.to_string()] {
            assert_eq!(service::find_route(&routes, q).unwrap().id, a.id, "{q}");
        }
        assert!(matches!(
            service::find_route(&routes, "deep"),
            Err(ServiceError::NoSuchRoute(_))
        ));
        let mut dup = b.clone();
        dup.id = Uuid::from_u128(0xc0);
        let twins = vec![b, dup];
        assert!(matches!(
            service::find_route(&twins, "Kimi"),
            Err(ServiceError::AmbiguousRoute(_))
        ));
        assert_eq!(
            service::default_route_upstream_id(&a).unwrap().as_str(),
            "deepseek"
        );
    }
}

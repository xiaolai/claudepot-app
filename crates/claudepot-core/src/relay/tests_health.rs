//! `GET /relay/health` through a real loopback listener (plan P3 exit
//! criterion: the health auth matrix), and `client::probe` against it.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use super::cancel::CancelToken;
use super::client::{self, ClientError};
use super::devices::{DeviceRegistry, Grant, Revocation};
use super::forward::{base_builder, SubscriptionHandler, Timeouts};
use super::health::{HealthReport, OwnerState, UpstreamStatus, SALT_HEADER};
use super::identity::{self, TargetKind};
use super::lease::mock::MockBackend;
use super::lease::LeaseManager;
use super::secret::Secret;
use super::server::{self, Limits, RelayState};
use super::target::{Operation, UpstreamId};
use super::upstream::{
    AdmittedRequest, Capabilities, Outcome, OutcomeFuture, OutcomeKind, TargetIdentity,
    UpstreamError, UpstreamHandler, UpstreamResponse,
};

const SALT_A: &str = "0101010101010101010101010101010101010101010101010101010101010101";
const SALT_B: &str = "0202020202020202020202020202020202020202020202020202020202020202";

fn id(s: &str) -> UpstreamId {
    UpstreamId::parse(s).unwrap()
}

fn target(name: &str, kind: TargetKind, generation: u64) -> TargetIdentity {
    TargetIdentity {
        id: id(name),
        fingerprint: format!("digest-{name}"),
        generation,
        kind,
    }
}

fn grant(name: &str, generation: u64) -> Grant {
    Grant {
        upstream_id: id(name),
        accepted_generation: generation,
        epoch: String::new(),
    }
}

/// An upstream whose status is whatever the test says, and whose
/// requests complete at once with a fixed model and TTFB.
struct Fixed {
    target: TargetIdentity,
    caps: Capabilities,
    status: UpstreamStatus,
}

impl Fixed {
    fn new(target: TargetIdentity, status: UpstreamStatus) -> Arc<Self> {
        Arc::new(Self {
            target,
            caps: Capabilities {
                operations: [Operation::Messages].into(),
            },
            status,
        })
    }
}

#[async_trait::async_trait]
impl UpstreamHandler for Fixed {
    fn target(&self) -> &TargetIdentity {
        &self.target
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    async fn handle(
        &self,
        _req: AdmittedRequest,
        _cancel: CancelToken,
    ) -> Result<(UpstreamResponse, OutcomeFuture), UpstreamError> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Outcome {
            kind: OutcomeKind::Complete,
            status: Some(200),
            ttfb: Some(Duration::from_millis(250)),
            upstream_request_id: None,
            bytes: 2,
            usage: None,
            native_usage: None,
            model: Some("model-x".into()),
            error: None,
        });
        Ok((
            UpstreamResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Box::pin(futures_util::stream::once(async {
                    Ok(axum::body::Bytes::from_static(b"{}"))
                })),
            },
            rx,
        ))
    }
    async fn status(&self) -> UpstreamStatus {
        self.status.clone()
    }
}

fn ready(account: Option<(&str, &str)>) -> UpstreamStatus {
    UpstreamStatus {
        credential_ready: true,
        account: account.map(|(a, o)| (a.to_owned(), o.to_owned())),
        ..Default::default()
    }
}

struct Served {
    base: reqwest::Url,
    devices: DeviceRegistry,
    _shutdown: Stop,
}

struct Stop(CancelToken);
impl Drop for Stop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn serve(state: RelayState, devices: DeviceRegistry) -> Served {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let shutdown = CancelToken::new();
    tokio::spawn(server::serve(
        l,
        server::router(Arc::new(state)),
        None,
        Limits::default(),
        shutdown.clone(),
    ));
    Served {
        base: format!("http://{addr}").parse().unwrap(),
        devices,
        _shutdown: Stop(shutdown),
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

async fn get_health(
    base: &reqwest::Url,
    token: Option<&str>,
    salt: Option<&str>,
) -> (u16, serde_json::Value) {
    let mut req = http().get(base.join("/relay/health").unwrap());
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(s) = salt {
        req = req.header(SALT_HEADER, s);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(serde_json::Value::Null))
}

/// Two upstreams served, `sub` granted, `ds` published but not granted.
async fn two_upstreams() -> (Served, String, String) {
    let devices = DeviceRegistry::new();
    let issued = devices.issue("laptop", vec![grant("sub", 1)]).unwrap();
    let state = RelayState::new(devices.clone(), Limits::default())
        .with_server_id("srv-1")
        .with_upstream(
            Fixed::new(
                target("sub", TargetKind::Subscription, 1),
                ready(Some(("acct", "org"))),
            ),
            true,
        )
        .with_upstream(
            Fixed::new(target("ds", TargetKind::PassThrough, 1), ready(None)),
            false,
        );
    let served = serve(state, devices).await;
    (
        served,
        issued.plaintext.expose().to_owned(),
        issued.device_id,
    )
}

#[tokio::test]
async fn health_auth_matrix() {
    let (s, token, device_id) = two_upstreams().await;
    let liveness = serde_json::json!({ "ok": true });

    // Every caller that is not a paired device learns liveness and
    // nothing else — not even that its token was wrong.
    let bare = token.trim_start_matches("cprl_").to_owned();
    let wrong = format!("cprl_{}", "0".repeat(64));
    for t in [
        None,
        Some(wrong.as_str()),
        Some(bare.as_str()),
        Some("sk-ant-oat01-x"),
    ] {
        let (status, body) = get_health(&s.base, t, Some(SALT_A)).await;
        assert_eq!((status, &body), (200, &liveness), "{t:?}");
    }

    // A paired device must say whose salt to key fingerprints by.
    for salt in [None, Some("zz"), Some("ab")] {
        let (status, body) = get_health(&s.base, Some(&token), salt).await;
        assert_eq!(status, 400, "{salt:?}");
        assert_eq!(body["error"]["message"], "bad_request");
    }

    let (status, body) = get_health(&s.base, Some(&token), Some(SALT_A)).await;
    assert_eq!(status, 200);
    let r: HealthReport = serde_json::from_value(body).unwrap();
    assert_eq!(r.server_id, "srv-1");
    assert_eq!(r.version, env!("CARGO_PKG_VERSION"));
    let ids: Vec<_> = r.upstreams.iter().map(|u| u.id.as_str()).collect();
    assert_eq!(ids, ["sub"], "an ungranted upstream is not disclosed");
    let sub = &r.upstreams[0];
    assert!(sub.default && sub.accepted && sub.credential_ready);
    assert_eq!(sub.capacity_free, Limits::default().per_device);
    assert_eq!(
        sub.target_fingerprint,
        identity::client_fingerprint(&identity::parse_salt(SALT_A).unwrap(), "digest-sub"),
        "the client sees the salted fingerprint, never the server digest"
    );

    // Health is a control path; under an upstream prefix it is 404.
    let resp = http()
        .get(s.base.join("/u/sub/relay/health").unwrap())
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    assert_eq!(
        s.devices.revoke(&device_id).unwrap(),
        Some(Revocation::Revoked)
    );
    let (status, body) = get_health(&s.base, Some(&token), Some(SALT_A)).await;
    assert_eq!(
        (status, body),
        (200, liveness),
        "a revoked token is a stranger"
    );
}

#[tokio::test]
async fn readiness_is_per_upstream_in_both_directions() {
    let devices = DeviceRegistry::new();
    let issued = devices
        .issue("laptop", vec![grant("sub", 1), grant("ds", 1)])
        .unwrap();
    let blocked = UpstreamStatus {
        credential_ready: false,
        credential_error: Some(UpstreamError::OwnerConflict),
        owner_state: Some(OwnerState::Conflict),
        ..Default::default()
    };
    let state = RelayState::new(devices.clone(), Limits::default())
        .with_server_id("srv")
        .with_upstream(
            Fixed::new(target("sub", TargetKind::Subscription, 1), blocked.clone()),
            true,
        )
        .with_upstream(
            Fixed::new(target("ds", TargetKind::PassThrough, 1), ready(None)),
            false,
        );
    let s = serve(state, devices).await;
    let token = issued.plaintext.expose().to_owned();
    let (_, body) = get_health(&s.base, Some(&token), Some(SALT_A)).await;
    let r: HealthReport = serde_json::from_value(body).unwrap();
    let by = |n: &str| r.upstreams.iter().find(|u| u.id == n).unwrap().clone();
    let (ds, sub) = (by("ds"), by("sub"));
    assert!(
        ds.credential_ready,
        "a blocked subscription leaves the route ready"
    );
    assert!(!sub.credential_ready);
    assert_eq!(sub.owner_state, Some(OwnerState::Conflict));
    assert_eq!(sub.credential_error.as_deref(), Some("owner_conflict"));
    assert_eq!(ds.owner_state, None);

    // And the reverse: a route with no usable key leaves the
    // subscription ready.
    let devices = DeviceRegistry::new();
    let issued = devices
        .issue("laptop", vec![grant("sub", 1), grant("ds", 1)])
        .unwrap();
    let state = RelayState::new(devices.clone(), Limits::default())
        .with_upstream(
            Fixed::new(target("sub", TargetKind::Subscription, 1), ready(None)),
            true,
        )
        .with_upstream(
            Fixed::new(
                target("ds", TargetKind::PassThrough, 1),
                UpstreamStatus::default(),
            ),
            false,
        );
    let s = serve(state, devices).await;
    let (_, body) = get_health(&s.base, Some(issued.plaintext.expose()), Some(SALT_A)).await;
    let r: HealthReport = serde_json::from_value(body).unwrap();
    let ready_ids: Vec<_> = r
        .upstreams
        .iter()
        .filter(|u| u.credential_ready)
        .map(|u| u.id.as_str())
        .collect();
    assert_eq!(ready_ids, ["sub"]);
}

#[tokio::test]
async fn an_older_accepted_generation_is_reported_not_accepted() {
    let devices = DeviceRegistry::new();
    let issued = devices.issue("laptop", vec![grant("sub", 1)]).unwrap();
    let state = RelayState::new(devices.clone(), Limits::default()).with_upstream(
        Fixed::new(target("sub", TargetKind::Subscription, 2), ready(None)),
        true,
    );
    let s = serve(state, devices).await;
    let (_, body) = get_health(&s.base, Some(issued.plaintext.expose()), Some(SALT_A)).await;
    let u = &serde_json::from_value::<HealthReport>(body)
        .unwrap()
        .upstreams[0];
    assert_eq!(
        (u.accepted_generation, u.current_generation, u.accepted),
        (1, 2, false)
    );
}

#[tokio::test]
async fn account_fingerprints_match_across_servers_for_one_client_only() {
    let mut seen = Vec::new();
    for _server in 0..2 {
        let (s, token, _) = two_upstreams().await;
        for salt in [SALT_A, SALT_B] {
            let (_, body) = get_health(&s.base, Some(&token), Some(salt)).await;
            let r: HealthReport = serde_json::from_value(body).unwrap();
            seen.push(r.upstreams[0].account_fingerprint.clone().unwrap());
        }
    }
    // [srv1/A, srv1/B, srv2/A, srv2/B]
    assert_eq!(seen[0], seen[2], "one client, two servers, one account");
    assert_eq!(seen[1], seen[3]);
    assert_ne!(seen[0], seen[1], "two clients cannot correlate");
}

#[tokio::test]
async fn a_completed_request_appears_as_a_latency_sample_and_probe_reads_it() {
    let (s, token, _) = two_upstreams().await;
    let resp = http()
        .post(s.base.join("/v1/messages").unwrap())
        .bearer_auth(&token)
        .header("content-type", "application/json")
        .body(r#"{"model":"sonnet","max_tokens":1}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();

    let secret = Secret::new(token.clone());
    let mut probe = None;
    for _ in 0..50 {
        let p = client::probe(&s.base, None, &secret, SALT_A).await.unwrap();
        if !p.health.upstreams[0].latency.is_empty() {
            probe = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let p = probe.expect("the outcome reaches the stats");
    let l = &p.health.upstreams[0].latency;
    assert_eq!(l.len(), 1);
    assert_eq!(
        (l[0].model.as_str(), l[0].count, l[0].median_ttfb_ms),
        ("model-x", 1, 250),
        "keyed by the model that answered, not the one asked for"
    );
    assert_eq!(p.health.server_id, "srv-1");
}

#[tokio::test]
async fn the_outcome_record_carries_the_requested_model() {
    let devices = DeviceRegistry::new();
    let issued = devices.issue("laptop", vec![grant("sub", 1)]).unwrap();
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let state = RelayState::new(devices.clone(), Limits::default())
        .with_upstream(
            Fixed::new(target("sub", TargetKind::Subscription, 1), ready(None)),
            true,
        )
        .with_sink(Arc::new(move |r| sink.lock().push(r)));
    let s = serve(state, devices).await;
    let resp = http()
        .post(s.base.join("/v1/messages").unwrap())
        .bearer_auth(issued.plaintext.expose())
        .body(r#"{"model":"sonnet","max_tokens":1}"#)
        .send()
        .await
        .unwrap();
    let _ = resp.bytes().await.unwrap();
    for _ in 0..50 {
        if !seen.lock().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let recs = seen.lock();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].requested_model.as_deref(), Some("sonnet"));
}

/// Fails before any response header, as a connect error does.
struct Unreachable(TargetIdentity, Capabilities);

#[async_trait::async_trait]
impl UpstreamHandler for Unreachable {
    fn target(&self) -> &TargetIdentity {
        &self.0
    }
    fn capabilities(&self) -> &Capabilities {
        &self.1
    }
    async fn handle(
        &self,
        _req: AdmittedRequest,
        _cancel: CancelToken,
    ) -> Result<(UpstreamResponse, OutcomeFuture), UpstreamError> {
        Err(UpstreamError::Transport)
    }
}

#[tokio::test]
async fn a_failure_before_response_headers_is_recorded_as_failed() {
    let devices = DeviceRegistry::new();
    let issued = devices.issue("laptop", vec![grant("sub", 1)]).unwrap();
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let handler = Arc::new(Unreachable(
        target("sub", TargetKind::Subscription, 1),
        Capabilities {
            operations: [Operation::Messages].into(),
        },
    ));
    let state = RelayState::new(devices.clone(), Limits::default())
        .with_upstream(handler, true)
        .with_sink(Arc::new(move |r| sink.lock().push(r)));
    let s = serve(state, devices).await;
    let resp = http()
        .post(s.base.join("/v1/messages").unwrap())
        .bearer_auth(issued.plaintext.expose())
        .body(r#"{"model":"sonnet","max_tokens":1}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 502);
    for _ in 0..50 {
        if !seen.lock().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let recs = seen.lock();
    assert_eq!(recs.len(), 1, "the failure reaches the ledger");
    let o = &recs[0].outcome;
    assert_eq!(o.kind, OutcomeKind::Failed);
    assert_eq!((o.status, o.ttfb, o.bytes), (None, None, 0));
    assert_eq!(o.error.as_deref(), Some("upstream_unreachable"));
    assert_eq!(recs[0].requested_model.as_deref(), Some("sonnet"));
}

#[tokio::test]
async fn probe_refuses_a_stranger_and_an_unpinned_remote_server() {
    let (s, _, _) = two_upstreams().await;
    let e = client::probe(
        &s.base,
        None,
        &Secret::new(format!("cprl_{}", "0".repeat(64))),
        SALT_A,
    )
    .await
    .unwrap_err();
    assert!(matches!(e, ClientError::NotPaired), "{e:?}");

    let remote: reqwest::Url = "http://100.64.0.9:8787".parse().unwrap();
    let e = client::probe(&remote, None, &Secret::new(String::from("cprl_x")), SALT_A)
        .await
        .unwrap_err();
    assert!(matches!(e, ClientError::Unpinned), "{e:?}");
}

#[tokio::test]
async fn a_stalled_loopback_server_times_out_instead_of_hanging_the_probe() {
    // Accepts the connection and never answers.
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base: reqwest::Url = format!("http://{}", l.local_addr().unwrap())
        .parse()
        .unwrap();
    let _hold = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((c, _)) = l.accept().await {
            held.push(c);
        }
    });
    let started = std::time::Instant::now();
    let e = client::probe_within(
        &base,
        None,
        &Secret::new(String::from("cprl_x")),
        SALT_A,
        std::time::Duration::from_millis(200),
    )
    .await
    .unwrap_err();
    assert!(matches!(e, ClientError::Transport), "{e:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn an_oversized_control_response_is_refused_unread() {
    let origin = upstream_replying(|| {
        let mut big = String::from("{\"server_id\":\"");
        big.push_str(&"x".repeat(2 * 1024 * 1024));
        big.push_str("\"}");
        big.into_response()
    })
    .await;
    let base: reqwest::Url = origin.parse().unwrap();
    let e = client::probe(&base, None, &Secret::new(String::from("cprl_x")), SALT_A)
        .await
        .unwrap_err();
    assert!(matches!(e, ClientError::BadResponse), "{e:?}");
}

// ---------------------------------------------------------------------
// The subscription handler's own status
// ---------------------------------------------------------------------

async fn upstream_replying(resp: fn() -> Response) -> String {
    let app = Router::new().fallback(move || async move { resp() });
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

async fn subscription(origin: &str, backend: Arc<MockBackend>) -> (Served, String) {
    let timeouts = Timeouts::default();
    let handler = SubscriptionHandler::with_client(
        target("sub", TargetKind::Subscription, 1),
        Arc::new(LeaseManager::new(backend)),
        base_builder(&timeouts).no_proxy().build().unwrap(),
        origin.to_owned(),
        timeouts,
    );
    let devices = DeviceRegistry::new();
    let issued = devices.issue("laptop", vec![grant("sub", 1)]).unwrap();
    let state =
        RelayState::new(devices.clone(), Limits::default()).with_upstream(Arc::new(handler), true);
    (
        serve(state, devices).await,
        issued.plaintext.expose().to_owned(),
    )
}

async fn send(s: &Served, token: &str) -> u16 {
    let resp = http()
        .post(s.base.join("/v1/messages").unwrap())
        .bearer_auth(token)
        .body("{}")
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let _ = resp.bytes().await;
    status
}

#[tokio::test]
async fn a_429_puts_the_subscription_in_cooldown_for_its_retry_after() {
    let origin = upstream_replying(|| {
        Response::builder()
            .status(429)
            .header("retry-after", "120")
            .body(Body::from("{}"))
            .unwrap()
    })
    .await;
    let (s, token) = subscription(&origin, MockBackend::new("sk-ant-oat01-T", 3600)).await;
    let (_, before) = get_health(&s.base, Some(&token), Some(SALT_A)).await;
    assert!(before["upstreams"][0]["cooldown_secs"].is_null());

    assert_eq!(send(&s, &token).await, 429, "the 429 itself is forwarded");
    let (_, after) = get_health(&s.base, Some(&token), Some(SALT_A)).await;
    let r: HealthReport = serde_json::from_value(after).unwrap();
    let left = r.upstreams[0].cooldown_secs.unwrap();
    assert!((110..=120).contains(&left), "{left}");
    assert!(
        r.upstreams[0].credential_ready,
        "a cooldown is not a credential fault"
    );
}

#[tokio::test]
async fn an_owner_conflict_is_reported_and_a_cold_lease_is_ready() {
    let origin = upstream_replying(|| StatusCode::OK.into_response()).await;
    let backend = MockBackend::new("sk-ant-oat01-T", 3600);
    let (s, token) = subscription(&origin, backend.clone()).await;

    let (_, cold) = get_health(&s.base, Some(&token), Some(SALT_A)).await;
    let r: HealthReport = serde_json::from_value(cold).unwrap();
    assert!(
        r.upstreams[0].credential_ready,
        "never-acquired is not a fault"
    );
    assert_eq!(r.upstreams[0].owner_state, Some(OwnerState::Clear));
    assert!(
        r.upstreams[0].account_fingerprint.is_none(),
        "no identity is known before the first acquisition"
    );

    backend.cc_owns.store(true, Ordering::SeqCst);
    assert_eq!(send(&s, &token).await, 403);
    let (_, after) = get_health(&s.base, Some(&token), Some(SALT_A)).await;
    let r: HealthReport = serde_json::from_value(after).unwrap();
    let u = &r.upstreams[0];
    assert!(!u.credential_ready);
    assert_eq!(u.owner_state, Some(OwnerState::Conflict));
    assert_eq!(u.credential_error.as_deref(), Some("owner_conflict"));
}

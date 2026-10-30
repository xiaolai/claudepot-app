//! Mock-upstream suite for the relay (plan P1 exit criteria).
//!
//! Each test runs a real relay listener on loopback in front of a real
//! loopback "Anthropic", so what is asserted is what crosses a socket:
//! status lines, header sets and body bytes.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::StreamExt;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::cancel::CancelToken;
use super::devices::{DeviceRegistry, Grant, Revocation};
use super::forward::{base_builder, SubscriptionHandler, Timeouts};
use super::lease::mock::MockBackend;
use super::lease::LeaseManager;
use super::server::{self, Limits, OutcomeRecord, RelayState};
use super::target::UpstreamId;
use super::upstream::{
    AdmittedRequest, Capabilities, OutcomeFuture, OutcomeKind, TargetIdentity, UpstreamError,
    UpstreamHandler, UpstreamResponse,
};

const OAUTH: &str = "sk-ant-oat01-CANARYoauthTOKEN";

// ---------------------------------------------------------------------
// The mock upstream
// ---------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Mode {
    /// 200 SSE, the fixed chunks below, split across writes.
    Sse,
    /// 401 for the initial token, 200 SSE for any renewed one.
    RejectInitial,
    /// 401 for every token.
    RejectAll,
    Redirect,
    Status(u16),
    /// One chunk every `every`, `count` times.
    Drip {
        every: Duration,
        count: usize,
    },
    /// One chunk, then nothing, ever.
    Hang,
    /// Headers only after this long.
    SlowHeaders(Duration),
}

const SSE: &[&[u8]] = &[
    b"event: message_start\r\ndata: {\"type\":\"message_start\"}\r\n\r\n",
    b"event: content_block_delta\r\ndata: {\"type\":\"content_",
    b"block_delta\",\"delta\":{\"text\":\"h\xc3",
    b"\xa9llo\"}}\r\n\r\n",
    b"event: message_stop\r\ndata: {\"type\":\"message_stop\"}\r\n\r\n",
];

struct Seen {
    path: String,
    headers: HeaderMap,
    body: Bytes,
}

struct Upstream {
    mode: Mutex<Mode>,
    hits: AtomicUsize,
    seen: Mutex<Vec<Seen>>,
    /// Set when a producer notices nobody is reading any more.
    abandoned: Arc<AtomicBool>,
}

async fn upstream_handler(State(u): State<Arc<Upstream>>, req: Request) -> Response {
    u.hits.fetch_add(1, Ordering::SeqCst);
    let path = req.uri().to_string();
    let headers = req.headers().clone();
    let body = axum::body::to_bytes(req.into_body(), usize::MAX)
        .await
        .unwrap();
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    u.seen.lock().push(Seen {
        path,
        headers,
        body,
    });
    let mode = *u.mode.lock();
    match mode {
        Mode::Sse => sse(),
        Mode::RejectInitial if auth == format!("Bearer {OAUTH}") => {
            StatusCode::UNAUTHORIZED.into_response()
        }
        Mode::RejectInitial => sse(),
        Mode::RejectAll => StatusCode::UNAUTHORIZED.into_response(),
        Mode::Redirect => Response::builder()
            .status(307)
            .header("location", "http://127.0.0.1:9/elsewhere")
            .body(Body::empty())
            .unwrap(),
        Mode::Status(s) => Response::builder()
            .status(s)
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"type":"error","error":{"type":"api_error","message":"x"}}"#,
            ))
            .unwrap(),
        Mode::Drip { every, count } => {
            let s = futures_util::stream::unfold(0usize, move |i| async move {
                if i == count {
                    return None;
                }
                tokio::time::sleep(every).await;
                Some((
                    Ok::<_, std::io::Error>(Bytes::from_static(b"data: tick\n\n")),
                    i + 1,
                ))
            });
            Response::new(Body::from_stream(s))
        }
        Mode::Hang => {
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
            let abandoned = u.abandoned.clone();
            tokio::spawn(async move {
                let _ = tx.send(Ok(Bytes::from_static(b"data: first\n\n"))).await;
                // Keep probing: `send` fails once the body is dropped,
                // which is what a relay that cancels upstream causes.
                loop {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    if tx.is_closed() {
                        abandoned.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            });
            let s = tokio_stream_from(rx);
            Response::new(Body::from_stream(s))
        }
        Mode::SlowHeaders(d) => {
            tokio::time::sleep(d).await;
            sse()
        }
    }
}

fn tokio_stream_from(
    rx: tokio::sync::mpsc::Receiver<Result<Bytes, std::io::Error>>,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> {
    futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|i| (i, rx)) })
}

fn sse() -> Response {
    let s = futures_util::stream::unfold(0usize, |i| async move {
        let chunk = SSE.get(i)?;
        tokio::time::sleep(Duration::from_millis(2)).await;
        Some((Ok::<_, std::io::Error>(Bytes::from_static(chunk)), i + 1))
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .header("request-id", "req_mock")
        .header("set-cookie", "tracking=1")
        .body(Body::from_stream(s))
        .unwrap()
}

async fn spawn_upstream(mode: Mode) -> (String, Arc<Upstream>) {
    let u = Arc::new(Upstream {
        mode: Mutex::new(mode),
        hits: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
        abandoned: Arc::new(AtomicBool::new(false)),
    });
    let app = Router::new()
        .fallback(upstream_handler)
        .with_state(u.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), u)
}

// ---------------------------------------------------------------------
// The relay under test
// ---------------------------------------------------------------------

struct Relay {
    addr: SocketAddr,
    devices: DeviceRegistry,
    /// What the router serves from; the revoke socket is armed against it.
    #[cfg_attr(not(unix), allow(dead_code))]
    state: Arc<RelayState>,
    token: String,
    device_id: String,
    backend: Arc<MockBackend>,
    outcomes: Arc<Mutex<Vec<OutcomeRecord>>>,
    shutdown: ShutdownOnDrop,
    serve_task: Option<tokio::task::JoinHandle<()>>,
}

struct ShutdownOnDrop(CancelToken);
impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn sub_id() -> UpstreamId {
    UpstreamId::parse("sub").unwrap()
}

fn sub_target() -> TargetIdentity {
    TargetIdentity {
        id: sub_id(),
        fingerprint: "fp-sub".into(),
        generation: 1,
        kind: crate::relay::identity::TargetKind::Subscription,
    }
}

fn grants() -> Vec<Grant> {
    vec![Grant {
        upstream_id: sub_id(),
        accepted_generation: 1,
        epoch: String::new(),
    }]
}

async fn spawn_relay_with(
    origin: &str,
    limits: Limits,
    timeouts: Timeouts,
    extra: Option<Arc<dyn UpstreamHandler>>,
) -> Relay {
    let backend = MockBackend::new(OAUTH, 3600);
    let lease = Arc::new(LeaseManager::new(backend.clone()));
    let client = base_builder(&timeouts).no_proxy().build().unwrap();
    let handler =
        SubscriptionHandler::with_client(sub_target(), lease, client, origin.to_owned(), timeouts);
    let devices = DeviceRegistry::new();
    let issued = devices.issue("laptop", grants()).unwrap();
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let sink_outcomes = outcomes.clone();
    let mut state = RelayState::new(devices.clone(), limits)
        .with_upstream(Arc::new(handler), true)
        .with_sink(Arc::new(move |r| sink_outcomes.lock().push(r)));
    if let Some(h) = extra {
        state = state.with_upstream(h, false);
    }
    let state = Arc::new(state);
    let router = server::router(state.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let shutdown = CancelToken::new();
    let serve_task = tokio::spawn(server::serve(l, router, None, limits, shutdown.clone()));
    Relay {
        addr,
        devices,
        state,
        token: issued.plaintext.expose().to_owned(),
        device_id: issued.device_id,
        backend,
        outcomes,
        shutdown: ShutdownOnDrop(shutdown),
        serve_task: Some(serve_task),
    }
}

async fn spawn_relay(origin: &str) -> Relay {
    spawn_relay_with(origin, Limits::default(), Timeouts::default(), None).await
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

impl Relay {
    /// Stop the listener and wait for `serve` to return, as
    /// `Running::stop_and_drain` does.
    async fn stop(&mut self) {
        self.shutdown.0.cancel();
        if let Some(t) = self.serve_task.take() {
            t.await.unwrap();
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        client()
            .post(self.url(path))
            .bearer_auth(&self.token)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(r#"{"model":"claude-sonnet-4-5","max_tokens":1}"#)
    }
}

async fn error_code(resp: reqwest::Response) -> (u16, String) {
    let status = resp.status().as_u16();
    let v: serde_json::Value = resp.json().await.unwrap();
    (status, v["error"]["message"].as_str().unwrap().to_owned())
}

/// A request line the HTTP client libraries would normalise away.
async fn raw(addr: SocketAddr, method: &str, target: &str, token: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {target} HTTP/1.1\r\nHost: relay\r\n{auth}Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out).await;
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (status, text)
}

// ---------------------------------------------------------------------
// Auth matrix
// ---------------------------------------------------------------------

#[tokio::test]
async fn auth_matrix() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let r = spawn_relay(&origin).await;
    let c = client();
    let path = "/v1/messages?beta=true";

    // Hello needs no credential, bare and prefixed.
    for p in ["/api/hello", "/u/sub/api/hello"] {
        let resp = c.head(r.url(p)).send().await.unwrap();
        assert_eq!(resp.status(), 200, "{p}");
    }

    let no_token = c.post(r.url(path)).body("{}").send().await.unwrap();
    assert_eq!(
        error_code(no_token).await,
        (403, "relay_unauthenticated".into())
    );

    let wrong = c
        .post(r.url(path))
        .bearer_auth("cprl_0000000000000000000000000000000000000000000000000000000000000000")
        .send()
        .await
        .unwrap();
    assert_eq!(
        error_code(wrong).await,
        (403, "relay_unauthenticated".into())
    );

    // A remote-control-shaped token (bare hex) and the OAuth token itself.
    let bare = r.token.trim_start_matches("cprl_").to_owned();
    for t in [bare.as_str(), OAUTH] {
        let resp = c.post(r.url(path)).bearer_auth(t).send().await.unwrap();
        assert_eq!(error_code(resp).await.0, 403);
    }

    // The token in x-api-key instead of Authorization is not accepted.
    let api_key = c
        .post(r.url(path))
        .header("x-api-key", &r.token)
        .send()
        .await
        .unwrap();
    assert_eq!(api_key.status(), 403);

    assert_eq!(
        up.hits.load(Ordering::SeqCst),
        0,
        "no refusal reached upstream"
    );

    let ok = r.post(path).send().await.unwrap();
    assert_eq!(ok.status(), 200);
    let _ = ok.bytes().await.unwrap();
    assert_eq!(up.hits.load(Ordering::SeqCst), 1);

    assert_eq!(
        r.devices.revoke(&r.device_id).unwrap(),
        Some(Revocation::Revoked)
    );
    let revoked = r.post(path).send().await.unwrap();
    assert_eq!(
        error_code(revoked).await,
        (403, "relay_unauthenticated".into())
    );
    assert_eq!(up.hits.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------
// Parser refusals, through the socket
// ---------------------------------------------------------------------

#[tokio::test]
async fn malformed_targets_are_404_after_auth_and_403_before_it() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let r = spawn_relay(&origin).await;
    let targets = [
        "//v1/messages",
        "/v1//messages",
        "/v1/messages/",
        "/u//v1/messages",
        "/u/sub/../v1/messages",
        "/u/sub/./v1/messages",
        "/u/a%2F..%2Fsub/v1/messages",
        "/u/%73ub/v1/messages",
        "/v1/messages?beta=true&model=x",
        "/u/sub/relay/health",
        "/v1/models",
        "http://api.anthropic.com/v1/messages",
    ];
    for t in targets {
        let (status, _) = raw(r.addr, "POST", t, Some(&r.token)).await;
        assert!(status == 404 || status == 400, "{t}: {status}");
        let (status, _) = raw(r.addr, "POST", t, None).await;
        assert!(
            status == 403 || status == 400,
            "{t} unauthenticated: {status}"
        );
    }
    assert_eq!(up.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_ungranted_upstream_is_refused_and_never_falls_back_to_the_subscription() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let r = spawn_relay(&origin).await;
    let resp = r.post("/u/deepseek/v1/messages").send().await.unwrap();
    assert_eq!(error_code(resp).await, (403, "upstream_not_granted".into()));
    assert_eq!(up.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_acceptance_query_reports_whether_the_grant_covers_the_current_target() {
    let (origin, _up) = spawn_upstream(Mode::Sse).await;
    let r = spawn_relay(&origin).await;
    let old = r
        .devices
        .issue(
            "old",
            vec![Grant {
                upstream_id: sub_id(),
                accepted_generation: 0,
                epoch: String::new(),
            }],
        )
        .unwrap();
    let salt = "00".repeat(crate::relay::identity::MIN_SALT_BYTES);
    let query = |token: String| {
        let salt = salt.clone();
        let url = r.url("/relay/accept");
        async move {
            let resp = client()
                .post(url)
                .bearer_auth(token)
                .json(&serde_json::json!({"upstream": "sub", "client_salt": salt}))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 200);
            resp.json::<serde_json::Value>().await.unwrap()
        }
    };
    let current = query(r.token.clone()).await;
    assert_eq!(current["accepted"], true, "{current}");
    assert_eq!(current["generation"], 1);
    let stale = query(old.plaintext.expose().to_owned()).await;
    assert_eq!(stale["accepted"], false, "{stale}");
    // A query recorded nothing: the stale grant is still refused.
    let resp = client()
        .post(r.url("/u/sub/v1/messages"))
        .bearer_auth(old.plaintext.expose())
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(error_code(resp).await, (403, "acceptance_required".into()));
}

#[tokio::test]
async fn a_grant_for_an_older_generation_is_acceptance_required() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let r = spawn_relay(&origin).await;
    let old = r
        .devices
        .issue(
            "old",
            vec![Grant {
                upstream_id: sub_id(),
                accepted_generation: 0,
                epoch: String::new(),
            }],
        )
        .unwrap();
    let resp = client()
        .post(r.url("/u/sub/v1/messages"))
        .bearer_auth(old.plaintext.expose())
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(error_code(resp).await, (403, "acceptance_required".into()));
    assert_eq!(up.hits.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------
// Byte-identical forwarding
// ---------------------------------------------------------------------

#[tokio::test]
async fn request_and_response_bytes_cross_unchanged() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let r = spawn_relay(&origin).await;
    // Not valid JSON on purpose: the forwarder must not parse the body.
    let body: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    let resp = client()
        .post(r.url("/u/sub/v1/messages?beta=true"))
        .bearer_auth(&r.token)
        .header("x-api-key", "sk-ant-api03-must-not-cross")
        .header("anthropic-beta", "claude-code-20250219")
        .header("x-stainless-lang", "js")
        .header("cookie", "a=b")
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["content-type"], "text/event-stream");
    assert_eq!(resp.headers()["request-id"], "req_mock");
    assert!(resp.headers().get("set-cookie").is_none());
    let got = resp.bytes().await.unwrap();
    assert_eq!(got.as_ref(), SSE.concat().as_slice());

    let seen = up.seen.lock();
    let s = &seen[0];
    assert_eq!(
        s.path, "/v1/messages?beta=true",
        "prefix stripped, query kept"
    );
    assert_eq!(s.body.as_ref(), body.as_slice());
    assert_eq!(s.headers["authorization"], format!("Bearer {OAUTH}"));
    assert_eq!(
        s.headers["anthropic-beta"],
        "claude-code-20250219,oauth-2025-04-20"
    );
    assert_eq!(s.headers["x-stainless-lang"], "js");
    assert!(s.headers.get("x-api-key").is_none());
    assert!(s.headers.get("cookie").is_none());
    let all: String = s
        .headers
        .values()
        .map(|v| v.to_str().unwrap_or_default().to_owned())
        .collect();
    assert!(!all.contains(&r.token), "the device token never crosses");
}

#[tokio::test]
async fn upstream_errors_are_forwarded_as_the_upstream_sent_them() {
    let (origin, _up) = spawn_upstream(Mode::Status(529)).await;
    let r = spawn_relay(&origin).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(resp.status(), 529);
    let _ = resp.bytes().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.outcomes.lock()[0].outcome.kind, OutcomeKind::Failed);
}

// ---------------------------------------------------------------------
// Redirects and transport failures
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_redirect_is_refused_not_followed() {
    let (origin, up) = spawn_upstream(Mode::Redirect).await;
    let r = spawn_relay(&origin).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(error_code(resp).await, (502, "upstream_redirect".into()));
    assert_eq!(up.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_unreachable_upstream_is_a_fixed_502() {
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let r = spawn_relay(&origin).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(error_code(resp).await, (502, "upstream_unreachable".into()));
}

// ---------------------------------------------------------------------
// 401: one renewal, under concurrency
// ---------------------------------------------------------------------

#[tokio::test]
async fn thirty_two_concurrent_401s_cause_one_refresh() {
    let (origin, up) = spawn_upstream(Mode::RejectInitial).await;
    let limits = Limits {
        per_device: 32,
        ..Limits::default()
    };
    let r = Arc::new(spawn_relay_with(&origin, limits, Timeouts::default(), None).await);
    let tasks: Vec<_> = (0..32)
        .map(|_| {
            let r = r.clone();
            tokio::spawn(async move {
                let resp = r.post("/v1/messages").send().await.unwrap();
                let status = resp.status();
                let _ = resp.bytes().await;
                status
            })
        })
        .collect();
    for t in tasks {
        assert_eq!(t.await.unwrap(), 200);
    }
    assert_eq!(r.backend.refreshes(), 1);
    // 32 rejected + 32 retried.
    assert_eq!(up.hits.load(Ordering::SeqCst), 64);
}

#[tokio::test]
async fn a_second_401_is_a_403_credential_rejected() {
    let (origin, up) = spawn_upstream(Mode::RejectAll).await;
    let r = spawn_relay(&origin).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(
        error_code(resp).await,
        (403, "upstream_credential_rejected".into())
    );
    assert_eq!(up.hits.load(Ordering::SeqCst), 2);
    assert_eq!(r.backend.refreshes(), 1);
    // Health must not keep scoring the rejected credential as ready.
    let salt = "00".repeat(crate::relay::identity::MIN_SALT_BYTES);
    let health: serde_json::Value = client()
        .get(r.url("/relay/health"))
        .bearer_auth(&r.token)
        .header(crate::relay::health::SALT_HEADER, salt)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sub = &health["upstreams"][0];
    assert_eq!(sub["credential_ready"], false, "{health}");
    assert_eq!(
        sub["credential_error"], "upstream_credential_rejected",
        "{health}"
    );
}

#[tokio::test]
async fn an_accepted_request_clears_a_recorded_rejection_only() {
    let lease = LeaseManager::new(MockBackend::new(OAUTH, 3600));
    lease.rejected();
    assert!(!lease.readiness().ready());
    lease.accepted();
    assert!(lease.readiness().ready());
}

// ---------------------------------------------------------------------
// Streaming, permits, cancellation, timeouts
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_admission_permit_is_held_until_the_body_ends() {
    let (origin, _up) = spawn_upstream(Mode::Drip {
        every: Duration::from_millis(100),
        count: 4,
    })
    .await;
    let limits = Limits {
        per_device: 1,
        ..Limits::default()
    };
    let r = spawn_relay_with(&origin, limits, Timeouts::default(), None).await;
    let first = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(
        first.status(),
        200,
        "headers are back, body still streaming"
    );

    let second = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(
        error_code(second).await,
        (503, "relay_over_capacity".into())
    );

    let _ = first.bytes().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let third = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(third.status(), 200);
}

#[tokio::test]
async fn a_client_disconnect_cancels_the_upstream_request() {
    let (origin, up) = spawn_upstream(Mode::Hang).await;
    let r = spawn_relay(&origin).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    let mut body = resp.bytes_stream();
    let first = body.next().await.unwrap().unwrap();
    assert_eq!(first.as_ref(), b"data: first\n\n");
    drop(body);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !up.abandoned.load(Ordering::SeqCst) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "upstream still being read"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.outcomes.lock()[0].outcome.kind, OutcomeKind::Partial);
}

#[tokio::test]
async fn revoking_a_device_ends_its_in_flight_stream() {
    let (origin, up) = spawn_upstream(Mode::Hang).await;
    let r = spawn_relay(&origin).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    let mut body = resp.bytes_stream();
    let _ = body.next().await.unwrap().unwrap();
    r.devices.revoke(&r.device_id).unwrap();
    let rest = tokio::time::timeout(Duration::from_secs(5), body.next())
        .await
        .expect("the stream ended promptly");
    assert!(matches!(rest, Some(Err(_)) | None));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !up.abandoned.load(Ordering::SeqCst) {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// `relay revoke` from another shell, end to end: the request crosses the
/// revoke socket, and the stream it ends is the one in flight here.
#[cfg(unix)]
#[tokio::test]
async fn a_revoke_through_the_socket_ends_the_in_flight_stream() {
    use super::revoke_socket::{self, LiveRevoke};
    let (origin, up) = spawn_upstream(Mode::Hang).await;
    let r = spawn_relay(&origin).await;
    let dir = tempfile::tempdir().unwrap();
    let (live, task) = revoke_socket::arm(dir.path(), r.state.clone(), r.shutdown.0.clone());
    assert_eq!(live, LiveRevoke::Listening);

    let resp = r.post("/v1/messages").send().await.unwrap();
    let mut body = resp.bytes_stream();
    let _ = body.next().await.unwrap().unwrap();
    assert_eq!(
        revoke_socket::revoke(dir.path(), &r.device_id)
            .await
            .unwrap(),
        Some(Revocation::Revoked)
    );
    let rest = tokio::time::timeout(Duration::from_secs(5), body.next())
        .await
        .expect("the stream ended promptly");
    assert!(matches!(rest, Some(Err(_)) | None));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !up.abandoned.load(Ordering::SeqCst) {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let next = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(next.status(), 403, "the next request is refused");

    r.shutdown.0.cancel();
    task.unwrap().await.unwrap();
    assert!(!revoke_socket::socket_path(dir.path()).exists());
}

#[tokio::test]
async fn revoking_a_device_before_headers_ends_the_request_and_records_it() {
    let (origin, _up) = spawn_upstream(Mode::SlowHeaders(Duration::from_secs(30))).await;
    let r = spawn_relay(&origin).await;
    let pending = tokio::spawn(r.post("/v1/messages").send());
    tokio::time::sleep(Duration::from_millis(200)).await;
    r.devices.revoke(&r.device_id).unwrap();
    let resp = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("the request ended promptly")
        .unwrap()
        .unwrap();
    assert_eq!(resp.status(), 403);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let recs = r.outcomes.lock();
    assert_eq!(recs.len(), 1, "the cancelled request reaches the ledger");
    assert_eq!(recs[0].outcome.kind, OutcomeKind::Partial);
    assert_eq!(
        recs[0].outcome.error.as_deref(),
        Some("relay_unauthenticated")
    );
}

/// The >10-minute criterion, at scale: the idle timeout resets on every
/// chunk and there is no total deadline, so a stream many idle-windows
/// long completes, while one silent gap longer than the window ends it.
/// With the real values (10 min idle, nothing total) the same code
/// carries a stream of any length.
#[tokio::test]
async fn a_stream_far_longer_than_the_idle_window_completes() {
    let (origin, _up) = spawn_upstream(Mode::Drip {
        every: Duration::from_millis(60),
        count: 25,
    })
    .await;
    let timeouts = Timeouts {
        read_idle: Duration::from_millis(250),
        ..Timeouts::default()
    };
    let r = spawn_relay_with(&origin, Limits::default(), timeouts, None).await;
    let started = std::time::Instant::now();
    let resp = r.post("/v1/messages").send().await.unwrap();
    let got = resp.bytes().await.unwrap();
    assert!(started.elapsed() > Duration::from_millis(250) * 5);
    assert_eq!(got.len(), b"data: tick\n\n".len() * 25);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.outcomes.lock()[0].outcome.kind, OutcomeKind::Complete);
}

#[tokio::test]
async fn a_silent_gap_longer_than_the_idle_window_aborts_the_stream() {
    let (origin, _up) = spawn_upstream(Mode::Drip {
        every: Duration::from_millis(400),
        count: 3,
    })
    .await;
    let timeouts = Timeouts {
        read_idle: Duration::from_millis(150),
        ..Timeouts::default()
    };
    let r = spawn_relay_with(&origin, Limits::default(), timeouts, None).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    assert!(
        resp.bytes().await.is_err(),
        "the client sees an aborted body"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.outcomes.lock()[0].outcome.kind, OutcomeKind::Partial);
}

#[tokio::test]
async fn waiting_too_long_for_response_headers_is_a_504() {
    let (origin, _up) = spawn_upstream(Mode::SlowHeaders(Duration::from_millis(500))).await;
    let timeouts = Timeouts {
        response_header: Duration::from_millis(100),
        ..Timeouts::default()
    };
    let r = spawn_relay_with(&origin, Limits::default(), timeouts, None).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    assert_eq!(error_code(resp).await, (504, "upstream_timeout".into()));
}

#[test]
fn the_relay_client_sets_no_total_deadline() {
    // reqwest exposes no getter, so this pins the source instead: the
    // builder the relay client starts from must never grow `.timeout(`.
    let src = include_str!("forward.rs");
    let start = src.find("pub(crate) fn base_builder").unwrap();
    let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
    assert!(body.contains(".read_timeout("));
    assert!(!body.contains(".timeout("));
    let defaults = Timeouts::default();
    assert_eq!(defaults.read_idle, Duration::from_secs(600));
    assert_eq!(defaults.response_header, Duration::from_secs(120));
    assert_eq!(defaults.connect, Duration::from_secs(15));
}

// ---------------------------------------------------------------------
// Body limits
// ---------------------------------------------------------------------

#[tokio::test]
async fn an_oversized_body_is_413_and_never_dispatched() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let limits = Limits {
        max_body: 1024,
        ..Limits::default()
    };
    let r = spawn_relay_with(&origin, limits, Timeouts::default(), None).await;
    let declared = r
        .post("/v1/messages")
        .body(vec![b'x'; 2048])
        .send()
        .await
        .unwrap();
    assert_eq!(error_code(declared).await, (413, "body_too_large".into()));

    // Chunked, no Content-Length: caught while reading.
    let chunks = futures_util::stream::iter(
        (0..4).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![b'x'; 512]))),
    );
    let chunked = client()
        .post(r.url("/v1/messages"))
        .bearer_auth(&r.token)
        .body(reqwest::Body::wrap_stream(chunks))
        .send()
        .await
        .unwrap();
    assert_eq!(error_code(chunked).await, (413, "body_too_large".into()));
    assert_eq!(up.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_stalled_upload_is_abandoned() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let limits = Limits {
        body_idle: Duration::from_millis(150),
        ..Limits::default()
    };
    let r = spawn_relay_with(&origin, limits, Timeouts::default(), None).await;
    let mut s = TcpStream::connect(r.addr).await.unwrap();
    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: relay\r\nAuthorization: Bearer {}\r\nContent-Length: 100\r\n\r\n{{",
        r.token
    );
    s.write_all(head.as_bytes()).await.unwrap();
    let mut out = vec![0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(3), s.read(&mut out))
        .await
        .expect("the relay answered")
        .unwrap();
    assert!(String::from_utf8_lossy(&out[..n]).starts_with("HTTP/1.1 408"));
    assert_eq!(up.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_aggregate_body_budget_refuses_with_503() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let limits = Limits {
        max_body: 1000,
        body_budget: 1500,
        body_idle: Duration::from_secs(2),
        ..Limits::default()
    };
    let r = spawn_relay_with(&origin, limits, Timeouts::default(), None).await;
    // Hold 1000 bytes of budget with an upload that has not finished.
    let mut s = TcpStream::connect(r.addr).await.unwrap();
    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: relay\r\nAuthorization: Bearer {}\r\nContent-Length: 1000\r\n\r\n{{",
        r.token
    );
    s.write_all(head.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = r
        .post("/v1/messages")
        .body(vec![b'x'; 900])
        .send()
        .await
        .unwrap();
    assert_eq!(
        error_code(second).await,
        (503, "relay_over_capacity".into())
    );
    assert_eq!(up.hits.load(Ordering::SeqCst), 0);
}

/// A device revoked while its upload is still trickling in gives back its
/// slots now, not when the client finishes or the idle window expires —
/// otherwise a revoked client could hold global capacity by drip-feeding.
#[tokio::test]
async fn revoking_a_device_mid_upload_ends_it_and_frees_its_slot() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let limits = Limits {
        global: 1,
        body_idle: Duration::from_secs(30),
        ..Limits::default()
    };
    let r = spawn_relay_with(&origin, limits, Timeouts::default(), None).await;
    let mut s = TcpStream::connect(r.addr).await.unwrap();
    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: relay\r\nAuthorization: Bearer {}\r\nContent-Length: 100\r\n\r\n{{",
        r.token
    );
    s.write_all(head.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The one global slot is held by the stalled upload.
    let other = r.devices.issue("desktop", grants()).unwrap();
    let blocked = client()
        .post(r.url("/v1/messages"))
        .bearer_auth(other.plaintext.expose())
        .body(r#"{"model":"claude-sonnet-4-5","max_tokens":1}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(
        error_code(blocked).await,
        (503, "relay_over_capacity".into())
    );

    r.devices.revoke(&r.device_id).unwrap();
    let mut out = vec![0u8; 512];
    let n = tokio::time::timeout(Duration::from_secs(3), s.read(&mut out))
        .await
        .expect("the relay answered without waiting for the body")
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out[..n]).starts_with("HTTP/1.1 403"),
        "{}",
        String::from_utf8_lossy(&out[..n])
    );

    let freed = client()
        .post(r.url("/v1/messages"))
        .bearer_auth(other.plaintext.expose())
        .body(r#"{"model":"claude-sonnet-4-5","max_tokens":1}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(freed.status(), 200, "the revoked upload's slot came back");
    assert_eq!(up.hits.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------
// Shutdown: stop means no connection is left
// ---------------------------------------------------------------------

/// The bug this pins: stop ended the accept loop only, and a keep-alive
/// connection went on carrying billable requests after "stopped".
#[tokio::test]
async fn a_keep_alive_connection_carries_no_request_after_stop() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let mut r = spawn_relay(&origin).await;
    // One client, so the second request reuses the pooled connection.
    let c = client();
    let (url, token) = (r.url("/v1/messages"), r.token.clone());
    let send = |c: &reqwest::Client| {
        c.post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .body(r#"{"model":"claude-sonnet-4-5","max_tokens":1}"#)
            .send()
    };
    let first = send(&c).await.unwrap();
    assert_eq!(first.status(), 200);
    first.bytes().await.unwrap();
    assert_eq!(up.hits.load(Ordering::SeqCst), 1);

    tokio::time::timeout(Duration::from_secs(3), r.stop())
        .await
        .expect("an idle connection closes at once");

    let second = send(&c).await;
    assert!(second.is_err(), "{:?}", second.map(|r| r.status()));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        up.hits.load(Ordering::SeqCst),
        1,
        "no request reached the upstream after stop"
    );
}

/// An in-flight stream gets the drain bound and is then cut, which drops
/// the upstream request; `serve` returns only after that.
#[tokio::test]
async fn a_stream_in_flight_at_stop_is_cut_within_the_drain_bound() {
    let (origin, up) = spawn_upstream(Mode::Hang).await;
    let limits = Limits {
        shutdown_drain: Duration::from_millis(300),
        ..Limits::default()
    };
    let mut r = spawn_relay_with(&origin, limits, Timeouts::default(), None).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    let mut body = resp.bytes_stream();
    let _ = body.next().await.unwrap().unwrap();

    let started = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(3), r.stop())
        .await
        .expect("stop is bounded by the drain");
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "the stream was given its drain window"
    );
    // By the time stop returned, the response was already cut.
    let rest = tokio::time::timeout(Duration::from_millis(200), body.next())
        .await
        .expect("the stream had ended when stop returned");
    assert!(matches!(rest, Some(Err(_)) | None));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !up.abandoned.load(Ordering::SeqCst) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "upstream still read"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// An upload under way when stop is asked for is refused, never
/// dispatched once its body completes.
#[tokio::test]
async fn an_upload_under_way_at_stop_is_never_dispatched() {
    let (origin, up) = spawn_upstream(Mode::Sse).await;
    let mut r = spawn_relay(&origin).await;
    let mut s = TcpStream::connect(r.addr).await.unwrap();
    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nHost: relay\r\nAuthorization: Bearer {}\r\nContent-Length: 44\r\n\r\n{{",
        r.token
    );
    s.write_all(head.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let stopping = tokio::spawn(async move {
        r.stop().await;
        r
    });
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        let _ = s
            .write_all(br#""model":"claude-sonnet-4-5","max_tokens":1}"#)
            .await;
        let _ = s.read_to_end(&mut out).await;
    })
    .await
    .expect("the connection closed");
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.is_empty() || text.starts_with("HTTP/1.1 503"),
        "{text}"
    );
    let _r = tokio::time::timeout(Duration::from_secs(3), stopping)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(up.hits.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------
// Canary: no credential in any log line or error
// ---------------------------------------------------------------------

#[derive(Clone, Default)]
struct Capture(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Panics on dispatch — the "task panic" case.
struct PanicHandler {
    target: TargetIdentity,
    caps: Capabilities,
}

#[async_trait::async_trait]
impl UpstreamHandler for PanicHandler {
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
        panic!("handler panicked");
    }
}

const CANARY_CHILD: &str = "relay::tests::canary_scan_in_a_fresh_process";

/// In a child process. Run in-suite, a thread-local subscriber missed
/// events: tracing caches each callsite's interest process-wide, and a
/// callsite another test thread registered first could be cached as
/// uninterested — the capture shrank and the scan could pass blind.
#[test]
fn neither_canary_appears_in_any_log_line_or_error() {
    crate::testing::run_in_child(CANARY_CHILD, &[], &[]);
}

#[tokio::test(flavor = "current_thread")]
async fn canary_scan_in_a_fresh_process() {
    if !crate::testing::in_child(CANARY_CHILD) {
        return; // only meaningful inside the child spawned above
    }
    let cap = Capture::default();
    let writer = cap.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // Global and first: installed before any relay, reqwest or hyper
    // callsite registers in this process.
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let boom_id = UpstreamId::parse("boom").unwrap();
    let panicking: Arc<dyn UpstreamHandler> = Arc::new(PanicHandler {
        target: TargetIdentity {
            id: boom_id.clone(),
            fingerprint: "fp".into(),
            generation: 1,
            kind: crate::relay::identity::TargetKind::Subscription,
        },
        caps: Capabilities {
            operations: [super::upstream::Operation::Messages].into(),
        },
    });

    fn collect(out: &mut String, status: u16, headers: &reqwest::header::HeaderMap, body: &[u8]) {
        out.push_str(&format!("{status} {headers:?} "));
        out.push_str(&String::from_utf8_lossy(body));
    }
    let mut bodies = String::new();
    let mut device_tokens = Vec::new();
    let mut records: Vec<OutcomeRecord> = Vec::new();

    for mode in [
        Mode::RejectAll,
        Mode::Status(500),
        Mode::Redirect,
        Mode::Sse,
    ] {
        let (origin, _up) = spawn_upstream(mode).await;
        let r = spawn_relay_with(
            &origin,
            Limits::default(),
            Timeouts::default(),
            Some(panicking.clone()),
        )
        .await;
        r.devices.revoke(&r.device_id).unwrap();
        let fresh = r
            .devices
            .issue(
                "canary",
                vec![
                    grants()[0].clone(),
                    Grant {
                        upstream_id: boom_id.clone(),
                        accepted_generation: 1,
                        epoch: String::new(),
                    },
                ],
            )
            .unwrap();
        let device = fresh.plaintext.expose().to_owned();
        device_tokens.push(device.clone());
        device_tokens.push(r.token.clone());
        let c = client();
        for (path, token) in [
            ("/v1/messages", Some(device.as_str())),
            ("/u/sub/v1/messages", Some(device.as_str())),
            ("/u/boom/v1/messages", Some(device.as_str())),
            ("/u/nope/v1/messages", Some(device.as_str())),
            ("/v1/models", Some(device.as_str())),
            ("/v1/messages", Some(OAUTH)),
            ("/v1/messages", None),
        ] {
            let mut req = c.post(r.url(path)).body("{}");
            if let Some(t) = token {
                req = req.bearer_auth(t);
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let headers = resp.headers().clone();
                    let body = resp.bytes().await.unwrap_or_default();
                    collect(&mut bodies, status, &headers, &body);
                }
                Err(e) => bodies.push_str(&format!("{e:?}")),
            }
        }
        let (_, text) = raw(r.addr, "POST", "/u/a%2F../v1/messages", Some(&device)).await;
        bodies.push_str(&text);
        // The server survives the panic.
        let after = c.head(r.url("/api/hello")).send().await.unwrap();
        assert_eq!(after.status(), 200);
        tokio::time::sleep(Duration::from_millis(50)).await;
        records.append(&mut r.outcomes.lock());
    }
    // Transport failure.
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let r = spawn_relay(&origin).await;
    let resp = r.post("/v1/messages").send().await.unwrap();
    let headers = resp.headers().clone();
    let status = resp.status().as_u16();
    collect(&mut bodies, status, &headers, &resp.bytes().await.unwrap());
    device_tokens.push(r.token.clone());

    tokio::time::sleep(Duration::from_millis(100)).await;
    records.append(&mut r.outcomes.lock());
    // Every outcome goes through the real ledger writer, so what is
    // scanned is the file `relay-usage.jsonl` would hold.
    let dir = tempfile::tempdir().unwrap();
    let ledger = super::ledger::Ledger::new(
        dir.path().join("relay-usage.jsonl"),
        crate::pricing::PriceBook::bundled_only(),
    );
    for rec in &records {
        ledger.append(rec, chrono::Utc::now()).unwrap();
    }
    let usage_file = std::fs::read_to_string(ledger.path()).unwrap();
    assert!(
        usage_file.lines().count() >= 8,
        "the ledger scan saw real outcomes: {} lines",
        usage_file.lines().count()
    );
    let logs = String::from_utf8(cap.0.lock().unwrap().clone()).unwrap();
    assert!(
        logs.len() > 1000,
        "TRACE capture actually ran: {} bytes",
        logs.len()
    );
    let mut canaries = vec![OAUTH, "CANARYoauth"];
    for t in &device_tokens {
        canaries.push(t.as_str());
        canaries.push(&t[5..]);
    }
    for canary in canaries {
        assert!(!logs.contains(canary), "canary in logs");
        assert!(!bodies.contains(canary), "canary in a response");
        assert!(!usage_file.contains(canary), "canary in relay-usage.jsonl");
    }
    assert!(usage_file.contains("upstream_credential_rejected"));
    assert!(bodies.contains("upstream_credential_rejected"));
    assert!(bodies.contains("upstream_unreachable"));
}

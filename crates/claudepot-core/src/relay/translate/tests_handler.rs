//! [`TranslateHandler`] against a loopback Responses provider, called
//! directly: the credential it presents, what health reports after a
//! rejection, and the latency the outcome records.

use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde_json::json;
use tokio::net::TcpListener;

use super::super::super::cancel::CancelToken;
use super::super::super::forward::{base_builder, Timeouts};
use super::super::super::identity::TargetKind;
use super::super::super::route_secret::{KeychainReader, RouteKey, RouteSecretError};
use super::super::super::secret::Secret;
use super::super::super::target::UpstreamId;
use super::super::super::upstream::{
    AdmittedRequest, BufferedBody, Operation, TargetIdentity, UpstreamError, UpstreamHandler,
};
use super::super::continuation::{self, ContinuationStore};
use super::super::tests::route;
use super::super::{route_target_spec, Translation};
use super::*;
use crate::routes::{RouteProvider, RouteStore};

#[derive(Default)]
struct Provider {
    status: AtomicU16,
    /// Held before the first body byte, after the headers.
    first_byte_delay: Duration,
    /// Answer this many requests 401 before `status` applies.
    reject_first: AtomicUsize,
    /// After the stream, keep the connection open instead of ending it.
    hold_open: bool,
    /// Answer 401 to this `authorization` header.
    reject_auth: Mutex<Option<String>>,
    /// Held before the response headers of every request not refused.
    header_delay: Duration,
    hits: AtomicUsize,
    auth: Mutex<Vec<String>>,
    accounts: Mutex<Vec<String>>,
    bodies: Mutex<Vec<Bytes>>,
}

const STREAM: &[&str] = &[
    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r1\"}}\n\n",
    "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"m\",\"content_index\":0,\"delta\":\"hi\"}\n\n",
    "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"hi\"}]}],\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\n",
];

async fn provider_handler(
    State(p): State<Arc<Provider>>,
    headers: HeaderMap,
    request_body: Bytes,
) -> Response {
    p.hits.fetch_add(1, Ordering::SeqCst);
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned()
    };
    p.auth.lock().push(header("authorization"));
    p.accounts.lock().push(header("chatgpt-account-id"));
    p.bodies.lock().push(request_body);
    // A compare-exchange loop, not `fetch_update`: deprecated on the
    // current toolchain, and its replacement is newer than the MSRV.
    let rejecting = loop {
        let n = p.reject_first.load(Ordering::SeqCst);
        if n == 0 {
            break false;
        }
        if p.reject_first
            .compare_exchange(n, n - 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            break true;
        }
    };
    let refused = p.reject_auth.lock().as_deref() == Some(header("authorization").as_str());
    if !rejecting && !refused {
        tokio::time::sleep(p.header_delay).await;
    }
    let status = if rejecting || refused {
        401
    } else {
        p.status.load(Ordering::SeqCst)
    };
    if status != 200 {
        return (StatusCode::from_u16(status).unwrap(), "{}").into_response();
    }
    let delay = p.first_byte_delay;
    let chunks =
        futures_util::stream::iter(STREAM.iter().enumerate()).then(move |(i, chunk)| async move {
            if i == 0 {
                tokio::time::sleep(delay).await;
            }
            Ok::<_, std::io::Error>(Bytes::from_static(chunk.as_bytes()))
        });
    let body = if p.hold_open {
        // Heartbeats, forever, after the terminal event.
        chunks
            .chain(futures_util::stream::unfold((), |()| async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((Ok(Bytes::from_static(b": ping\n\n")), ()))
            }))
            .boxed()
    } else {
        chunks.boxed()
    };
    (
        [("content-type", "text/event-stream")],
        Body::from_stream(body),
    )
        .into_response()
}

async fn spawn_provider(status: u16, first_byte_delay: Duration) -> (String, Arc<Provider>) {
    spawn(Provider {
        status: AtomicU16::new(status),
        first_byte_delay,
        ..Provider::default()
    })
    .await
}

async fn spawn(p: Provider) -> (String, Arc<Provider>) {
    let p = Arc::new(p);
    let app = Router::new()
        .fallback(provider_handler)
        .with_state(p.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}/v1"), p)
}

/// A keychain the test can change.
struct SwitchKeychain(Mutex<Option<&'static str>>);
impl KeychainReader for SwitchKeychain {
    fn read(&self, _route: crate::routes::RouteId) -> Result<Option<Secret>, RouteSecretError> {
        Ok(self.0.lock().map(Secret::new))
    }
}

fn keychain_route(origin: &str) -> Route {
    let mut r = route(origin, "m");
    if let RouteProvider::Gateway(c) = &mut r.provider {
        c.use_keychain = true;
        c.api_key.clear();
    }
    r
}

fn handler(r: Route, credential: Credential) -> TranslateHandler {
    let t = Translation::Openai;
    let target = TargetIdentity {
        id: UpstreamId::parse("translated").unwrap(),
        fingerprint: route_target_spec(&r, t).unwrap().digest(),
        generation: 1,
        kind: TargetKind::Translated,
    };
    let timeouts = Timeouts::default();
    let client = base_builder(&timeouts).no_proxy().build().unwrap();
    let store = Arc::new(ContinuationStore::new(continuation::Limits::default()));
    TranslateHandler::with_client(target, r, t, client, timeouts, credential, store)
}

fn request(stream: bool) -> AdmittedRequest {
    let body = json!({"model": "claude-sonnet-4-5", "max_tokens": 8, "stream": stream,
                      "messages": [{"role": "user", "content": "hi"}]});
    AdmittedRequest {
        operation: Operation::Messages,
        device_id: "laptop".into(),
        beta: false,
        headers: HeaderMap::new(),
        body: BufferedBody::unbudgeted(Bytes::from(body.to_string())),
    }
}

/// Send one request and drain its body; the outcome on success.
async fn send(
    h: &TranslateHandler,
    stream: bool,
) -> Result<super::super::super::upstream::Outcome, UpstreamError> {
    let (resp, outcome) = h.handle(request(stream), CancelToken::new()).await?;
    let _: Vec<_> = resp.body.collect().await;
    Ok(outcome.await.unwrap())
}

async fn status_until(h: &TranslateHandler, ready: bool) -> UpstreamStatus {
    for _ in 0..200 {
        let s = h.status().await;
        if s.credential_ready == ready {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    h.status().await
}

#[tokio::test]
async fn a_translated_plaintext_key_rotated_in_routes_json_is_used_without_a_restart() {
    let (origin, p) = spawn_provider(200, Duration::ZERO).await;
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("routes.json");
    let mut store = RouteStore::open_at(file.clone()).unwrap();
    let approved = store.add(route(&origin, "m")).unwrap();
    let key = RouteKey::new(approved.clone(), Arc::new(SwitchKeychain(Mutex::new(None))))
        .with_routes_file(file);
    let h = handler(approved.clone(), Credential::Route(key));

    send(&h, true).await.unwrap();
    let mut rotated = approved.clone();
    if let RouteProvider::Gateway(c) = &mut rotated.provider {
        c.api_key = "sk-translated-ROTATED".into();
    }
    store.update(rotated).unwrap();
    send(&h, true).await.unwrap();

    assert_eq!(
        *p.auth.lock(),
        [
            "Bearer sk-translated-CANARY",
            "Bearer sk-translated-ROTATED"
        ]
    );
}

#[tokio::test]
async fn a_provider_redirect_is_refused_not_followed() {
    let (origin, p) = spawn_provider(302, Duration::ZERO).await;
    let h = handler(
        route(&origin, "m"),
        Credential::RouteKey(Arc::new(SwitchKeychain(Mutex::new(None)))),
    );
    assert_eq!(send(&h, false).await.unwrap_err(), UpstreamError::Redirect);
    assert_eq!(p.hits.load(Ordering::SeqCst), 1, "sent once, not followed");
}

#[tokio::test]
async fn a_rejected_route_key_is_reported_by_health_until_it_is_repaired() {
    let (origin, p) = spawn_provider(401, Duration::ZERO).await;
    let kc = Arc::new(SwitchKeychain(Mutex::new(Some("sk-revoked"))));
    let r = keychain_route(&origin);
    let mut h = handler(r, Credential::RouteKey(kc.clone()));
    h.health = Arc::new(CredentialHealth::new(
        Duration::ZERO,
        super::super::super::passthrough::REJECTION_HOLD,
    ));

    assert_eq!(
        send(&h, true).await.unwrap_err(),
        UpstreamError::CredentialRejected
    );
    let s = h.status().await;
    assert!(!s.credential_ready);
    assert_eq!(s.credential_error, Some(UpstreamError::CredentialRejected));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !h.status().await.credential_ready,
        "re-reading the same key keeps it rejected"
    );

    *kc.0.lock() = Some("sk-repaired");
    assert!(status_until(&h, true).await.credential_ready);
    assert_eq!(p.hits.load(Ordering::SeqCst), 1, "recovery sent no request");

    p.status.store(200, Ordering::SeqCst);
    send(&h, true).await.unwrap();
    assert!(h.status().await.credential_ready);
    assert_eq!(p.auth.lock().last().unwrap(), "Bearer sk-repaired");
}

#[tokio::test]
async fn an_older_acceptance_does_not_clear_a_newer_rejection_of_another_key() {
    let (origin, p) = spawn(Provider {
        status: AtomicU16::new(200),
        reject_auth: Mutex::new(Some("Bearer sk-new".into())),
        header_delay: Duration::from_millis(300),
        ..Provider::default()
    })
    .await;
    let kc = Arc::new(SwitchKeychain(Mutex::new(Some("sk-old"))));
    let mut h = handler(keychain_route(&origin), Credential::RouteKey(kc.clone()));
    h.health = Arc::new(CredentialHealth::new(
        Duration::ZERO,
        super::super::super::passthrough::REJECTION_HOLD,
    ));
    let h = Arc::new(h);

    // Request A reads the old key; its answer is slow.
    let older = tokio::spawn({
        let h = h.clone();
        async move { send(&h, true).await.map(|_| ()) }
    });
    while p.hits.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    // The key changes, and the new one is rejected while A is in flight.
    *kc.0.lock() = Some("sk-new");
    assert_eq!(
        send(&h, true).await.unwrap_err(),
        UpstreamError::CredentialRejected
    );
    // A's success arrives last; it speaks for the old key only.
    older.await.unwrap().unwrap();

    let s = h.status().await;
    assert!(!s.credential_ready, "the current key is still rejected");
    assert_eq!(s.credential_error, Some(UpstreamError::CredentialRejected));
}

#[tokio::test]
async fn a_collected_response_reports_the_providers_first_byte_latency() {
    let delay = Duration::from_millis(300);
    let (origin, _p) = spawn_provider(200, delay).await;
    let h = handler(
        route(&origin, "m"),
        Credential::RouteKey(Arc::new(SwitchKeychain(Mutex::new(None)))),
    );
    let outcome = send(&h, false).await.unwrap();
    let ttfb = outcome.ttfb.unwrap();
    assert!(
        ttfb >= delay - Duration::from_millis(20),
        "the provider took {delay:?} to its first byte; recorded {ttfb:?}"
    );
    assert!(ttfb < Duration::from_secs(5), "{ttfb:?}");
}

#[tokio::test]
async fn a_collected_response_returns_at_the_terminal_event_on_a_connection_left_open() {
    let (origin, _p) = spawn(Provider {
        status: AtomicU16::new(200),
        hold_open: true,
        ..Provider::default()
    })
    .await;
    let h = handler(
        route(&origin, "m"),
        Credential::RouteKey(Arc::new(SwitchKeychain(Mutex::new(None)))),
    );
    let (resp, _outcome) = tokio::time::timeout(
        Duration::from_secs(5),
        h.handle(request(false), CancelToken::new()),
    )
    .await
    .expect("collection stops at the terminal event, not at EOF")
    .unwrap();
    assert_eq!(resp.status, StatusCode::OK);
    let body: Vec<_> = tokio::time::timeout(Duration::from_secs(5), resp.body.collect())
        .await
        .unwrap();
    let body: Vec<u8> = body.into_iter().flat_map(|c| c.unwrap().to_vec()).collect();
    let message: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(message["stop_reason"], "end_turn");
    assert_eq!(message["content"][0]["text"], "hi");
}

#[cfg(unix)]
mod codex_renewal {
    use super::super::super::codex::tests::{calls, fake_codex, jwt};
    use super::super::super::codex::{ensure_home, CodexAuth};
    use super::*;

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        home: std::path::PathBuf,
        first: String,
        second: String,
        p: Arc<Provider>,
        h: TranslateHandler,
    }

    async fn fixture(p: Provider) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let first = jwt("acct_1", now() + 3600);
        let second = jwt("acct_1", now() + 7200);
        let bin = fake_codex(dir.path(), &first, &second);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let (origin, p) = spawn(p).await;
        let auth = Arc::new(CodexAuth::new(bin, &home));
        let h = handler(route(&origin, "m"), Credential::Codex(auth));
        Fixture {
            _dir: dir,
            home,
            first,
            second,
            p,
            h,
        }
    }

    #[tokio::test]
    async fn a_rejected_codex_token_is_renewed_once_and_the_same_body_replayed() {
        let f = fixture(Provider {
            status: AtomicU16::new(200),
            reject_first: AtomicUsize::new(1),
            ..Provider::default()
        })
        .await;
        send(&f.h, true).await.unwrap();
        assert_eq!(
            *f.p.auth.lock(),
            [
                format!("Bearer {}", f.first),
                format!("Bearer {}", f.second)
            ]
        );
        assert_eq!(*f.p.accounts.lock(), ["acct_1", "acct_1"]);
        let bodies = f.p.bodies.lock().clone();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0], bodies[1], "the retry replays the same body");
        assert_eq!(calls(&f.home), ["read", "refresh"]);
        assert!(f.h.status().await.credential_ready);
    }

    #[tokio::test]
    async fn a_renewed_codex_token_rejected_again_is_refused_without_a_third_try() {
        let f = fixture(Provider {
            status: AtomicU16::new(401),
            ..Provider::default()
        })
        .await;
        assert_eq!(
            send(&f.h, true).await.unwrap_err(),
            UpstreamError::CredentialRejected
        );
        assert_eq!(f.p.hits.load(Ordering::SeqCst), 2);
        assert_eq!(
            *f.p.auth.lock(),
            [
                format!("Bearer {}", f.first),
                format!("Bearer {}", f.second)
            ]
        );
        assert_eq!(calls(&f.home), ["read", "refresh"]);
    }
}

#[test]
fn a_provider_event_before_a_framing_fault_in_one_chunk_is_kept() {
    // One network read: a complete event, then bytes that are not UTF-8.
    // The event goes out in the same step as the fault, not lost behind it.
    let mut hops = super::Hops {
        lines: super::SseLines::default(),
        chat: None,
        store: Arc::new(ContinuationStore::new(continuation::Limits::default())),
        key: continuation::Key {
            device_id: "d".into(),
            fingerprint: "f".into(),
            model: "m".into(),
            call_id: String::new(),
        },
    };
    let mut chunk = b"data: {\"type\":\"response.created\"}\n\n".to_vec();
    chunk.extend_from_slice(b"data: \xff\xfe\n\n");
    let (events, fault) = hops.feed(&chunk);
    assert_eq!(events, [json!({"type": "response.created"})]);
    assert_eq!(fault, Some(super::super::stream::Fault::Malformed));
}

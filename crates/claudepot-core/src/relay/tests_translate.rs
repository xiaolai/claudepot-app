//! Translated-upstream suite (plan P10): a real relay listener in front of
//! a loopback provider speaking Responses or Chat Completions, so what is
//! asserted is what crosses a socket.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

use super::cancel::CancelToken;
use super::devices::{DeviceRegistry, Grant};
use super::forward::{base_builder, SubscriptionHandler, Timeouts};
use super::identity::TargetKind;
use super::lease::mock::MockBackend;
use super::lease::LeaseManager;
use super::ledger::LedgerLine;
use super::route_secret::{KeychainReader, RouteSecretError};
use super::secret::Secret;
use super::server::{self, Limits, OutcomeRecord, RelayState};
use super::target::UpstreamId;
use super::translate::continuation::{self, ContinuationStore};
use super::translate::handler::{Credential, TranslateHandler};
use super::translate::{route_target_spec, Translation, MAX_ARGS_PER_CALL, MAX_RETAINED_TEXT};
use super::upstream::{OutcomeKind, ProviderUsage, TargetIdentity};
use crate::pricing::PriceBook;
use crate::routes::{AuthScheme, GatewayConfig, Route, RouteProvider};

const ROUTE_KEY: &str = "sk-translated-CANARYkey";
const LEAK: &str = "UPSTREAM-LEAK-CANARY";

// ---------------------------------------------------------------------
// The mock provider
// ---------------------------------------------------------------------

#[derive(Clone)]
enum Mode {
    /// The given chunks, then end of stream.
    Replay(Vec<Bytes>),
    /// A status with a body that must never reach the client.
    Status(u16),
    /// Responses text deltas of one byte each, forever.
    EndlessResponses,
    /// One Chat tool call whose arguments grow by `frag` bytes, forever.
    EndlessChatArgs { frag: usize },
    /// Chat text deltas of `frag` bytes, forever.
    EndlessChatText { frag: usize },
}

struct Seen {
    path: String,
    headers: HeaderMap,
    body: Value,
}

struct Provider {
    mode: Mutex<Mode>,
    hits: AtomicUsize,
    seen: Mutex<Vec<Seen>>,
    /// Set when an endless body is dropped: the relay let go of it.
    dropped: Arc<AtomicBool>,
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn data(v: Value) -> String {
    format!("data: {v}\n\n")
}

fn endless(
    flag: Arc<AtomicBool>,
    first: String,
    next: impl Fn(u64) -> String + Send + 'static,
) -> Body {
    let s = futures_util::stream::unfold((DropFlag(flag), 0u64), move |(g, n)| {
        let chunk = if n == 0 { first.clone() } else { next(n) };
        async move {
            tokio::task::yield_now().await;
            Some((Ok::<_, std::io::Error>(Bytes::from(chunk)), (g, n + 1)))
        }
    });
    Body::from_stream(s)
}

async fn provider_handler(
    State(p): State<Arc<Provider>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    p.hits.fetch_add(1, Ordering::SeqCst);
    p.seen.lock().push(Seen {
        path: uri.path().to_owned(),
        headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    let mode = p.mode.lock().clone();
    let sse = [("content-type", "text/event-stream")];
    match mode {
        Mode::Replay(chunks) => {
            let s = futures_util::stream::iter(chunks.into_iter().map(Ok::<_, std::io::Error>));
            (sse, Body::from_stream(s)).into_response()
        }
        Mode::Status(code) => (
            StatusCode::from_u16(code).unwrap(),
            [("retry-after", "7")],
            format!(r#"{{"error":{{"message":"{LEAK} {ROUTE_KEY}"}}}}"#),
        )
            .into_response(),
        Mode::EndlessResponses => {
            let first = data(json!({"type": "response.created", "response": {"id": "r1"}}))
                + &data(
                    json!({"type": "response.content_part.added", "item_id": "m1",
                    "content_index": 0, "part": {"type": "output_text", "text": ""}}),
                );
            let body = endless(p.dropped.clone(), first, |_| {
                data(
                    json!({"type": "response.output_text.delta", "item_id": "m1",
                    "content_index": 0, "delta": "x"}),
                )
            });
            (sse, body).into_response()
        }
        Mode::EndlessChatArgs { frag } => {
            let first = data(
                json!({"id": "c1", "choices": [{"index": 0, "delta": {"tool_calls": [
                {"index": 0, "id": "call_a", "type": "function",
                 "function": {"name": "Bash", "arguments": ""}}]}}]}),
            );
            let body = endless(p.dropped.clone(), first, move |_| {
                data(
                    json!({"id": "c1", "choices": [{"index": 0, "delta": {"tool_calls": [
                    {"index": 0, "function": {"arguments": "a".repeat(frag)}}]}}]}),
                )
            });
            (sse, body).into_response()
        }
        Mode::EndlessChatText { frag } => {
            let chunk = move |_| {
                data(json!({"id": "c1", "choices": [{"index": 0,
                    "delta": {"content": "t".repeat(frag)}}]}))
            };
            let body = endless(p.dropped.clone(), chunk(0), chunk);
            (sse, body).into_response()
        }
    }
}

async fn spawn_provider(mode: Mode) -> (String, Arc<Provider>) {
    let p = Arc::new(Provider {
        mode: Mutex::new(mode),
        hits: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
        dropped: Arc::new(AtomicBool::new(false)),
    });
    let app = Router::new()
        .fallback(provider_handler)
        .with_state(p.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}/v1"), p)
}

/// A loopback "Anthropic" for the subscription upstream beside it.
async fn spawn_anthropic() -> String {
    let sse = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let app = Router::new().fallback(move || async move {
        ([("content-type", "text/event-stream")], sse).into_response()
    });
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

// ---------------------------------------------------------------------
// Provider scripts
// ---------------------------------------------------------------------

fn responses_text() -> Vec<Bytes> {
    [
        data(json!({"type": "response.created", "response": {"id": "resp_1"}})),
        data(
            json!({"type": "response.content_part.added", "item_id": "m1", "content_index": 0,
            "part": {"type": "output_text", "text": ""}}),
        ),
        data(
            json!({"type": "response.output_text.delta", "item_id": "m1", "content_index": 0,
            "delta": "hi"}),
        ),
        data(
            json!({"type": "response.content_part.done", "item_id": "m1", "content_index": 0,
            "part": {"type": "output_text", "text": "hi"}}),
        ),
        data(
            json!({"type": "response.completed", "response": {"id": "resp_1",
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}],
            "usage": {"input_tokens": 3, "output_tokens": 1}}}),
        ),
    ]
    .into_iter()
    .map(Bytes::from)
    .collect()
}

/// Gemini answering with two parallel calls, each with its own signature.
fn gemini_two_calls() -> Vec<Bytes> {
    let call = |i: u64, id: &str, cmd: &str, sig: &str| {
        json!({"index": i, "id": id, "type": "function",
            "function": {"name": "Bash", "arguments": json!({"command": cmd}).to_string()},
            "extra_content": {"google": {"thought_signature": sig}}})
    };
    [
        data(json!({"id": "g1", "choices": [{"index": 0, "delta": {"role": "assistant",
            "tool_calls": [call(0, "call_a", "ls", "sig-a"), call(1, "call_b", "pwd", "sig-b")]}}]})),
        data(json!({"id": "g1", "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 7}})),
        "data: [DONE]\n\n".to_owned(),
    ]
    .into_iter()
    .map(Bytes::from)
    .collect()
}

fn chat_text(text: &str) -> Vec<Bytes> {
    [
        data(json!({"id": "c1", "choices": [{"index": 0, "delta": {"content": text}}]})),
        data(
            json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 2, "completion_tokens": 1}}),
        ),
        "data: [DONE]\n\n".to_owned(),
    ]
    .into_iter()
    .map(Bytes::from)
    .collect()
}

// ---------------------------------------------------------------------
// The relay under test
// ---------------------------------------------------------------------

struct NoKeychain;
impl KeychainReader for NoKeychain {
    fn read(&self, _: crate::routes::RouteId) -> Result<Option<Secret>, RouteSecretError> {
        Ok(None)
    }
}

fn route(base_url: &str) -> Route {
    Route {
        id: Uuid::from_u128(0x7b),
        name: "Translated".into(),
        provider: RouteProvider::Gateway(GatewayConfig {
            base_url: base_url.into(),
            api_key: ROUTE_KEY.into(),
            auth_scheme: AuthScheme::Bearer,
            enable_tool_search: false,
            use_keychain: false,
        }),
        model: "provider-big".into(),
        small_fast_model: Some("provider-small".into()),
        additional_models: vec![],
        wrapper_name: "claude-translated".into(),
        deployment_organization_uuid: Uuid::nil(),
        active_on_desktop: false,
        installed_on_cli: true,
        is_private_cloud: false,
        capabilities_override: None,
    }
}

fn tr_id() -> UpstreamId {
    UpstreamId::parse("translated").unwrap()
}

fn sub_id() -> UpstreamId {
    UpstreamId::parse("sub").unwrap()
}

fn handler(origin: &str, t: Translation, store: Arc<ContinuationStore>) -> TranslateHandler {
    let r = route(origin);
    let target = TargetIdentity {
        id: tr_id(),
        fingerprint: route_target_spec(&r, t).unwrap().digest(),
        generation: 1,
        kind: TargetKind::Translated,
    };
    let timeouts = Timeouts::default();
    let client = base_builder(&timeouts).no_proxy().build().unwrap();
    TranslateHandler::with_client(
        target,
        r,
        t,
        client,
        timeouts,
        Credential::RouteKey(Arc::new(NoKeychain)),
        store,
    )
}

fn store() -> Arc<ContinuationStore> {
    Arc::new(ContinuationStore::new(continuation::Limits::default()))
}

fn grants() -> Vec<Grant> {
    [tr_id(), sub_id()]
        .into_iter()
        .map(|upstream_id| Grant {
            upstream_id,
            accepted_generation: 1,
            epoch: String::new(),
        })
        .collect()
}

struct Relay {
    addr: std::net::SocketAddr,
    devices: DeviceRegistry,
    token: String,
    shutdown: CancelToken,
    records: Arc<Mutex<Vec<OutcomeRecord>>>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn spawn_relay(h: TranslateHandler) -> Relay {
    let timeouts = Timeouts::default();
    let lease = Arc::new(LeaseManager::new(MockBackend::new(
        "sk-ant-oat01-CANARY",
        3600,
    )));
    let client = base_builder(&timeouts).no_proxy().build().unwrap();
    let sub_target = TargetIdentity {
        id: sub_id(),
        fingerprint: "fp-sub".into(),
        generation: 1,
        kind: TargetKind::Subscription,
    };
    let sub = SubscriptionHandler::with_client(
        sub_target,
        lease,
        client,
        spawn_anthropic().await,
        timeouts,
    );
    let devices = DeviceRegistry::new();
    let issued = devices.issue("laptop", grants()).unwrap();
    let limits = Limits::default();
    let records: Arc<Mutex<Vec<OutcomeRecord>>> = Arc::default();
    let sink = records.clone();
    let state = RelayState::new(devices.clone(), limits)
        .with_upstream(Arc::new(sub), true)
        .with_upstream(Arc::new(h), false)
        .with_sink(Arc::new(move |r| sink.lock().push(r)));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let shutdown = CancelToken::new();
    tokio::spawn(server::serve(
        l,
        server::router(Arc::new(state)),
        None,
        limits,
        shutdown.clone(),
    ));
    Relay {
        addr,
        devices,
        token: issued.plaintext.expose().to_owned(),
        shutdown,
        records,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

impl Relay {
    async fn send_as(&self, token: &str, path: &str, body: &Value) -> reqwest::Response {
        client()
            .post(format!("http://{}{path}", self.addr))
            .bearer_auth(token)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap()
    }

    async fn send(&self, body: &Value) -> reqwest::Response {
        self.send_as(&self.token, "/u/translated/v1/messages", body)
            .await
    }

    /// The one outcome recorded so far; it lands after the body ends.
    async fn record(&self) -> OutcomeRecord {
        for _ in 0..200 {
            if let Some(r) = self.records.lock().first() {
                return r.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no outcome recorded");
    }

    fn second_device(&self) -> String {
        let issued = self.devices.issue("phone", grants()).unwrap();
        issued.plaintext.expose().to_owned()
    }
}

async fn error_code(resp: reqwest::Response) -> (u16, String) {
    let status = resp.status().as_u16();
    let v: Value = resp.json().await.unwrap();
    (status, v["error"]["message"].as_str().unwrap().to_owned())
}

/// The `data:` payloads of an Anthropic SSE body.
fn events(body: &str) -> Vec<Value> {
    body.lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|d| serde_json::from_str(d).unwrap())
        .collect()
}

fn ended(body: &str) -> bool {
    events(body)
        .last()
        .is_some_and(|e| matches!(e["type"].as_str(), Some("message_stop" | "error")))
}

fn hello(stream: bool) -> Value {
    json!({"model": "claude-sonnet-4-5", "max_tokens": 64, "stream": stream,
        "messages": [{"role": "user", "content": "hi"}]})
}

const BASH: &str = r#"{"name":"Bash","description":"run","input_schema":{"type":"object","properties":{"command":{"type":"string"}}}}"#;

fn tools() -> Value {
    json!([serde_json::from_str::<Value>(BASH).unwrap()])
}

/// The second turn: both calls replayed with their results.
fn gemini_turn_two() -> Value {
    json!({"model": "claude-sonnet-4-5", "max_tokens": 64, "stream": true, "tools": tools(),
        "messages": [
            {"role": "user", "content": "look around"},
            {"role": "assistant", "content": [
                {"type": "tool_use", "id": "call_a", "name": "Bash", "input": {"command": "ls"}},
                {"type": "tool_use", "id": "call_b", "name": "Bash", "input": {"command": "pwd"}}]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_a", "content": "a.txt"},
                {"type": "tool_result", "tool_use_id": "call_b", "content": "/tmp"}]}]})
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_responses_stream_arrives_as_anthropic_events_on_the_route_key() {
    let (origin, p) = spawn_provider(Mode::Replay(responses_text())).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay.send(&hello(true)).await;
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    let ev = events(&body);
    let kinds: Vec<_> = ev.iter().filter_map(|e| e["type"].as_str()).collect();
    assert_eq!(kinds.first(), Some(&"message_start"));
    assert_eq!(kinds.last(), Some(&"message_stop"));
    assert!(body.contains("\"hi\""), "{body}");
    assert_eq!(ev[0]["message"]["model"], "provider-big");

    let seen = p.seen.lock();
    assert_eq!(seen[0].path, "/v1/responses");
    assert_eq!(seen[0].body["model"], "provider-big");
    assert_eq!(
        seen[0].headers["authorization"].to_str().unwrap(),
        format!("Bearer {ROUTE_KEY}")
    );
}

#[tokio::test]
async fn a_non_streaming_request_gets_one_collected_message() {
    let (origin, _p) = spawn_provider(Mode::Replay(responses_text())).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay.send(&hello(false)).await;
    assert_eq!(resp.status(), 200);
    let m: Value = resp.json().await.unwrap();
    assert_eq!(m["type"], "message");
    assert_eq!(m["stop_reason"], "end_turn");
    assert_eq!(m["content"][0]["text"], "hi");
    assert_eq!(m["usage"]["input_tokens"], 3);
}

#[tokio::test]
async fn count_tokens_is_refused_with_its_own_code_and_never_dispatched() {
    let (origin, p) = spawn_provider(Mode::Replay(responses_text())).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay
        .send_as(
            &relay.token,
            "/u/translated/v1/messages/count_tokens",
            &hello(false),
        )
        .await;
    assert_eq!(
        error_code(resp).await,
        (400, "count_tokens_unsupported".into())
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn gemini_parallel_calls_replay_their_signatures_and_fail_defined_without_them() {
    let (origin, p) = spawn_provider(Mode::Replay(gemini_two_calls())).await;
    let shared = store();
    let relay = spawn_relay(handler(&origin, Translation::Gemini, shared.clone())).await;

    // Turn one: two calls, each carrying its own signature.
    let mut first = hello(true);
    first["tools"] = tools();
    let body = relay.send(&first).await.text().await.unwrap();
    let ids: Vec<_> = events(&body)
        .iter()
        .filter(|e| e["content_block"]["type"] == "tool_use")
        .map(|e| e["content_block"]["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, ["call_a", "call_b"], "{body}");

    // Turn two on the same device: each call goes back with its own.
    *p.mode.lock() = Mode::Replay(chat_text("done"));
    let resp = relay.send(&gemini_turn_two()).await;
    assert_eq!(resp.status(), 200);
    assert!(ended(&resp.text().await.unwrap()));
    {
        let seen = p.seen.lock();
        assert_eq!(seen[1].path, "/v1/chat/completions");
        let calls: Vec<_> = seen[1].body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["tool_calls"].as_array().cloned().unwrap_or_default())
            .map(|c| {
                (
                    c["id"].as_str().unwrap().to_owned(),
                    c["extra_content"]["google"]["thought_signature"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                )
            })
            .collect();
        assert_eq!(
            calls,
            [
                ("call_a".into(), "sig-a".into()),
                ("call_b".into(), "sig-b".into())
            ]
        );
    }
    let hits = p.hits.load(Ordering::SeqCst);

    // Another device holds no signature for these calls.
    let other = relay.second_device();
    let resp = relay
        .send_as(&other, "/u/translated/v1/messages", &gemini_turn_two())
        .await;
    assert_eq!(
        error_code(resp).await,
        (400, "continuation_state_lost".into())
    );

    // A restarted relay starts with an empty store.
    drop(relay);
    let restarted = spawn_relay(handler(&origin, Translation::Gemini, store())).await;
    let resp = restarted.send(&gemini_turn_two()).await;
    assert_eq!(
        error_code(resp).await,
        (400, "continuation_state_lost".into())
    );
    assert_eq!(
        p.hits.load(Ordering::SeqCst),
        hits,
        "neither refusal was dispatched"
    );
    drop(shared);
}

#[tokio::test]
async fn provider_errors_are_restated_without_provider_text() {
    for (code, want) in [(429u16, 429u16), (500, 500), (400, 400)] {
        let (origin, _p) = spawn_provider(Mode::Status(code)).await;
        let relay = spawn_relay(handler(&origin, Translation::Grok, store())).await;
        let resp = relay.send(&hello(true)).await;
        assert_eq!(resp.status().as_u16(), want);
        let retry = resp.headers().get("retry-after").cloned();
        let text = resp.text().await.unwrap();
        assert!(!text.contains(LEAK) && !text.contains(ROUTE_KEY), "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["type"], "error");
        assert!(v["error"]["type"].is_string());
        if code == 429 {
            assert_eq!(retry.unwrap(), "7");
        }
    }
    let (origin, _p) = spawn_provider(Mode::Status(401)).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay.send(&hello(true)).await;
    assert_eq!(
        error_code(resp).await,
        (403, "upstream_credential_rejected".into())
    );
}

/// The isolation gate: a provider that never stops costs its own request a
/// bounded amount and nothing else.
#[tokio::test]
async fn an_endless_provider_is_capped_and_leaves_the_subscription_alone() {
    // Streaming: one call's arguments in tiny fragments hit the per-call
    // cap, and the stream ends with an error event.
    let frag = 256;
    let (origin, p) = spawn_provider(Mode::EndlessChatArgs { frag }).await;
    let relay = spawn_relay(handler(&origin, Translation::Chat, store())).await;
    let resp = relay.send(&hello(true)).await;
    assert_eq!(resp.status(), 200);
    let body = tokio::time::timeout(Duration::from_secs(30), resp.text())
        .await
        .expect("the cap ended the stream")
        .unwrap();
    assert_eq!(events(&body).last().unwrap()["type"], "error");
    assert!(body.contains("exceeded the relay's limits"), "{body}");
    assert!(
        body.len() < MAX_ARGS_PER_CALL,
        "arguments are never forwarded raw"
    );
    wait_for(&p.dropped).await;

    // Collected: retained text hits its cap, and the request is a 502.
    let (origin, p) = spawn_provider(Mode::EndlessChatText { frag: 1024 }).await;
    let relay = spawn_relay(handler(&origin, Translation::Chat, store())).await;
    let resp = tokio::time::timeout(Duration::from_secs(60), relay.send(&hello(false)))
        .await
        .expect("the cap ended the request");
    assert_eq!(resp.status(), 502);
    let text = resp.text().await.unwrap();
    assert!(text.len() < MAX_RETAINED_TEXT / 1024, "{}", text.len());
    wait_for(&p.dropped).await;

    // While a translated stream runs forever, the subscription still serves.
    let (origin, _p) = spawn_provider(Mode::EndlessResponses).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay.send(&hello(true)).await;
    let mut stream = resp.bytes_stream();
    let _ = stream.next().await.unwrap().unwrap();
    let sub = relay
        .send_as(&relay.token, "/u/sub/v1/messages", &hello(true))
        .await;
    assert_eq!(sub.status(), 200);
    assert!(sub.text().await.unwrap().contains("message_stop"));
    let _ = stream.next().await.unwrap().unwrap();
}

#[tokio::test]
async fn dropping_the_client_drops_the_provider_stream() {
    let (origin, p) = spawn_provider(Mode::EndlessResponses).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay.send(&hello(true)).await;
    let mut stream = resp.bytes_stream();
    for _ in 0..3 {
        let _ = stream.next().await.unwrap().unwrap();
    }
    assert!(!p.dropped.load(Ordering::SeqCst));
    drop(stream);
    wait_for(&p.dropped).await;
}

#[tokio::test]
async fn revoking_the_device_ends_a_translated_stream() {
    let (origin, p) = spawn_provider(Mode::EndlessResponses).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay.send(&hello(true)).await;
    let mut stream = resp.bytes_stream();
    let _ = stream.next().await.unwrap().unwrap();
    let id = relay.devices.list()[0].id.clone();
    relay.devices.revoke(&id).unwrap();
    let rest = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(_)) = stream.next().await {}
    })
    .await;
    assert!(rest.is_ok(), "the stream ended after revocation");
    wait_for(&p.dropped).await;
}

async fn wait_for(flag: &AtomicBool) {
    for _ in 0..500 {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the provider stream was never dropped");
}

// ---------------------------------------------------------------------
// Native usage (plan §4.4): the provider's own counters reach the ledger
// ---------------------------------------------------------------------

/// A Responses turn whose usage carries cached and reasoning counts.
fn responses_with_usage() -> Vec<Bytes> {
    [
        data(json!({"type": "response.created", "response": {"id": "resp_u"}})),
        data(
            json!({"type": "response.completed", "response": {"id": "resp_u",
            "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": "ok"}]}],
            "usage": {"input_tokens": 10, "output_tokens": 6,
                "input_tokens_details": {"cached_tokens": 4},
                "output_tokens_details": {"reasoning_tokens": 5}}}}),
        ),
    ]
    .into_iter()
    .map(Bytes::from)
    .collect()
}

fn ledger(rec: &OutcomeRecord) -> LedgerLine {
    LedgerLine::from_record(rec, chrono::Utc::now(), &PriceBook::bundled_only())
}

/// Provider semantics stay native (input includes cached); the
/// normalized side is what the client saw (cached split out).
fn assert_native_beside_normalized(rec: &OutcomeRecord) {
    assert_eq!(rec.outcome.kind, OutcomeKind::Complete);
    assert_eq!(
        rec.outcome.native_usage,
        Some(ProviderUsage {
            input_tokens: 10,
            output_tokens: 6,
            cached_input_tokens: 4,
            reasoning_tokens: Some(5),
        })
    );
    let usage = ledger(rec).usage.expect("usage pair");
    let native = usage.native.expect("native counters");
    assert_eq!(
        (
            native.input_tokens,
            native.cache_read_input_tokens,
            native.reasoning_tokens
        ),
        (10, 4, Some(5))
    );
    assert_eq!(
        (
            usage.normalized.input_tokens,
            usage.normalized.cache_read_input_tokens
        ),
        (6, 4)
    );
    assert_eq!(usage.normalized.reasoning_tokens, None);
}

#[tokio::test]
async fn a_streamed_translated_turn_records_the_providers_own_counters() {
    let (origin, _p) = spawn_provider(Mode::Replay(responses_with_usage())).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay.send(&hello(true)).await;
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert!(ended(&body), "{body}");
    assert_native_beside_normalized(&relay.record().await);
}

#[tokio::test]
async fn a_collected_translated_turn_records_the_providers_own_counters() {
    let (origin, _p) = spawn_provider(Mode::Replay(responses_with_usage())).await;
    let relay = spawn_relay(handler(&origin, Translation::Openai, store())).await;
    let resp = relay.send(&hello(false)).await;
    assert_eq!(resp.status(), 200);
    let _ = resp.bytes().await.unwrap();
    assert_native_beside_normalized(&relay.record().await);
}

#[tokio::test]
async fn a_chat_providers_reasoning_count_survives_into_the_ledger() {
    let chunks = [
        data(json!({"id": "c1", "choices": [{"index": 0, "delta": {"content": "ok"}}]})),
        data(
            json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 6,
                "prompt_tokens_details": {"cached_tokens": 4},
                "completion_tokens_details": {"reasoning_tokens": 5}}}),
        ),
        "data: [DONE]\n\n".to_owned(),
    ]
    .into_iter()
    .map(Bytes::from)
    .collect();
    let (origin, _p) = spawn_provider(Mode::Replay(chunks)).await;
    let relay = spawn_relay(handler(&origin, Translation::Chat, store())).await;
    let body = relay.send(&hello(true)).await.text().await.unwrap();
    assert!(ended(&body), "{body}");
    assert_native_beside_normalized(&relay.record().await);
}

#[tokio::test]
async fn a_provider_that_reports_no_usage_records_no_native_counters() {
    let chunks = [
        data(json!({"id": "c1", "choices": [{"index": 0, "delta": {"content": "ok"}}]})),
        data(json!({"id": "c1", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]})),
        "data: [DONE]\n\n".to_owned(),
    ]
    .into_iter()
    .map(Bytes::from)
    .collect();
    let (origin, _p) = spawn_provider(Mode::Replay(chunks)).await;
    let relay = spawn_relay(handler(&origin, Translation::Chat, store())).await;
    let body = relay.send(&hello(true)).await.text().await.unwrap();
    assert!(ended(&body), "{body}");
    let rec = relay.record().await;
    assert_eq!(rec.outcome.native_usage, None);
    assert!(ledger(&rec).usage.and_then(|u| u.native).is_none());
}

/// Mangled provider streams: whatever arrives, the client gets a stream
/// that ends with `message_stop` or an `error` event, and nothing panics.
#[tokio::test]
async fn mangled_provider_streams_always_end_cleanly() {
    let mut seed: u64 = 0x5eed_c1cd;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        seed >> 33
    };
    let (origin_chat, chat) = spawn_provider(Mode::Status(500)).await;
    let (origin_resp, resp_p) = spawn_provider(Mode::Status(500)).await;
    let relay_chat = spawn_relay(handler(&origin_chat, Translation::Gemini, store())).await;
    let relay_resp = spawn_relay(handler(&origin_resp, Translation::Openai, store())).await;
    for i in 0..96 {
        let (script, relay, p) = if i % 2 == 0 {
            (gemini_two_calls(), &relay_chat, &chat)
        } else {
            (responses_text(), &relay_resp, &resp_p)
        };
        let mut bytes: Vec<u8> = script.concat();
        match next() % 4 {
            0 => bytes.truncate((next() as usize) % bytes.len()),
            1 => {
                for _ in 0..(1 + next() % 4) {
                    let at = (next() as usize) % bytes.len();
                    bytes[at] = (next() % 256) as u8;
                }
            }
            2 => {
                let at = (next() as usize) % bytes.len();
                bytes.splice(
                    at..at,
                    b"\ndata: {\"type\":\"response.output_text.delta\"}\n\n"
                        .iter()
                        .copied(),
                );
            }
            _ => {}
        }
        let mut chunks = Vec::new();
        let mut rest = &bytes[..];
        while !rest.is_empty() {
            let n = 1 + (next() as usize) % rest.len().min(64);
            chunks.push(Bytes::copy_from_slice(&rest[..n]));
            rest = &rest[n..];
        }
        *p.mode.lock() = Mode::Replay(chunks);
        let mut body = hello(true);
        body["tools"] = tools();
        let resp = tokio::time::timeout(Duration::from_secs(10), relay.send(&body))
            .await
            .expect("no hang");
        assert_eq!(resp.status(), 200, "case {i}");
        let text = resp.text().await.unwrap();
        assert!(ended(&text), "case {i}: {text}");
    }
}

// ---------------------------------------------------------------------
// Published and served through `service`
// ---------------------------------------------------------------------

mod service_level {
    use super::*;
    use crate::relay::service;
    use crate::routes::RouteStore;

    #[tokio::test]
    async fn a_translated_route_is_published_served_and_needs_acceptance_to_switch() {
        let d = tempfile::tempdir().unwrap();
        service::set_bind(d.path(), "127.0.0.1", Some(0), None).unwrap();
        let (origin, p) = spawn_provider(Mode::Replay(chat_text("ok"))).await;
        let added = RouteStore::open_at(d.path().join(service::ROUTES_FILENAME))
            .unwrap()
            .add(route(&origin))
            .unwrap();
        let (id, g1) = service::publish_route(d.path(), &added, None, Some(Translation::Chat))
            .await
            .unwrap();
        assert_eq!((id.as_str(), g1), ("translated", 1));
        let config = std::fs::read_to_string(d.path().join("relay-config.json")).unwrap();
        assert!(
            config.contains("\"translate\": \"chat\"") || config.contains("\"translate\":\"chat\""),
            "{config}"
        );

        let serve = |d: std::path::PathBuf| async move {
            service::serve_with_routes(
                &d,
                |_| None,
                |target, r, t, store| {
                    let t = t?;
                    let timeouts = Timeouts::default();
                    let client = base_builder(&timeouts).no_proxy().build().ok()?;
                    Some(service::RouteHandler::Translate(
                        TranslateHandler::with_client(
                            target,
                            r.clone(),
                            t,
                            client,
                            timeouts,
                            Credential::RouteKey(Arc::new(NoKeychain)),
                            store.clone(),
                        ),
                    ))
                },
            )
            .await
            .unwrap()
        };
        let served = serve(d.path().to_owned()).await;
        assert!(served.warnings.is_empty(), "{:?}", served.warnings);
        let (relay, listener) = listen(&served).await;
        let resp = relay.send(&hello(true)).await;
        assert_eq!(resp.status(), 200);
        assert!(ended(&resp.text().await.unwrap()));
        assert_eq!(p.seen.lock()[0].path, "/v1/chat/completions");
        // The publish below takes the serve lock, so both listeners over
        // the served state must have ended.
        drop(relay);
        listener.await.unwrap();
        let mut served = served;
        served.running.stop_and_drain().await;
        drop(served);

        // Switching the protocol is a new target: the old grant is stale.
        let (_, g2) = service::publish_route(d.path(), &added, None, Some(Translation::Openai))
            .await
            .unwrap();
        assert_eq!(g2, 2);
        let served = serve(d.path().to_owned()).await;
        let (relay, _listener) = listen(&served).await;
        let resp = relay.send(&hello(true)).await;
        assert_eq!(error_code(resp).await, (403, "acceptance_required".into()));
        assert_eq!(
            p.hits.load(Ordering::SeqCst),
            1,
            "nothing reached the new target"
        );
    }

    /// Plain HTTP over the serving relay's own state, with a grant at
    /// generation 1. The handle is the listener's task: it holds that
    /// state, and with it the serve lock, until it has ended.
    async fn listen(served: &service::Served) -> (Relay, tokio::task::JoinHandle<()>) {
        let state = served.running.state.clone();
        let grant = Grant {
            upstream_id: tr_id(),
            accepted_generation: 1,
            epoch: state.epoch().to_owned(),
        };
        let issued = state.devices().issue("laptop", vec![grant]).unwrap();
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let shutdown = CancelToken::new();
        let task = tokio::spawn(server::serve(
            l,
            server::router(state.clone()),
            None,
            Limits::default(),
            shutdown.clone(),
        ));
        let relay = Relay {
            addr,
            devices: state.devices().clone(),
            token: issued.plaintext.expose().to_owned(),
            shutdown,
            records: Arc::default(),
        };
        (relay, task)
    }
}

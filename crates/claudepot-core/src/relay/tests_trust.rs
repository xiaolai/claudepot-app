//! Trust, pairing and target-binding suite (plan P2 exit criteria).
//!
//! Real TLS listeners on loopback: the relay under test, and a "MITM"
//! relay with its own CA whose handler counts every request that reaches
//! it. What is asserted is what crossed a socket.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, StatusCode};
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::cancel::CancelToken;
use super::client::{self, ClientError};
use super::devices::{DeviceRegistry, Grant};
use super::identity;
use super::pairing::Pairing;
use super::server::{self, ControlPlane, Limits, RelayState};
use super::target::{Operation, UpstreamId};
use super::tls::{self, TrustPaths};
use super::upstream::{
    AdmittedRequest, Capabilities, Outcome, OutcomeFuture, OutcomeKind, TargetIdentity,
    UpstreamError, UpstreamHandler, UpstreamResponse,
};

/// Counts every request it serves; answers 200 with a fixed body.
struct Counting {
    target: TargetIdentity,
    caps: Capabilities,
    hits: AtomicUsize,
}

impl Counting {
    fn new(id: &str, digest: &str, generation: u64) -> Arc<Self> {
        Arc::new(Self {
            target: TargetIdentity {
                id: UpstreamId::parse(id).unwrap(),
                fingerprint: digest.to_owned(),
                generation,
                kind: crate::relay::identity::TargetKind::Subscription,
            },
            caps: Capabilities {
                operations: [Operation::Messages].into(),
            },
            hits: AtomicUsize::new(0),
        })
    }
    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl UpstreamHandler for Counting {
    fn target(&self) -> &TargetIdentity {
        &self.target
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    async fn status(&self) -> crate::relay::health::UpstreamStatus {
        crate::relay::health::UpstreamStatus {
            credential_ready: true,
            ..Default::default()
        }
    }
    async fn handle(
        &self,
        _req: AdmittedRequest,
        _cancel: CancelToken,
    ) -> Result<(UpstreamResponse, OutcomeFuture), UpstreamError> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(Outcome {
            kind: OutcomeKind::Complete,
            status: Some(200),
            ttfb: None,
            upstream_request_id: None,
            bytes: 2,
            usage: None,
            native_usage: None,
            model: None,
            error: None,
        });
        Ok((
            UpstreamResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Box::pin(futures_util::stream::once(async {
                    Ok(Bytes::from_static(b"ok"))
                })),
            },
            rx,
        ))
    }
}

fn digest(account: &str) -> String {
    identity::TargetSpec::subscription(
        "https://api.anthropic.com",
        account,
        "c8f1a2b3-4d5e-4f60-8172-93a4b5c6d7e8",
    )
    .digest()
}

const ACCOUNT_A: &str = "0b6c4f7e-2a8d-4c3e-9f51-7d2e8a1b3c40";
const ACCOUNT_B: &str = "1d2e3f40-5a6b-4c7d-8e9f-a0b1c2d3e4f5";
const EPOCH: &str = "epoch-1";

struct TlsRelay {
    base: reqwest::Url,
    state: Arc<RelayState>,
    control: Arc<ControlPlane>,
    ca_pem: String,
    _dir: tempfile::TempDir,
    _shutdown: Shutdown,
}

struct Shutdown(CancelToken);
impl Drop for Shutdown {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn names() -> Vec<String> {
    vec!["127.0.0.1".into(), "localhost".into()]
}

async fn spawn_tls_relay(upstreams: Vec<Arc<dyn UpstreamHandler>>) -> TlsRelay {
    let dir = tempfile::tempdir().unwrap();
    let (trust, _) =
        tls::ensure(&TrustPaths::in_dir(dir.path()), &names(), SystemTime::now()).unwrap();
    let control = Arc::new(ControlPlane {
        server_id: uuid::Uuid::new_v4().to_string(),
        ca_pem: trust.ca_pem.clone(),
        ca_fingerprint: trust.ca_fingerprint.clone(),
        pairing: Pairing::new(),
    });
    let mut state = RelayState::new(DeviceRegistry::new(), Limits::default())
        .with_epoch(EPOCH)
        .with_server_id(control.server_id.clone())
        .with_control(control.clone());
    for (i, u) in upstreams.into_iter().enumerate() {
        state = state.with_upstream(u, i == 0);
    }
    let state = Arc::new(state);
    let acceptor = TlsAcceptor::from(tls::server_config(&trust).unwrap());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let shutdown = CancelToken::new();
    tokio::spawn(server::serve(
        l,
        server::router(state.clone()),
        Some(acceptor),
        Limits::default(),
        shutdown.clone(),
    ));
    TlsRelay {
        base: format!("https://{addr}").parse().unwrap(),
        state,
        control,
        ca_pem: trust.ca_pem,
        _dir: dir,
        _shutdown: Shutdown(shutdown),
    }
}

/// A TLS server with its own CA that counts every HTTP request reaching
/// it, by path, and answers `/relay/ca` with its own CA like a real relay.
struct Mitm {
    base: reqwest::Url,
    seen: Arc<Mutex<Vec<String>>>,
    _dir: tempfile::TempDir,
    _shutdown: Shutdown,
}

async fn spawn_mitm() -> Mitm {
    let dir = tempfile::tempdir().unwrap();
    let (trust, _) =
        tls::ensure(&TrustPaths::in_dir(dir.path()), &names(), SystemTime::now()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let ca = trust.ca_pem.clone();
    let fp = trust.ca_fingerprint.clone();
    let seen2 = seen.clone();
    let app = axum::Router::new().fallback(move |req: axum::extract::Request| {
        let seen = seen2.clone();
        let ca = ca.clone();
        let fp = fp.clone();
        async move {
            seen.lock().push(req.uri().path().to_owned());
            let body = serde_json::json!({"ca_pem": ca, "fingerprint": fp, "server_id": "mitm"});
            axum::response::Response::new(Body::from(body.to_string()))
        }
    });
    let acceptor = TlsAcceptor::from(tls::server_config(&trust).unwrap());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let shutdown = CancelToken::new();
    tokio::spawn(server::serve(
        l,
        app,
        Some(acceptor),
        Limits::default(),
        shutdown.clone(),
    ));
    Mitm {
        base: format!("https://{addr}").parse().unwrap(),
        seen,
        _dir: dir,
        _shutdown: Shutdown(shutdown),
    }
}

async fn pair_with(r: &TlsRelay, salt: &str) -> client::Paired {
    let code = r
        .control
        .pairing
        .offer(false, std::time::Instant::now())
        .code;
    let fetched = client::fetch_ca(&r.base).await.unwrap();
    client::pair(
        &r.base,
        &fetched,
        &r.control.ca_fingerprint,
        &code,
        "laptop",
        salt,
    )
    .await
    .unwrap()
}

async fn messages(r: &TlsRelay, token: &str) -> (u16, String) {
    let resp = client::pinned_client(&r.ca_pem)
        .unwrap()
        .post(r.base.join("/v1/messages").unwrap())
        .bearer_auth(token)
        .body("{}")
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap();
    let code = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
    (status, code)
}

// ---------------------------------------------------------------------
// Pairing and the pinned connection
// ---------------------------------------------------------------------

#[tokio::test]
async fn pairing_grants_a_working_token_and_salted_fingerprints() {
    let a = Counting::new("sub", &digest(ACCOUNT_A), 1);
    let r = spawn_tls_relay(vec![a.clone()]).await;
    let salt = client::new_client_salt();
    let paired = pair_with(&r, &salt).await;

    assert!(paired.token.expose().starts_with("cprl_"));
    assert_eq!(paired.server_id, r.control.server_id);
    let offer = &paired.upstreams[0];
    let salt_bytes = identity::parse_salt(&salt).unwrap();
    assert_eq!(
        offer.fingerprint,
        identity::client_fingerprint(&salt_bytes, &digest(ACCOUNT_A))
    );
    assert_ne!(offer.fingerprint, digest(ACCOUNT_A), "never the raw digest");

    assert_eq!(messages(&r, paired.token.expose()).await.0, 200);
    assert_eq!(a.hits(), 1);
}

#[tokio::test]
async fn a_substituted_ca_at_fetch_receives_no_secret() {
    let real = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
    let mitm = spawn_mitm().await;
    let code = real
        .control
        .pairing
        .offer(false, std::time::Instant::now())
        .code;

    // The client was pointed at the MITM and approves the REAL fingerprint
    // (read off the server's screen).
    let fetched = client::fetch_ca(&mitm.base).await.unwrap();
    let r = client::pair(
        &mitm.base,
        &fetched,
        &real.control.ca_fingerprint,
        &code,
        "laptop",
        &client::new_client_salt(),
    )
    .await;
    assert!(matches!(r, Err(ClientError::FingerprintMismatch)));
    // Even the server's self-reported fingerprint cannot talk the client
    // round: it is the computed one that is compared.
    let mut lying = fetched.clone();
    lying.fingerprint = real.control.ca_fingerprint.clone();
    assert!(client::check_fingerprint(&lying, &real.control.ca_fingerprint).is_err());

    assert_eq!(*mitm.seen.lock(), vec!["/relay/ca".to_owned()]);
}

#[tokio::test]
async fn a_substituted_chain_on_reconnect_receives_no_request_at_all() {
    let a = Counting::new("sub", &digest(ACCOUNT_A), 1);
    let real = spawn_tls_relay(vec![a.clone()]).await;
    let salt = client::new_client_salt();
    let paired = pair_with(&real, &salt).await;
    let mitm = spawn_mitm().await;

    // The client reconnects to what it believes is the relay, pinned to
    // the real CA, carrying the token.
    let r = client::accept(&mitm.base, &real.ca_pem, &paired.token, "sub", None, &salt).await;
    assert!(matches!(r, Err(ClientError::Transport)), "{r:?}");
    let api = client::pinned_client(&real.ca_pem)
        .unwrap()
        .post(mitm.base.join("/v1/messages").unwrap())
        .bearer_auth(paired.token.expose())
        .send()
        .await;
    assert!(api.is_err());
    assert!(
        mitm.seen.lock().is_empty(),
        "the MITM saw {:?}",
        mitm.seen.lock()
    );
}

#[tokio::test]
async fn a_code_is_single_use_and_a_wrong_code_issues_nothing() {
    let r = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
    let fetched = client::fetch_ca(&r.base).await.unwrap();
    let fp = r.control.ca_fingerprint.clone();
    let salt = client::new_client_salt();

    let code = r
        .control
        .pairing
        .offer(false, std::time::Instant::now())
        .code;
    let wrong = client::pair(&r.base, &fetched, &fp, "XXXXXXXX", "x", &salt).await;
    assert!(matches!(
        wrong,
        Err(ClientError::Refused { status: 403, ref code }) if code == "pairing_code_invalid"
    ));
    assert!(client::pair(&r.base, &fetched, &fp, &code, "x", &salt)
        .await
        .is_ok());
    let again = client::pair(&r.base, &fetched, &fp, &code, "y", &salt).await;
    assert!(matches!(
        again,
        Err(ClientError::Refused { ref code, .. }) if code == "pairing_unavailable"
    ));
    assert_eq!(r.state.devices().list().len(), 1);
}

#[tokio::test]
async fn pairing_is_not_served_without_tls() {
    let state = Arc::new(
        RelayState::new(DeviceRegistry::new(), Limits::default())
            .with_upstream(Counting::new("sub", &digest(ACCOUNT_A), 1), true),
    );
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let shutdown = CancelToken::new();
    let _s = Shutdown(shutdown.clone());
    tokio::spawn(server::serve(
        l,
        server::router(state),
        None,
        Limits::default(),
        shutdown,
    ));
    let c = reqwest::Client::builder().no_proxy().build().unwrap();
    for (m, p) in [("GET", "/relay/ca"), ("POST", "/relay/pair")] {
        let resp = c
            .request(m.parse().unwrap(), format!("http://{addr}{p}"))
            .body("{}")
            .send()
            .await
            .unwrap();
        // Unauthenticated callers learn nothing: 403, not 404.
        assert_eq!(resp.status(), 403, "{p}");
    }
}

// ---------------------------------------------------------------------
// Relay and remote-control tokens never cross
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_remote_control_token_is_refused_by_the_relay() {
    let r = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
    let remote = crate::remote::token::new_device_token();
    // Even a relay registry that somehow held the remote token's hash.
    let (status, code) = messages(&r, &remote.plaintext).await;
    assert_eq!((status, code.as_str()), (403, "relay_unauthenticated"));
    let prefixed = format!("cprl_{}", remote.plaintext);
    assert_eq!(messages(&r, &prefixed).await.0, 403);
}

#[tokio::test]
async fn a_relay_token_is_refused_by_remote_control() {
    let r = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
    let paired = pair_with(&r, &client::new_client_salt()).await;
    let remote = crate::remote::token::new_device_token();
    let remote_devices = vec![crate::remote::Device {
        id: uuid::Uuid::new_v4(),
        name: "phone".into(),
        token_hash: remote.hash.clone(),
        created_at: chrono::Utc::now(),
        last_seen: None,
        revoked_at: None,
        expires_at: None,
    }];
    let now = chrono::Utc::now();
    assert!(crate::remote::authenticate(&remote_devices, paired.token.expose(), now).is_none());
    let bare = paired.token.expose().trim_start_matches("cprl_");
    assert!(crate::remote::authenticate(&remote_devices, bare, now).is_none());
    assert!(crate::remote::authenticate(&remote_devices, &remote.plaintext, now).is_some());
    // Separate stores, separate files.
    assert_ne!(
        super::store::DEVICES_FILENAME,
        crate::remote::store::DEVICES_FILENAME
    );
}

// ---------------------------------------------------------------------
// Independent trust roots
// ---------------------------------------------------------------------

#[test]
fn rotating_either_trust_root_leaves_the_other_byte_equal() {
    let dir = tempfile::tempdir().unwrap();
    let remote_files = [
        crate::remote::tls::CERT_FILENAME,
        crate::remote::tls::KEY_FILENAME,
        "remote-ca-key.pem",
    ];
    for (i, f) in remote_files.iter().enumerate() {
        std::fs::write(dir.path().join(f), format!("remote material {i}")).unwrap();
    }
    let snapshot = |files: &[&str]| -> Vec<Vec<u8>> {
        files
            .iter()
            .map(|f| std::fs::read(dir.path().join(f)).unwrap())
            .collect()
    };
    let before = snapshot(&remote_files);

    let paths = TrustPaths::in_dir(dir.path());
    let first = tls::ensure(&paths, &names(), SystemTime::now()).unwrap().0;
    let rotated = tls::rotate_ca(&paths, &names(), SystemTime::now()).unwrap();
    assert_ne!(first.ca_fingerprint, rotated.ca_fingerprint);
    assert_eq!(
        snapshot(&remote_files),
        before,
        "relay rotation touched remote"
    );

    let relay_files = [
        tls::CA_CERT_FILENAME,
        tls::CA_KEY_FILENAME,
        tls::CERT_FILENAME,
        tls::KEY_FILENAME,
    ];
    let relay_before = snapshot(&relay_files);
    // A remote-control re-mint rewrites only its own files.
    for f in remote_files {
        std::fs::write(dir.path().join(f), b"re-minted remote material").unwrap();
    }
    let (after, ensured) = tls::ensure(&paths, &names(), SystemTime::now()).unwrap();
    assert_eq!(ensured, tls::Ensured::Loaded);
    assert_eq!(after.ca_fingerprint, rotated.ca_fingerprint);
    assert_eq!(
        snapshot(&relay_files),
        relay_before,
        "remote rotation touched relay"
    );
}

// ---------------------------------------------------------------------
// Target binding: an account change never reaches B before `accept`
// ---------------------------------------------------------------------

async fn accept_flow(r: &TlsRelay, paired: &client::Paired, salt: &str) -> client::Acceptance {
    let shown = client::accept(&r.base, &r.ca_pem, &paired.token, "sub", None, salt)
        .await
        .unwrap();
    assert!(!shown.accepted);
    client::accept(
        &r.base,
        &r.ca_pem,
        &paired.token,
        "sub",
        Some(&shown.fingerprint),
        salt,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn an_account_change_after_the_health_check_reaches_b_zero_times_before_accept() {
    let a = Counting::new("sub", &digest(ACCOUNT_A), 1);
    let r = spawn_tls_relay(vec![a.clone()]).await;
    let salt = client::new_client_salt();
    let paired = pair_with(&r, &salt).await;
    // A health check would see A here. Then the served account changes
    // before the first request is admitted.
    let b = Counting::new("sub", &digest(ACCOUNT_B), 2);
    r.state.replace_upstream(b.clone());

    for _ in 0..3 {
        let (status, code) = messages(&r, paired.token.expose()).await;
        assert_eq!((status, code.as_str()), (403, "acceptance_required"));
    }
    assert_eq!((a.hits(), b.hits()), (0, 0));

    // A stale fingerprint is not an acceptance.
    let stale = paired.upstreams[0].fingerprint.clone();
    let wrong = client::accept(
        &r.base,
        &r.ca_pem,
        &paired.token,
        "sub",
        Some(&stale),
        &salt,
    )
    .await;
    assert!(matches!(
        wrong,
        Err(ClientError::Refused { ref code, .. }) if code == "fingerprint_mismatch"
    ));
    assert_eq!(b.hits(), 0);

    let accepted = accept_flow(&r, &paired, &salt).await;
    assert!(accepted.accepted);
    assert_eq!(accepted.generation, 2);
    assert_eq!(messages(&r, paired.token.expose()).await.0, 200);
    assert_eq!((a.hits(), b.hits()), (0, 1));
}

#[tokio::test]
async fn an_account_change_mid_session_reaches_b_zero_times_before_accept() {
    let a = Counting::new("sub", &digest(ACCOUNT_A), 1);
    let r = spawn_tls_relay(vec![a.clone()]).await;
    let salt = client::new_client_salt();
    let paired = pair_with(&r, &salt).await;
    // The session is under way against A.
    for _ in 0..2 {
        assert_eq!(messages(&r, paired.token.expose()).await.0, 200);
    }
    let b = Counting::new("sub", &digest(ACCOUNT_B), 2);
    r.state.replace_upstream(b.clone());
    let (status, code) = messages(&r, paired.token.expose()).await;
    assert_eq!((status, code.as_str()), (403, "acceptance_required"));
    assert_eq!((a.hits(), b.hits()), (2, 0));
    accept_flow(&r, &paired, &salt).await;
    assert_eq!(messages(&r, paired.token.expose()).await.0, 200);
    assert_eq!((a.hits(), b.hits()), (2, 1));
}

#[tokio::test]
async fn a_grant_from_another_epoch_is_not_honoured() {
    let a = Counting::new("sub", &digest(ACCOUNT_A), 1);
    let r = spawn_tls_relay(vec![a.clone()]).await;
    // Same id, same generation number, but recorded before a config
    // reset restarted the numbering.
    let old = r
        .state
        .devices()
        .issue(
            "old",
            vec![Grant {
                upstream_id: UpstreamId::parse("sub").unwrap(),
                accepted_generation: 1,
                epoch: "epoch-0".into(),
            }],
        )
        .unwrap();
    let (status, code) = messages(&r, old.plaintext.expose()).await;
    assert_eq!((status, code.as_str()), (403, "acceptance_required"));
    assert_eq!(a.hits(), 0);
}

// ---------------------------------------------------------------------
// Grants are a snapshot
// ---------------------------------------------------------------------

async fn named(r: &TlsRelay, token: &str, id: &str) -> (u16, String) {
    let resp = client::pinned_client(&r.ca_pem)
        .unwrap()
        .post(r.base.join(&format!("/u/{id}/v1/messages")).unwrap())
        .bearer_auth(token)
        .body("{}")
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let v: serde_json::Value =
        serde_json::from_slice(&resp.bytes().await.unwrap()).unwrap_or_default();
    (
        status,
        v["error"]["message"].as_str().unwrap_or("").to_owned(),
    )
}

#[tokio::test]
async fn a_new_publication_is_unreachable_by_an_old_token() {
    let r = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
    let salt = client::new_client_salt();
    let paired = pair_with(&r, &salt).await;
    let ds = Counting::new("deepseek", "ds-digest", 1);
    r.state.replace_upstream(ds.clone());

    let (status, code) = named(&r, paired.token.expose(), "deepseek").await;
    assert_eq!((status, code.as_str()), (403, "upstream_not_granted"));
    // Nor can the old token grant it to itself.
    let r2 = client::accept(&r.base, &r.ca_pem, &paired.token, "deepseek", None, &salt).await;
    assert!(matches!(
        r2,
        Err(ClientError::Refused { ref code, .. }) if code == "upstream_not_granted"
    ));
    assert_eq!(ds.hits(), 0);
}

#[tokio::test]
async fn include_future_offers_a_new_publication_for_acceptance_only() {
    let r = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
    let salt = client::new_client_salt();
    let code = r
        .control
        .pairing
        .offer(true, std::time::Instant::now())
        .code;
    let fetched = client::fetch_ca(&r.base).await.unwrap();
    let paired = client::pair(
        &r.base,
        &fetched,
        &r.control.ca_fingerprint,
        &code,
        "laptop",
        &salt,
    )
    .await
    .unwrap();
    let ds = Counting::new("deepseek", "ds-digest", 1);
    r.state.replace_upstream(ds.clone());

    let (status, code) = named(&r, paired.token.expose(), "deepseek").await;
    assert_eq!((status, code.as_str()), (403, "acceptance_required"));
    assert_eq!(ds.hits(), 0);
    let shown = client::accept(&r.base, &r.ca_pem, &paired.token, "deepseek", None, &salt)
        .await
        .unwrap();
    client::accept(
        &r.base,
        &r.ca_pem,
        &paired.token,
        "deepseek",
        Some(&shown.fingerprint),
        &salt,
    )
    .await
    .unwrap();
    assert_eq!(named(&r, paired.token.expose(), "deepseek").await.0, 200);
    assert_eq!(ds.hits(), 1);
}

// ---------------------------------------------------------------------
// Fresh profile, then restart from the same stores
// ---------------------------------------------------------------------

#[tokio::test]
async fn a_pairing_survives_a_restart_from_the_same_stores() {
    use super::runtime::{self, Serving};
    use super::service::acquire_serve_lock;

    let dir = tempfile::tempdir().unwrap();
    let serving = Serving {
        bind: "127.0.0.1".parse().unwrap(),
        port: 0,
        tls: true,
    };
    let boot = |hits: Arc<Counting>| {
        let lock = acquire_serve_lock(dir.path()).unwrap();
        runtime::prepare(lock, serving, vec![hits], SystemTime::now()).unwrap()
    };

    let a = Counting::new("sub", &digest(ACCOUNT_A), 1);
    let first = boot(a.clone());
    let ca_pem = first.trust.as_ref().unwrap().0.ca_pem.clone();
    let fp = first.trust.as_ref().unwrap().0.ca_fingerprint.clone();
    let mut running = runtime::start(first).await.unwrap();
    let base: reqwest::Url = format!("https://{}", running.addr).parse().unwrap();
    let code = running
        .state
        .control()
        .unwrap()
        .pairing
        .offer(false, std::time::Instant::now())
        .code;
    let fetched = client::fetch_ca(&base).await.unwrap();
    let salt = client::new_client_salt();
    let paired = client::pair(&base, &fetched, &fp, &code, "laptop", &salt)
        .await
        .unwrap();
    // The restart takes the serve lock, which the first relay holds until
    // its last connection has let go of it.
    running.stop_and_drain().await;
    drop(running);

    let second = boot(a.clone());
    assert_eq!(second.trust.as_ref().unwrap().0.ca_fingerprint, fp);
    let running = runtime::start(second).await.unwrap();
    let base: reqwest::Url = format!("https://{}", running.addr).parse().unwrap();
    let resp = client::pinned_client(&ca_pem)
        .unwrap()
        .post(base.join("/v1/messages").unwrap())
        .bearer_auth(paired.token.expose())
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(a.hits(), 1);
    // The pairing code died with the first process.
    let again = client::pair(&base, &fetched, &fp, &code, "other", &salt).await;
    assert!(matches!(
        again,
        Err(ClientError::Refused { ref code, .. }) if code == "pairing_unavailable"
    ));
}

// ---------------------------------------------------------------------
// The client verbs end to end (`client_ops`)
// ---------------------------------------------------------------------

mod client_verbs {
    use super::*;
    use crate::relay::client_ops::{self, ClientCtx, ClientOpError, Vault};
    use crate::relay::select::Ineligible;
    use crate::relay::servers;

    fn ctx(
        dir: &std::path::Path,
    ) -> (
        ClientCtx,
        Arc<Mutex<std::collections::HashMap<String, String>>>,
    ) {
        let tokens = Arc::new(Mutex::new(std::collections::HashMap::new()));
        (
            ClientCtx {
                servers_path: dir.join(servers::SERVERS_FILENAME),
                data_dir: dir.to_owned(),
                vault: Vault::Memory(tokens.clone()),
            },
            tokens,
        )
    }

    fn address(r: &TlsRelay) -> String {
        format!("127.0.0.1:{}", r.base.port().unwrap())
    }

    fn set_value<'a>(plan: &'a client_ops::RunPlan, key: &str) -> Option<&'a std::ffi::OsStr> {
        plan.env
            .set
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_os_str())
    }

    #[tokio::test]
    async fn a_wrong_fingerprint_pairs_nothing_and_stores_nothing() {
        let r = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tokens) = ctx(dir.path());
        let code = r
            .control
            .pairing
            .offer(false, std::time::Instant::now())
            .code;
        let offer = client_ops::fetch_ca(&address(&r)).await.unwrap();
        assert_eq!(offer.fingerprint, r.control.ca_fingerprint);
        let wrong = "00".repeat(32);
        let err = client_ops::complete_pair(&ctx, &offer, &wrong, &code, "laptop", None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ClientOpError::Client(ClientError::FingerprintMismatch)
        ));
        assert!(tokens.lock().is_empty());
        assert!(servers::load(&ctx.servers_path)
            .unwrap()
            .value
            .servers
            .is_empty());
    }

    #[tokio::test]
    async fn a_failed_re_pair_keeps_the_previous_token() {
        let r = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tokens) = ctx(dir.path());
        let pair = |name: &'static str| {
            let code = r
                .control
                .pairing
                .offer(false, std::time::Instant::now())
                .code;
            let ctx = &ctx;
            let address = address(&r);
            async move {
                let offer = client_ops::fetch_ca(&address).await.unwrap();
                client_ops::complete_pair(
                    ctx,
                    &offer,
                    &offer.fingerprint.clone(),
                    &code,
                    "laptop",
                    Some(name),
                )
                .await
            }
        };
        let first = pair("studio").await.unwrap();
        let before = tokens.lock().get(&first.server_id).cloned().unwrap();
        // A second record already owns the name the re-pair asks for, so
        // its save is refused after the new token was written.
        servers::mutate(&ctx.servers_path, |f| {
            let mut other = first.clone();
            other.server_id = "another-relay".into();
            other.name = "other".into();
            f.upsert(other);
        })
        .unwrap();
        let err = pair("other").await.unwrap_err();
        assert!(matches!(err, ClientOpError::Store(_)), "{err}");
        assert_eq!(
            tokens.lock().get(&first.server_id).cloned(),
            Some(before),
            "the record that still exists must keep its working token"
        );
    }

    #[tokio::test]
    async fn pair_run_and_accept_work_end_to_end_against_a_real_relay() {
        let a = Counting::new("sub", &digest(ACCOUNT_A), 1);
        let r = spawn_tls_relay(vec![a.clone()]).await;
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tokens) = ctx(dir.path());

        // Pair: the token goes to the vault, never to the JSON file.
        let code = r
            .control
            .pairing
            .offer(false, std::time::Instant::now())
            .code;
        let offer = client_ops::fetch_ca(&address(&r)).await.unwrap();
        let record = client_ops::complete_pair(
            &ctx,
            &offer,
            &offer.fingerprint.clone(),
            &code,
            "laptop",
            Some("studio"),
        )
        .await
        .unwrap();
        assert_eq!(record.server_id, r.control.server_id);
        assert_eq!(record.name, "studio");
        let token = tokens.lock().get(&record.server_id).cloned().unwrap();
        assert!(token.starts_with(crate::relay::devices::TOKEN_PREFIX));
        let on_disk = std::fs::read_to_string(&ctx.servers_path).unwrap();
        assert!(
            !on_disk.contains(&token),
            "the token reached relay-servers.json"
        );

        // Run: the only server, its default upstream, an explicit /u/<id>.
        let plan = client_ops::plan_run(&ctx, None, None, None, chrono::Utc::now())
            .await
            .unwrap();
        assert_eq!(plan.upstream.as_str(), "sub");
        assert_eq!(plan.pick.server_id, record.server_id);
        let base = set_value(&plan, "ANTHROPIC_BASE_URL")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(base.ends_with("/u/sub"), "{base}");
        let bundle = plan.ca_bundle.as_ref().unwrap().path().to_owned();
        assert_eq!(std::fs::read_to_string(&bundle).unwrap(), r.ca_pem);
        assert_eq!(
            set_value(&plan, "NODE_EXTRA_CA_CERTS").map(std::path::PathBuf::from),
            Some(bundle)
        );
        let profile = dir.path().join("relay-profiles").join("u-sub");
        assert!(profile.is_dir());
        assert_eq!(plan.env.token.expose(), token);

        // The served account changes: run refuses with the reason, and
        // nothing reaches the new account.
        let b = Counting::new("sub", &digest(ACCOUNT_B), 2);
        r.state.replace_upstream(b.clone());
        let err = match client_ops::plan_run(&ctx, None, None, None, chrono::Utc::now()).await {
            Err(e) => e,
            Ok(_) => panic!("a changed target must not be runnable"),
        };
        let ClientOpError::NoEligible { excluded, .. } = err else {
            panic!("expected NoEligible");
        };
        assert!(matches!(excluded[0].reason, Ineligible::FingerprintChanged));

        // A stale fingerprint is refused and recorded nowhere.
        let stale = digest(ACCOUNT_A);
        assert!(client_ops::record_acceptance(&ctx, "studio", "sub", &stale)
            .await
            .is_err());
        let file = servers::load(&ctx.servers_path).unwrap().value;
        let up = UpstreamId::parse("sub").unwrap();
        assert_eq!(file.servers[0].approved(&up).unwrap().generation, 1);

        // Accept what was shown; run works again, against B.
        let shown = client_ops::show_acceptance(&ctx, "studio", "sub")
            .await
            .unwrap();
        assert!(!shown.accepted);
        let accepted = client_ops::record_acceptance(&ctx, "studio", "sub", &shown.fingerprint)
            .await
            .unwrap();
        assert_eq!(accepted.generation, 2);
        let file = servers::load(&ctx.servers_path).unwrap().value;
        assert_eq!(file.servers[0].approved(&up).unwrap().generation, 2);
        assert!(
            client_ops::plan_run(&ctx, Some("studio"), Some("sub"), None, chrono::Utc::now())
                .await
                .is_ok()
        );
        assert_eq!((a.hits(), b.hits()), (0, 0), "planning sends no inference");
    }

    #[tokio::test]
    async fn run_names_unknown_servers_and_missing_tokens() {
        let r = spawn_tls_relay(vec![Counting::new("sub", &digest(ACCOUNT_A), 1)]).await;
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tokens) = ctx(dir.path());
        assert!(matches!(
            client_ops::plan_run(&ctx, None, None, None, chrono::Utc::now()).await,
            Err(ClientOpError::NoServers)
        ));
        let code = r
            .control
            .pairing
            .offer(false, std::time::Instant::now())
            .code;
        let offer = client_ops::fetch_ca(&address(&r)).await.unwrap();
        let fp = offer.fingerprint.clone();
        client_ops::complete_pair(&ctx, &offer, &fp, &code, "laptop", None)
            .await
            .unwrap();
        assert!(matches!(
            client_ops::plan_run(&ctx, Some("nope"), None, None, chrono::Utc::now()).await,
            Err(ClientOpError::UnknownServer(_))
        ));
        // A lost token makes the server unreachable, with the reason.
        tokens.lock().clear();
        let Err(ClientOpError::NoEligible { excluded, .. }) =
            client_ops::plan_run(&ctx, None, Some("sub"), None, chrono::Utc::now()).await
        else {
            panic!("expected NoEligible");
        };
        assert!(
            matches!(&excluded[0].reason, Ineligible::Unreachable(why) if why.contains("pair it again"))
        );
    }
}

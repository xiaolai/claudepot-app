//! End-to-end check that the remote surface actually listens.
//!
//! The unit tests drive the router through `tower::ServiceExt::oneshot`,
//! which proves routing and middleware but never opens a socket. This
//! binds a real port on loopback and speaks real HTTP to it, so the
//! accept loop, the body plumbing and the JSON encoding are exercised
//! too.
//!
//! Loopback with port 0 throughout: the OS picks a free port, nothing is
//! reachable off the machine, and no certificate is needed because
//! loopback is already a secure context.

use std::sync::Arc;

use claudepot_core::remote::config::{RemoteConfigFile, ServerConfig};
use claudepot_core::remote::serve;
use claudepot_core::remote::server::{router, AppState, Persist, Shared};
use claudepot_core::remote::DevicesFile;
use tokio::sync::Mutex;

struct NoPersist;
impl Persist for NoPersist {
    fn save(&self, _c: &RemoteConfigFile, _d: &DevicesFile) -> std::io::Result<()> {
        Ok(())
    }
}

const PW: &str = "correct horse battery";

fn state() -> Shared {
    let mut p = PW.to_string();
    let hash = claudepot_core::remote::password::hash_password(&mut p).unwrap();
    Arc::new(Mutex::new(AppState {
        config: RemoteConfigFile {
            password_hash: Some(hash),
            ..Default::default()
        },
        devices: DevicesFile::default(),
        persist: Box::new(NoPersist),
    }))
}

/// Start on an OS-chosen loopback port and return its base URL.
async fn start() -> (String, Shared) {
    let shared = state();
    let cfg = ServerConfig {
        enabled: true,
        bind: "127.0.0.1".parse().unwrap(),
        port: 0,
        allowed_hosts: Vec::new(),
    };
    let (listener, info) = serve::listen(&cfg).await.expect("listen");
    let app = router(shared.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app.into_make_service()).await;
    });
    (format!("http://{}", info.addr), shared)
}

#[tokio::test]
async fn the_server_answers_health_over_a_real_socket() {
    let (base, _s) = start().await;
    let res = reqwest::get(format!("{base}/api/health")).await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.json::<serde_json::Value>().await.unwrap(),
        serde_json::json!({ "ok": true })
    );
}

#[tokio::test]
async fn a_real_login_yields_a_token_that_opens_a_private_route() {
    let (base, _s) = start().await;
    let client = reqwest::Client::new();

    let unauth = client.get(format!("{base}/api/me")).send().await.unwrap();
    assert_eq!(unauth.status(), 401, "private routes are closed by default");

    let res = client
        .post(format!("{base}/api/login"))
        .json(&serde_json::json!({ "password": PW, "device_label": "e2e" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let token = res.json::<serde_json::Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    let me = client
        .get(format!("{base}/api/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
    assert_eq!(
        me.json::<serde_json::Value>().await.unwrap()["device"],
        "e2e"
    );
}

#[tokio::test]
async fn a_wrong_password_is_rejected_over_the_wire() {
    let (base, _s) = start().await;
    let res = reqwest::Client::new()
        .post(format!("{base}/api/login"))
        .json(&serde_json::json!({ "password": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
    assert_eq!(
        res.json::<serde_json::Value>().await.unwrap()["error"],
        "invalid",
        "the error must not say which half was wrong"
    );
}

#[tokio::test]
async fn a_forged_host_header_is_refused_by_a_real_request() {
    // The DNS-rebinding guard, exercised through the network stack
    // rather than a synthesised Request.
    let (base, shared) = start().await;
    let res = reqwest::Client::new()
        .post(format!("{base}/api/login"))
        .header("Host", "evil.example:1234")
        .json(&serde_json::json!({ "password": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 421);
    assert_eq!(
        shared.lock().await.config.failed_attempts,
        0,
        "a rejected Host must never reach the login throttle"
    );
}

#[tokio::test]
async fn the_client_is_served_from_the_binary() {
    let (base, _s) = start().await;
    let res = reqwest::get(&base).await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "text/html; charset=utf-8"
    );
    // The policy that keeps a CDN from creeping in later.
    let csp = res
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(res.text().await.unwrap().contains("Claudepot"));
}

#[tokio::test]
async fn the_service_worker_is_served_at_the_root_scope() {
    // A worker under /static would control only /static, so the scope
    // is a property of the URL and worth asserting.
    let (base, _s) = start().await;
    let res = reqwest::get(format!("{base}/sw.js")).await.unwrap();
    assert_eq!(res.status(), 200);
    assert!(res
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/javascript"));
}

#[tokio::test]
async fn the_manifest_has_the_type_browsers_require() {
    let (base, _s) = start().await;
    let res = reqwest::get(format!("{base}/manifest.webmanifest"))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "application/manifest+json",
        "some browsers ignore a manifest served as application/json"
    );
}

#[tokio::test]
async fn an_unknown_path_is_a_clean_404() {
    let (base, _s) = start().await;
    for p in ["/nope", "/../Cargo.toml", "/api/nothing"] {
        let res = reqwest::get(format!("{base}{p}")).await.unwrap();
        assert_eq!(res.status(), 404, "{p}");
    }
}

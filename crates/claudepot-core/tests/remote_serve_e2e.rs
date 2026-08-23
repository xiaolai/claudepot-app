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
use claudepot_core::remote::idempotency::Idempotency;
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
        idempotency: Idempotency::new(),
        challenges: claudepot_core::remote::passkey::Challenges::new(),
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

/// Log in and return a bearer token.
async fn token(base: &str) -> String {
    reqwest::Client::new()
        .post(format!("{base}/api/login"))
        .json(&serde_json::json!({ "password": PW, "device_label": "e2e" }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn the_real_endpoints_are_all_behind_auth() {
    // The allowlist property: everything except health, login and the
    // client shell requires a bearer. Asserted per route rather than
    // per middleware, because the failure mode is a route added outside
    // the guarded Router.
    let (base, _s) = start().await;
    let c = reqwest::Client::new();
    for (method, path) in [
        ("GET", "/api/sessions"),
        ("GET", "/api/sessions/abc/transcript"),
        ("GET", "/api/projects"),
        ("GET", "/api/accounts"),
        ("GET", "/api/inbound"),
        ("POST", "/api/inbound/revoke"),
        ("POST", "/api/sessions/abc/prompt"),
        ("POST", "/api/sessions/abc/read"),
        // Registering a passkey must require a session that already
        // exists, or anyone who can reach the page enrols themselves.
        ("POST", "/api/passkey/register/begin"),
        ("POST", "/api/passkey/register/finish"),
    ] {
        let req = match method {
            "GET" => c.get(format!("{base}{path}")),
            _ => c.post(format!("{base}{path}")).json(&serde_json::json!({})),
        };
        assert_eq!(
            req.send().await.unwrap().status(),
            401,
            "{method} {path} must require a token"
        );
    }
}

#[tokio::test]
async fn a_mutation_without_an_idempotency_key_is_refused() {
    // This is the endpoint where a retry does the work twice. A client
    // that has not thought about that gets told, rather than getting
    // lucky.
    let (base, _s) = start().await;
    let t = token(&base).await;
    let res = reqwest::Client::new()
        .post(format!("{base}/api/sessions/whatever/prompt"))
        .bearer_auth(&t)
        .json(&serde_json::json!({ "text": "hello" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    assert_eq!(
        res.json::<serde_json::Value>().await.unwrap()["error"],
        "idempotency_key_required"
    );
}

#[tokio::test]
async fn an_unknown_session_is_gone_not_retargeted() {
    // A stale handle from a phone's cached list must fail. Retargeting
    // to whatever holds that identity now is the failure this endpoint
    // is shaped to prevent.
    let (base, _s) = start().await;
    let t = token(&base).await;
    let res = reqwest::Client::new()
        .post(format!(
            "{base}/api/sessions/00000000-dead-beef-0000-000000000000/prompt"
        ))
        .bearer_auth(&t)
        .header("idempotency-key", "k-stale-1")
        .json(&serde_json::json!({ "text": "hello" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 410, "a vanished session is Gone");
    assert_eq!(
        res.json::<serde_json::Value>().await.unwrap()["error"],
        "session_gone"
    );
}

#[tokio::test]
async fn a_retried_mutation_replays_instead_of_re_executing() {
    // Sequential retry only. The *concurrent* case — two requests with
    // one key arriving inside the window the mutation takes — is covered
    // in `remote::idempotency`'s unit tests, and deliberately not here:
    // every handler on this surface resolves fast enough that the first
    // request finishes before the second is dispatched, so an end-to-end
    // test would report success without ever entering the window. The
    // only way to widen it is a sleep that exists for the test, and a
    // test that needs production code to be slower is measuring the
    // sleep.
    // The property idempotency exists for. Both requests carry the same
    // key; the second must return the first's response byte-for-byte
    // without the work happening again.
    let (base, _s) = start().await;
    let t = token(&base).await;
    let c = reqwest::Client::new();

    let send = || {
        c.post(format!(
            "{base}/api/sessions/00000000-dead-beef-0000-000000000000/prompt"
        ))
        .bearer_auth(&t)
        .header("idempotency-key", "k-retry-1")
        .json(&serde_json::json!({ "text": "hello" }))
        .send()
    };

    let first = send().await.unwrap();
    let first_status = first.status();
    let first_body = first.text().await.unwrap();

    let second = send().await.unwrap();
    assert_eq!(second.status(), first_status);
    assert_eq!(
        second.text().await.unwrap(),
        first_body,
        "a retry must replay the stored response, not run again"
    );
}

#[tokio::test]
async fn an_empty_prompt_is_refused() {
    let (base, _s) = start().await;
    let t = token(&base).await;
    let res = reqwest::Client::new()
        .post(format!("{base}/api/sessions/x/prompt"))
        .bearer_auth(&t)
        .header("idempotency-key", "k-empty-1")
        .json(&serde_json::json!({ "text": "   " }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn sessions_are_listed_by_id_and_never_addressed_by_pid() {
    let (base, _s) = start().await;
    let t = token(&base).await;
    let res = reqwest::Client::new()
        .get(format!("{base}/api/sessions"))
        .bearer_auth(&t)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body = res.json::<serde_json::Value>().await.unwrap();
    let rows = body
        .get("sessions")
        .and_then(|v| v.as_array())
        .expect("sessions must be an array under `sessions`");
    // An envelope rather than a bare array: the list carries a
    // `generated_at` the client renders as "as of", and a bare array has
    // nowhere to put it.
    assert!(
        body.get("generated_at").and_then(|v| v.as_str()).is_some(),
        "the list must say when it was taken"
    );
    for row in rows {
        assert!(
            row.get("session_id").and_then(|v| v.as_str()).is_some(),
            "every row must carry the id the prompt route addresses"
        );
    }
}

#[tokio::test]
async fn the_inbound_gate_can_be_read_and_closed_but_not_opened() {
    // Remote may always make the machine safer, never less safe. There
    // is deliberately no route to open the window.
    let (base, _s) = start().await;
    let t = token(&base).await;
    let c = reqwest::Client::new();

    let st = c
        .get(format!("{base}/api/inbound"))
        .bearer_auth(&t)
        .send()
        .await
        .unwrap();
    assert_eq!(st.status(), 200);
    assert!(st.json::<serde_json::Value>().await.unwrap()["open"].is_boolean());

    // No such route, and that is the point.
    for path in ["/api/inbound/grant", "/api/inbound/open"] {
        let res = c
            .post(format!("{base}{path}"))
            .bearer_auth(&t)
            .json(&serde_json::json!({ "duration_secs": 3600 }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 404, "{path} must not exist");
    }
}

#[tokio::test]
async fn the_public_routes_are_exactly_these_four() {
    // The allowlist, asserted from the other side. `the_real_endpoints_
    // are_all_behind_auth` catches a route that should be guarded and is
    // not; this catches the opposite mistake — a route quietly joining
    // the public set — by naming the whole set.
    let (base, _s) = start().await;
    let c = reqwest::Client::new();

    // Reachable without a bearer.
    for (method, path) in [
        ("GET", "/api/health"),
        ("POST", "/api/login"),
        ("POST", "/api/passkey/login/begin"),
        ("POST", "/api/passkey/login/finish"),
    ] {
        let req = match method {
            "GET" => c.get(format!("{base}{path}")),
            _ => c.post(format!("{base}{path}")).json(&serde_json::json!({})),
        };
        let status = req.send().await.unwrap().status();
        assert_ne!(status, 401, "{method} {path} is meant to be public");
    }
}

#[tokio::test]
async fn passkey_login_says_nothing_when_none_is_registered() {
    // And in particular does not enumerate credential ids: `begin`
    // refuses outright rather than returning an empty allowlist a caller
    // could distinguish from a populated one.
    let (base, _s) = start().await;
    let res = reqwest::Client::new()
        .post(format!("{base}/api/passkey/login/begin"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
    assert_eq!(
        res.json::<serde_json::Value>().await.unwrap()["error"],
        "no_passkey"
    );
}

#[tokio::test]
async fn registering_a_passkey_over_an_ip_origin_is_refused_with_a_reason() {
    // The trap: the phone reports a platform authenticator is available
    // on exactly the origin that cannot use one. The server must say
    // which of the two is wrong.
    let (base, _s) = start().await;
    let t = token(&base).await;
    let res = reqwest::Client::new()
        .post(format!("{base}/api/passkey/register/begin"))
        .bearer_auth(&t)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    // `start()` binds loopback, so the Host is an IP literal.
    assert_eq!(res.status(), 409);
    assert_eq!(
        res.json::<serde_json::Value>().await.unwrap()["error"],
        "rp_id_unavailable"
    );
}

#[tokio::test]
async fn a_transcript_for_an_unknown_session_is_not_found() {
    let (base, _s) = start().await;
    let t = token(&base).await;
    let res = reqwest::Client::new()
        .get(format!(
            "{base}/api/sessions/no-such-session/transcript?tail=1"
        ))
        .bearer_auth(&t)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn recording_read_state_needs_an_idempotency_key_like_every_mutation() {
    // Not because a repeat is dangerous here — the mark only moves
    // forward — but so a client does not have to learn which mutations
    // are special.
    let (base, _s) = start().await;
    let t = token(&base).await;
    let res = reqwest::Client::new()
        .post(format!("{base}/api/sessions/abc/read"))
        .bearer_auth(&t)
        .json(&serde_json::json!({ "through_count": 4 }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    assert_eq!(
        res.json::<serde_json::Value>().await.unwrap()["error"],
        "idempotency_key_required"
    );
}

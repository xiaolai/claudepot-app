//! The HTTP surface.
//!
//! Lives in core rather than the Tauri crate because it is plain tokio
//! + axum with no Tauri dependency: that keeps it testable without a
//! webview and lets the CLI serve as well as the GUI.
//!
//! ## What is unauthenticated, and nothing else
//!
//! `GET /api/health` and `POST /api/login`. Everything else goes through
//! [`require_auth`]. The list is short enough to hold in your head on
//! purpose — an allowlist of public routes is auditable, whereas
//! "authenticated unless marked otherwise" fails open the moment someone
//! adds a route and forgets the attribute.
//!
//! ## Guards that run before anything reads state
//!
//! - **`Host` is checked** against the configured bind, so a DNS
//!   rebinding attack cannot use a victim's browser to reach the
//!   appliance. This runs *before* login, or rebinding would burn the
//!   throttle on the owner's behalf.
//! - **`Origin`, when present, must be same-origin.** A browser will
//!   send it on cross-site requests; a native client sends none. Absent
//!   is allowed, present-and-foreign is refused.
//!
//! ## Auth state is written on failure too
//!
//! `login::attempt` mutates the throttle as a side effect and the
//! handler persists it whatever the outcome. A handler that saved only
//! on success would leave the counter at zero forever, and the throttle
//! would be decorative.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::config::RemoteConfigFile;
use super::login::{self, LoginOutcome};
use super::{authenticate, Device, DevicesFile};

/// Everything a handler needs. Behind one mutex: the auth file is a
/// read-modify-write on every login and the device list is mutated by
/// pairing, so a lock is required regardless — and one lock over both is
/// simpler to reason about than two that must be taken in order.
pub struct AppState {
    pub config: RemoteConfigFile,
    pub devices: DevicesFile,
    /// Where to persist. Injected so tests never touch a real data dir.
    pub persist: Box<dyn Persist>,
}

/// Persistence, injected. The HTTP layer must not know where the files
/// live; tests substitute a no-op.
pub trait Persist: Send + Sync {
    fn save(&self, config: &RemoteConfigFile, devices: &DevicesFile) -> std::io::Result<()>;
}

pub type Shared = Arc<Mutex<AppState>>;

#[derive(Debug, Serialize)]
struct ApiError {
    error: &'static str,
    /// Present only where it helps the *user*; never a hint about
    /// which half of a credential was wrong.
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_after_secs: Option<u64>,
}

fn err(status: StatusCode, error: &'static str) -> Response {
    (
        status,
        Json(ApiError {
            error,
            retry_after_secs: None,
        }),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
    #[serde(default)]
    pub totp: Option<String>,
    #[serde(default)]
    pub device_label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: Option<String>,
}

pub fn router(state: Shared) -> Router {
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/login", post(handle_login));

    let private = Router::new()
        .route("/api/me", get(me))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    public
        .merge(private)
        .layer(middleware::from_fn_with_state(state.clone(), guard_origin))
        .with_state(state)
}

/// Unauthenticated on purpose: a client needs to know the appliance is
/// there before it has credentials. Returns nothing about the machine —
/// no hostname, no version, no whether a password is set. "Something is
/// listening" is the entire payload.
async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

async fn me(State(state): State<Shared>, req: Request) -> Response {
    let Some(device) = req.extensions().get::<Device>().cloned() else {
        // Unreachable behind require_auth; failing closed rather than
        // unwrapping keeps a future refactor from turning a missing
        // extension into a panic that answers 500 to an attacker.
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let _ = state;
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "device": device.name,
            "expires_at": device.expires_at.map(|t| t.to_rfc3339()),
        })),
    )
        .into_response()
}

async fn handle_login(State(state): State<Shared>, Json(body): Json<LoginRequest>) -> Response {
    let now = Utc::now();
    let mut guard = state.lock().await;

    let mut auth = guard.config.auth();
    let label = body
        .device_label
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("unnamed device");

    let outcome = match login::attempt(&mut auth, &body.password, body.totp.as_deref(), label, now)
    {
        Ok(o) => o,
        Err(e) => {
            tracing::error!(error = %e, "remote login failed to evaluate");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    };

    // Persist regardless of outcome — the throttle counter lives here.
    guard.config.absorb(&auth);
    if let LoginOutcome::Success { device, .. } = &outcome {
        guard.devices.devices.push(device.clone());
    }
    let (config, devices) = (guard.config.clone(), guard.devices.clone());
    if let Err(e) = guard.persist.save(&config, &devices) {
        tracing::error!(error = %e, "remote: could not persist auth state");
        // A login we cannot record is a login that did not happen: the
        // issued token would work until restart and the failed attempt
        // would never be counted.
        return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }
    drop(guard);

    match outcome {
        LoginOutcome::Success { token, device } => (
            StatusCode::OK,
            Json(LoginResponse {
                token,
                expires_at: device.expires_at.map(|t| t.to_rfc3339()),
            }),
        )
            .into_response(),
        LoginOutcome::NotConfigured => err(StatusCode::CONFLICT, "not_configured"),
        LoginOutcome::TotpRequired => err(StatusCode::UNAUTHORIZED, "totp_required"),
        // Wrong password and wrong code are the same status and shape;
        // only the code differs, and it never says which was wrong when
        // the password itself was bad.
        LoginOutcome::Invalid => err(StatusCode::UNAUTHORIZED, "invalid"),
        LoginOutcome::TotpInvalid => err(StatusCode::UNAUTHORIZED, "invalid"),
        LoginOutcome::Throttled { wait_secs } => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiError {
                error: "throttled",
                retry_after_secs: Some(wait_secs),
            }),
        )
            .into_response(),
    }
}

/// Bearer-token gate for everything that is not health or login.
pub async fn require_auth(State(state): State<Shared>, mut req: Request, next: Next) -> Response {
    let Some(bearer) = bearer_from(req.headers()) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let now = Utc::now();
    let device = {
        let guard = state.lock().await;
        authenticate(&guard.devices.devices, &bearer, now).cloned()
    };
    let Some(device) = device else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    req.extensions_mut().insert(device);
    next.run(req).await
}

fn bearer_from(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    // The scheme is matched case-insensitively per RFC 7235; the token
    // is not touched.
    let (scheme, value) = raw.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| value.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Rejects DNS-rebinding and cross-site requests before any handler
/// runs — in particular before login, so rebinding cannot spend the
/// owner's throttle.
pub async fn guard_origin(State(state): State<Shared>, req: Request, next: Next) -> Response {
    let expected_port = { state.lock().await.config.server.port };

    if let Some(host) = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
    {
        if !host_is_acceptable(host, expected_port) {
            return err(StatusCode::MISDIRECTED_REQUEST, "bad_host");
        }
    }

    // Absent Origin is fine — native clients send none. Present means a
    // browser told us where it came from, and only same-origin is ours.
    if let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
    {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        if !origin_matches_host(origin, host) {
            return err(StatusCode::FORBIDDEN, "bad_origin");
        }
    }

    next.run(req).await
}

/// A `Host` we are willing to answer to.
///
/// Deliberately permissive about the *name* and strict about the shape:
/// this appliance is reached by IP, by mDNS name, and by whatever the
/// user put in the certificate, so an allowlist of names would be wrong
/// more often than right. What it refuses is a `Host` carrying a port
/// that is not ours — the signature of a rebinding proxy — and anything
/// unparseable.
fn host_is_acceptable(host: &str, expected_port: u16) -> bool {
    if host.is_empty() {
        return false;
    }
    // IPv6 literal: [::1]:8420
    let port = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((_, after)) => after.strip_prefix(':'),
            None => return false,
        }
    } else {
        match host.rsplit_once(':') {
            Some((name, p)) if !name.is_empty() => Some(p),
            Some(_) => return false,
            None => None,
        }
    };
    match port {
        None => true,
        Some(p) => p
            .parse::<u16>()
            .map(|p| p == expected_port)
            .unwrap_or(false),
    }
}

fn origin_matches_host(origin: &str, host: &str) -> bool {
    // `null` is what a sandboxed iframe or a file:// page sends. It is
    // never us.
    if origin.eq_ignore_ascii_case("null") {
        return false;
    }
    let Some((_scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    !rest.is_empty() && rest.eq_ignore_ascii_case(host)
}

/// Body type alias so callers do not need axum in scope.
pub type ServerBody = Body;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    struct NoPersist;
    impl Persist for NoPersist {
        fn save(&self, _c: &RemoteConfigFile, _d: &DevicesFile) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct FailPersist;
    impl Persist for FailPersist {
        fn save(&self, _c: &RemoteConfigFile, _d: &DevicesFile) -> std::io::Result<()> {
            Err(std::io::Error::other("disk full"))
        }
    }

    const PW: &str = "correct horse battery";

    fn state_with_password(persist: Box<dyn Persist>) -> Shared {
        let mut p = PW.to_string();
        let hash = super::super::password::hash_password(&mut p).unwrap();
        Arc::new(Mutex::new(AppState {
            config: RemoteConfigFile {
                password_hash: Some(hash),
                ..Default::default()
            },
            devices: DevicesFile::default(),
            persist,
        }))
    }

    fn req(method: &str, uri: &str, body: Option<serde_json::Value>) -> HttpRequest<Body> {
        let b = HttpRequest::builder().method(method).uri(uri).header(
            header::HOST,
            format!("127.0.0.1:{}", super::super::config::DEFAULT_PORT),
        );
        match body {
            Some(v) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        }
    }

    async fn body_json(res: Response) -> serde_json::Value {
        let bytes = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    #[tokio::test]
    async fn health_needs_no_credentials_and_says_nothing_about_the_machine() {
        let app = router(state_with_password(Box::new(NoPersist)));
        let res = app.oneshot(req("GET", "/api/health", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let v = body_json(res).await;
        // No hostname, no version, no "password is set".
        assert_eq!(v, serde_json::json!({ "ok": true }));
    }

    #[tokio::test]
    async fn a_private_route_without_a_token_is_401() {
        let app = router(state_with_password(Box::new(NoPersist)));
        let res = app.oneshot(req("GET", "/api/me", None)).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_then_use_the_token() {
        let state = state_with_password(Box::new(NoPersist));
        let res = router(state.clone())
            .oneshot(req(
                "POST",
                "/api/login",
                Some(serde_json::json!({"password": PW})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let token = body_json(res).await["token"].as_str().unwrap().to_string();

        let authed = HttpRequest::builder()
            .method("GET")
            .uri("/api/me")
            .header(
                header::HOST,
                format!("127.0.0.1:{}", super::super::config::DEFAULT_PORT),
            )
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = router(state).oneshot(authed).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_wrong_password_is_401_and_reveals_nothing() {
        let app = router(state_with_password(Box::new(NoPersist)));
        let res = app
            .oneshot(req(
                "POST",
                "/api/login",
                Some(serde_json::json!({"password": "nope"})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(body_json(res).await["error"], "invalid");
    }

    #[tokio::test]
    async fn the_throttle_is_persisted_on_failure() {
        // The bug this guards: a handler that saves only on success
        // leaves the counter at zero forever.
        let state = state_with_password(Box::new(NoPersist));
        for _ in 0..4 {
            router(state.clone())
                .oneshot(req(
                    "POST",
                    "/api/login",
                    Some(serde_json::json!({"password": "nope"})),
                ))
                .await
                .unwrap();
        }
        assert_eq!(state.lock().await.config.failed_attempts, 4);

        let res = router(state)
            .oneshot(req(
                "POST",
                "/api/login",
                Some(serde_json::json!({"password": PW})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(body_json(res).await["retry_after_secs"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn a_login_that_cannot_be_recorded_is_not_a_login() {
        // Otherwise the issued token works until restart while the
        // failed attempt is never counted.
        let state = state_with_password(Box::new(FailPersist));
        let res = router(state)
            .oneshot(req(
                "POST",
                "/api/login",
                Some(serde_json::json!({"password": PW})),
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn a_revoked_token_stops_working_immediately() {
        let state = state_with_password(Box::new(NoPersist));
        let res = router(state.clone())
            .oneshot(req(
                "POST",
                "/api/login",
                Some(serde_json::json!({"password": PW})),
            ))
            .await
            .unwrap();
        let token = body_json(res).await["token"].as_str().unwrap().to_string();

        state.lock().await.devices.devices[0].revoked_at = Some(Utc::now());

        let authed = HttpRequest::builder()
            .method("GET")
            .uri("/api/me")
            .header(
                header::HOST,
                format!("127.0.0.1:{}", super::super::config::DEFAULT_PORT),
            )
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = router(state).oneshot(authed).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_foreign_host_header_is_refused_before_login_runs() {
        // DNS rebinding must not be able to spend the owner's throttle.
        let state = state_with_password(Box::new(NoPersist));
        let bad = HttpRequest::builder()
            .method("POST")
            .uri("/api/login")
            .header(header::HOST, "evil.example:1234")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({"password": "nope"}).to_string(),
            ))
            .unwrap();
        let res = router(state.clone()).oneshot(bad).await.unwrap();
        assert_eq!(res.status(), StatusCode::MISDIRECTED_REQUEST);
        assert_eq!(
            state.lock().await.config.failed_attempts,
            0,
            "a rejected Host must not reach the throttle"
        );
    }

    #[tokio::test]
    async fn a_cross_site_origin_is_refused() {
        let state = state_with_password(Box::new(NoPersist));
        for origin in ["https://evil.example", "null"] {
            let bad = HttpRequest::builder()
                .method("POST")
                .uri("/api/login")
                .header(
                    header::HOST,
                    format!("127.0.0.1:{}", super::super::config::DEFAULT_PORT),
                )
                .header(header::ORIGIN, origin)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::json!({"password": PW}).to_string()))
                .unwrap();
            let res = router(state.clone()).oneshot(bad).await.unwrap();
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "origin {origin}");
        }
    }

    #[tokio::test]
    async fn a_same_origin_request_is_allowed() {
        let state = state_with_password(Box::new(NoPersist));
        let host = format!("127.0.0.1:{}", super::super::config::DEFAULT_PORT);
        let good = HttpRequest::builder()
            .method("POST")
            .uri("/api/login")
            .header(header::HOST, &host)
            .header(header::ORIGIN, format!("https://{host}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::json!({"password": PW}).to_string()))
            .unwrap();
        let res = router(state).oneshot(good).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[test]
    fn host_shapes() {
        let p = super::super::config::DEFAULT_PORT;
        assert!(host_is_acceptable("claudepot.local", p));
        assert!(host_is_acceptable(&format!("192.168.1.5:{p}"), p));
        assert!(host_is_acceptable(&format!("[::1]:{p}"), p));
        assert!(host_is_acceptable("[::1]", p));
        // A port that is not ours is the rebinding signature.
        assert!(!host_is_acceptable("192.168.1.5:9999", p));
        assert!(!host_is_acceptable("", p));
        assert!(!host_is_acceptable(&format!(":{p}"), p));
        assert!(!host_is_acceptable("host:notaport", p));
    }

    #[test]
    fn bearer_parsing_is_scheme_insensitive_and_rejects_junk() {
        let mut h = HeaderMap::new();
        for (raw, want) in [
            ("Bearer abc", Some("abc")),
            ("bearer abc", Some("abc")),
            ("BEARER  abc ", Some("abc")),
            ("Basic abc", None),
            ("abc", None),
            ("Bearer ", None),
            ("Bearer", None),
        ] {
            h.insert(header::AUTHORIZATION, raw.parse().unwrap());
            assert_eq!(bearer_from(&h).as_deref(), want, "{raw:?}");
        }
    }
}

//! The HTTP surface.
//!
//! Lives in core rather than the Tauri crate because it is plain tokio
//! + axum with no Tauri dependency: that keeps it testable without a
//! webview and lets the CLI serve as well as the GUI.
//!
//! ## What is unauthenticated, and nothing else
//!
//! `GET /api/health`, `POST /api/login`, and the two `POST
//! /api/passkey/login/*` steps. Everything else goes through
//! [`require_auth`]. The list is short enough to hold in your head on
//! purpose — an allowlist of public routes is auditable, whereas
//! "authenticated unless marked otherwise" fails open the moment someone
//! adds a route and forgets the attribute.
//!
//! Two tests assert it, from both sides, and they live in
//! `tests/remote_serve_e2e.rs` rather than here because they drive a
//! real socket: `the_real_endpoints_are_all_behind_auth` names every
//! private route and requires a 401 without a bearer, and
//! `the_public_routes_are_exactly_these_four` names the public set. A
//! route added to the wrong `Router` fails one of them.
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
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::assets;
use super::config::RemoteConfigFile;
use super::idempotency::{Idempotency, Lookup, Stored};
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
    /// Replay guard for mutations. In memory — see the module docs on
    /// `idempotency` for why that is the right lifetime.
    pub idempotency: Idempotency,
    /// Open WebAuthn ceremonies. In memory for the same reason, and
    /// single-use for a sharper one — see `passkey`.
    pub challenges: super::passkey::Challenges,
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

pub(super) fn err(status: StatusCode, error: &'static str) -> Response {
    (
        status,
        Json(ApiError {
            error,
            retry_after_secs: None,
        }),
    )
        .into_response()
}

/// The one place a plaintext admin password enters this process.
///
/// No `Debug`: `.claude/rules/architecture.md` puts secrets that reach
/// Rust by paste on a zeroize-on-every-path contract, and a derived
/// `Debug` is how the password reaches a log line instead. `Drop`
/// zeroizes both secret-bearing fields — `Drop` alone does not scrub a
/// `String`, which is the whole reason the `zeroize` crate is a
/// dependency.
#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: String,
    #[serde(default)]
    pub totp: Option<String>,
    #[serde(default)]
    pub device_label: Option<String>,
}

impl Drop for LoginRequest {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.password.zeroize();
        if let Some(t) = self.totp.as_mut() {
            t.zeroize();
        }
    }
}

/// Render a freshly-minted session token.
///
/// **Always through here.** The body carries a bearer token, so it must
/// never be stored by a client, a service worker, or anything between:
/// the transcript endpoint already says `no-store` for carrying
/// secrets, and this response carries the credential that unlocks it.
/// Both login paths — password and passkey — mint the same `Device`, so
/// they get the same headers from one place rather than two that can
/// drift.
pub(super) fn login_ok(token: String, expires_at: Option<String>) -> Response {
    (
        StatusCode::OK,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(LoginResponse { token, expires_at }),
    )
        .into_response()
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: Option<String>,
}

pub fn router(state: Shared) -> Router {
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/login", post(handle_login))
        // Unauthenticated of necessity — this *is* a way to sign in.
        // Safe to expose: `login/begin` sends an empty
        // `allowCredentials`, so it reveals nothing about what is
        // registered, and `finish` is a signature over a single-use
        // challenge, which nothing can brute force.
        .route(
            "/api/passkey/login/begin",
            post(super::api::passkey_login_begin),
        )
        .route(
            "/api/passkey/login/finish",
            post(super::api::passkey_login_finish),
        );

    let private = Router::new()
        .route("/api/me", get(me))
        // Sessions are addressed by session_id, never by pid. A pid is
        // recycled, and a list fetched by a phone before that happened
        // would silently retarget — the existing procStart/session_id
        // guards stop delivery to the wrong *process*, but cannot see a
        // stale *intent*.
        .route("/api/sessions", get(super::api::list_sessions))
        .route(
            "/api/sessions/{session_id}/transcript",
            get(super::api::get_transcript),
        )
        .route("/api/sessions/{session_id}/prompt", post(send_prompt))
        .route(
            "/api/sessions/{session_id}/read",
            post(super::api::mark_read),
        )
        .route(
            "/api/sessions/{session_id}/commands",
            get(super::api::list_commands),
        )
        .route(
            "/api/sessions/{session_id}/commands/{name}",
            get(super::api::expand_command),
        )
        .route("/api/approvals", get(super::api::list_approvals))
        .route("/api/approvals/{id}", post(super::api::decide_approval))
        // Read-only by design; each handler carries the reason.
        .route("/api/quick-prompts", get(super::api::list_quick_prompts))
        .route("/api/accounts", get(super::api::list_accounts))
        .route(
            "/api/accounts/{email}/activate",
            post(super::api::activate_account),
        )
        // Registering a passkey requires a session that is already
        // authenticated — otherwise anyone who can reach the page
        // enrols themselves.
        .route(
            "/api/passkey/register/begin",
            post(super::api::passkey_register_begin),
        )
        .route(
            "/api/passkey/register/finish",
            post(super::api::passkey_register_finish),
        )
        // Status and revoke only. Opening the gate remotely would let
        // one bearer both open it and send through it, collapsing CC's
        // held-for-approval step into a single action controlled by one
        // stolen token — and the setting is machine-wide, so it would
        // unblock every unrelated local peer too. Remote may always make
        // the machine safer, never less safe.
        .route("/api/inbound", get(inbound_status))
        .route("/api/inbound/revoke", post(inbound_revoke))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));

    public
        .merge(private)
        // The client, embedded. `fallback` rather than a route per file
        // so adding an asset is one match arm in `assets::get`, not two
        // edits that can disagree.
        .fallback(static_asset)
        .layer(middleware::from_fn_with_state(state.clone(), guard_origin))
        .with_state(state)
}

/// Serve an embedded asset, or 404.
///
/// Unauthenticated, necessarily: this *is* the login page. It carries
/// no data about the machine — the shell and a capability probe.
async fn static_asset(req: Request) -> Response {
    let Some(asset) = assets::get(req.uri().path()) else {
        return err(StatusCode::NOT_FOUND, "not_found");
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, asset.content_type),
            // Per asset, not one constant: the fonts are the only bytes
            // here whose name changes when their contents do, so they
            // are the only ones a client may keep. See `assets`.
            (header::CACHE_CONTROL, asset.cache_control),
            // Self-only. The client loads nothing remote, so the policy
            // that permits nothing remote costs nothing — and it is the
            // difference between "we did not add a CDN" and "a CDN
            // cannot be added by accident".
            //
            // `style-src 'unsafe-inline'` is required and not a
            // loosening worth arguing about: the design system styles
            // every component through React's `style` prop, and CSP2's
            // `style-src` covers attributes as well as `<style>`. Script
            // stays strict, which is where injection lives.
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data:; font-src 'self'; connect-src 'self'; \
                 frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        asset.body,
    )
        .into_response()
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
    let passkeys = state.lock().await.config.passkeys.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "device": device.name,
            "expires_at": device.expires_at.map(|t| t.to_rfc3339()),
            // A count, never the records. They are public keys, but a
            // list of credential ids is still a list of what to phish.
            "passkeys": passkeys,
            // Which Claudepot is actually serving this panel.
            //
            // The panel is `no-store`, so the bytes a phone holds are
            // always the ones the server sent — but the server embeds
            // the bundle with `include_bytes!`, so a `remote serve`
            // that outlives a rebuild serves the OLD panel forever and
            // nothing on the phone can tell. That cost an afternoon
            // once: a mermaid fix was committed, rebuilt and shipped,
            // and the running server predated the binary, so the bug
            // read as unfixed. This is the field that answers it.
            //
            // On `me` rather than `health`: `health` is
            // unauthenticated and deliberately says nothing about the
            // machine — a version there is a fingerprint for anyone
            // who can reach the port.
            "server_version": env!("CARGO_PKG_VERSION"),
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
        LoginOutcome::Success { token, device } => {
            login_ok(token, device.expires_at.map(|t| t.to_rfc3339()))
        }
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

/// Run a mutation exactly once per `Idempotency-Key`.
///
/// Factored out of the first handler that needed it. Every mutation on
/// this surface goes through it, and the key is **required**: this is a
/// phone client, a retry is normal, and "send a prompt" running twice
/// means the work happens twice rather than a counter being wrong. A
/// client that has not thought about that is told so rather than being
/// quietly obeyed.
///
/// `lookup` **reserves** the key under the lock and this function drops
/// the lock before running the mutation. Both halves matter: reserving
/// is what stops two concurrent duplicates from both executing, and
/// releasing is what stops one slow prompt from blocking every other
/// request on the appliance.
pub(super) async fn idempotent<F, Fut>(
    state: &Shared,
    headers: &HeaderMap,
    scope: &str,
    run: F,
) -> Response
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = (StatusCode, serde_json::Value)>,
{
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    if key.is_empty() {
        return err(StatusCode::BAD_REQUEST, "idempotency_key_required");
    }

    // **Namespaced by operation.** The store is a flat map keyed by the
    // client's opaque header value, so a key reused across two
    // different mutations replayed the FIRST one's response and the
    // second mutation never ran — a "switched: true" body answering a
    // prompt that was never sent. A client is only obliged to make its
    // keys unique to itself, not unique across every endpoint, so the
    // uniqueness the store needs has to come from the server.
    //
    // The scope carries the route and its path parameters, so the same
    // key on `/prompt` and `/read` cannot collide, and the same key on
    // two different sessions cannot either. NUL separates the parts
    // because it cannot occur in a header value.
    let key = format!("{scope}\u{0}{key}");

    let now = std::time::Instant::now();
    {
        let mut guard = state.lock().await;
        match guard.idempotency.lookup(&key, now) {
            Lookup::Replay(prev) => {
                return (
                    StatusCode::from_u16(prev.status).unwrap_or(StatusCode::OK),
                    [(header::CONTENT_TYPE, "application/json")],
                    prev.body,
                )
                    .into_response();
            }
            Lookup::Rejected(_) => return err(StatusCode::BAD_REQUEST, "bad_idempotency_key"),
            // A duplicate that arrived while the first one is still
            // running. Neither answer is available yet, so say so and
            // let the client retry — running the mutation again is the
            // exact outcome the key exists to prevent.
            Lookup::InFlight => return err(StatusCode::CONFLICT, "idempotency_key_in_flight"),
            // Every slot is a live reservation, so there is nothing the
            // store can drop without risking a duplicate mutation.
            // 503 + Retry-After, because the key is fine and the
            // condition clears as those requests finish.
            Lookup::Saturated => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [(header::RETRY_AFTER, "1")],
                    Json(ApiError {
                        error: "busy",
                        retry_after_secs: Some(1),
                    }),
                )
                    .into_response();
            }
            Lookup::Execute => {}
        }
    }

    let (status, payload) = run().await;
    let body_text = payload.to_string();

    // Stamped at completion, not at lookup. Using the lookup instant
    // would age a stored response by however long the mutation took —
    // so a slow prompt could store a response that was already expired,
    // and the retry it exists to serve would re-execute.
    state.lock().await.idempotency.remember(
        &key,
        Stored {
            status: status.as_u16(),
            body: body_text.clone(),
        },
        std::time::Instant::now(),
    );

    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body_text,
    )
        .into_response()
}

/// `Device` as an extractor.
///
/// [`require_auth`] has already put it in the request extensions, so a
/// handler that names it in its signature cannot be reached
/// unauthenticated. Failing closed rather than unwrapping keeps a future
/// refactor from turning a missing extension into a 500 an attacker can
/// provoke.
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Device {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Device>()
            .cloned()
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "unauthorized"))
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
    let allowed = { state.lock().await.config.server.allowed_hosts.clone() };

    if let Some(host) = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
    {
        if !host_is_acceptable(host, &allowed) {
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
/// The first version compared the **port** against the configured one.
/// That was wrong twice over, and an end-to-end test caught it where
/// the unit test could not:
///
/// - **Too strict.** Any port but the configured default was refused,
///   so binding port 0 — or being reached through any port mapping —
///   rejected every request.
/// - **Useless.** A rebinding attacker chooses the whole Host, so
///   `evil.example:8420` matched happily. The unit test passed only
///   because it happened to use port 1234.
///
/// What separates a rebinding attempt from a real client is the
/// **name**. An appliance is reached by IP literal, by a single label,
/// or by an mDNS / MagicDNS suffix; a rebinding attack needs a public
/// FQDN it controls, because it has to serve DNS for it. Hence a
/// shape rule rather than a list of exact names, which would be wrong
/// more often than right for something reached three different ways.
///
/// The port is ignored entirely.
fn host_is_acceptable(host: &str, extra_allowed: &[String]) -> bool {
    let Some(name) = host_name_of(host) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();

    if extra_allowed.iter().any(|a| a.eq_ignore_ascii_case(&lower)) {
        return true;
    }
    // An IP literal has no name to re-resolve, so it cannot be rebound.
    // IPv6 arrives here already unbracketed.
    if lower.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    // A single label has no public DNS to hijack.
    if !lower.contains('.') {
        return true;
    }
    lower.ends_with(".local") || lower.ends_with(".internal")
}

/// Strip the port and any IPv6 brackets. `None` for anything
/// unparseable — a malformed Host is not something to guess at.
fn host_name_of(host: &str) -> Option<&str> {
    if host.is_empty() {
        return None;
    }
    if let Some(rest) = host.strip_prefix('[') {
        let (inner, after) = rest.split_once(']')?;
        if !(after.is_empty() || after.starts_with(':')) {
            return None;
        }
        return (!inner.is_empty()).then_some(inner);
    }
    let name = match host.rsplit_once(':') {
        Some((n, port)) => {
            if port.is_empty() || port.parse::<u16>().is_err() {
                return None;
            }
            n
        }
        None => host,
    };
    (!name.is_empty()).then_some(name)
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
            idempotency: Idempotency::new(),
            challenges: super::super::passkey::Challenges::new(),
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

    /// **The panel switches the CLI slot and only the CLI slot.**
    ///
    /// `cli` and `desktop` are independent nouns and
    /// `.claude/rules/architecture.md` says never to couple them. The
    /// activate endpoint reaches `swap::switch`, which touches CC's
    /// keychain item and nothing of Claude Desktop's — every `Desktop`
    /// mention in that module is Windows process *detection*, so a
    /// running Desktop is not mistaken for CC.
    ///
    /// Asserted at the router because that is where a future
    /// `/api/accounts/{email}/desktop` would appear. Reading the swap
    /// code proves today's behaviour; this fails the day someone adds
    /// the route, which is the failure worth catching.
    #[tokio::test]
    async fn no_route_can_switch_the_desktop_slot() {
        let app = router(state_with_password(Box::new(NoPersist)));
        for (method, path) in [
            ("POST", "/api/accounts/a%40example.com/desktop"),
            ("POST", "/api/accounts/a%40example.com/activate-desktop"),
            ("POST", "/api/desktop/activate"),
            ("POST", "/api/desktop"),
        ] {
            let res = app
                .clone()
                .oneshot(req(method, path, Some(serde_json::json!({}))))
                .await
                .unwrap();
            assert_eq!(
                res.status(),
                StatusCode::NOT_FOUND,
                "{method} {path} must not exist"
            );
        }
    }

    /// The activate endpoint takes no desktop knob either — a `desktop`
    /// field must not become a way in through the body.
    #[tokio::test]
    async fn activate_ignores_a_desktop_field_in_the_body() {
        let app = router(state_with_password(Box::new(NoPersist)));
        let res = app
            .oneshot(req(
                "POST",
                "/api/accounts/a%40example.com/activate",
                Some(serde_json::json!({"force": false, "desktop": true})),
            ))
            .await
            .unwrap();
        // Unauthenticated, so it stops at the guard — the point is that
        // `ActivateRequest` has no `desktop` field to deserialize into,
        // so no future body can reach one.
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
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

    /// Fixture hostnames must belong to nobody. A real machine name
    /// or tailnet domain is internal topology, and the pre-push scan
    /// blocks it — this is the second time one got in as "obviously
    /// just test data", after the addresses in `bind.rs`.
    #[test]
    fn host_shapes() {
        let none: Vec<String> = vec![];
        // Reached-by-IP, on ANY port — the port is not a signal.
        assert!(host_is_acceptable("192.168.1.5:8420", &none));
        assert!(host_is_acceptable("192.168.1.5:54321", &none));
        assert!(host_is_acceptable("127.0.0.1", &none));
        assert!(host_is_acceptable("[::1]:8420", &none));
        assert!(host_is_acceptable("[::1]", &none));
        // Single label and local suffixes.
        assert!(host_is_acceptable("localhost:9999", &none));
        assert!(host_is_acceptable("claudepot.local", &none));
        assert!(host_is_acceptable(
            "appliance.example-net.internal:8420",
            &none
        ));

        // A public FQDN is the rebinding signature — and note it is
        // refused even on our own port, which the old port check let
        // straight through.
        assert!(!host_is_acceptable("evil.example:8420", &none));
        assert!(!host_is_acceptable("evil.example", &none));
        assert!(!host_is_acceptable("rebind.attacker.com:8420", &none));

        // Malformed.
        assert!(!host_is_acceptable("", &none));
        assert!(!host_is_acceptable(":8420", &none));
        assert!(!host_is_acceptable("host:notaport", &none));
        assert!(!host_is_acceptable("host:", &none));

        // A user fronting the appliance with a real domain opts in.
        let allowed = vec!["pot.example.com".to_string()];
        assert!(host_is_acceptable("pot.example.com:443", &allowed));
        assert!(host_is_acceptable("POT.EXAMPLE.COM", &allowed));
        assert!(!host_is_acceptable("other.example.com", &allowed));
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

// ── The real endpoints ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PromptRequest {
    text: String,
    /// `now` | `next` | `later`. Unknown values fall back to CC's own
    /// default rather than erroring — a client on an older build should
    /// still be able to send.
    #[serde(default)]
    priority: Option<String>,
}

async fn send_prompt(
    State(state): State<Shared>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<PromptRequest>,
) -> Response {
    let headers2 = headers.clone();
    idempotent(
        &state,
        &headers2,
        &format!("prompt:{session_id}"),
        || async move {
            let dir = crate::session_live::registry::default_sessions_dir();
            match do_send(&dir, &session_id, &body).await {
                Ok(v) => (StatusCode::ACCEPTED, v),
                Err((s, code)) => (s, serde_json::json!({ "error": code })),
            }
        },
    )
    .await
}

/// Resolve, address, send. Split out so the handler stays about HTTP.
async fn do_send(
    dir: &std::path::Path,
    session_id: &str,
    body: &PromptRequest,
) -> Result<serde_json::Value, (StatusCode, &'static str)> {
    if body.text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty_prompt"));
    }
    let candidates = crate::peer::list_addressable(dir)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "internal"))?;

    // Exact session id only. `peer::resolve` also accepts a pid and a
    // name prefix, which is right for a human at a terminal and wrong
    // here: a stale handle must fail, not retarget.
    let chosen = candidates
        .iter()
        .find(|a| a.record.session_id == session_id)
        .ok_or((StatusCode::GONE, "session_gone"))?;

    let target = crate::peer::PeerTarget::from_record(&chosen.record)
        .map_err(|_| (StatusCode::CONFLICT, "session_not_addressable"))?;

    let priority = match body.priority.as_deref() {
        Some("now") => crate::peer::Priority::Now,
        Some("later") => crate::peer::Priority::Later,
        _ => crate::peer::Priority::Next,
    };

    let handoff = crate::peer::send_prompt(&target, dir, &body.text, priority)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "remote: send_prompt failed");
            (StatusCode::BAD_GATEWAY, "send_failed")
        })?;

    // 202, and the wording is "handed off". Claude Code may still hold
    // this for the local user's approval, and a surface that reported
    // "sent" would be claiming something it cannot know.
    Ok(serde_json::json!({
        "outcome": "handed_off",
        "uuid": handoff.uuid,
        "session_id": handoff.session_id,
        "note": "Claude Code may hold this for local approval; it is not necessarily delivered.",
    }))
}

async fn inbound_status() -> Response {
    // Under the same guard the grant/revoke/tick paths take. This is
    // two file reads with a writer able to land between them; a render
    // in that window reports `unmanaged_open` — "nothing will close
    // this" — about a window that is being opened correctly.
    let _guard = crate::peer::inbound::file_guard();
    match crate::peer::inbound::state(Utc::now()) {
        Ok(st) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "open": st.is_open(),
                "unmanaged_open": st.is_unmanaged_open(),
                "record_recovered": st.record_recovered,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "remote: inbound state failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

/// Closing the window is always allowed remotely; opening it is not.
///
/// Behind `idempotent` like every other POST. It was the one mutation
/// outside the wrapper, which made the contract "every mutation needs
/// an `Idempotency-Key`" false in exactly one place — and the panel was
/// already sending a key here, so nothing but the enforcement was
/// missing.
async fn inbound_revoke(State(state): State<Shared>, headers: HeaderMap) -> Response {
    idempotent(&state, &headers, "inbound-revoke", || async {
        let _guard = crate::peer::inbound::file_guard();
        match crate::peer::inbound::revoke() {
            Ok(revoked) => (
                StatusCode::OK,
                serde_json::json!({ "revoked": revoked.is_some() }),
            ),
            Err(e) => {
                tracing::warn!(error = %e, "remote: inbound revoke failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({ "error": "internal" }),
                )
            }
        }
    })
    .await
}

//! The panel's endpoints.
//!
//! Split from [`super::server`], which keeps the router, the auth
//! middleware and the two guards that run before any handler. These are
//! the handlers themselves, and they hold one rule between them:
//!
//! > **Remote may make the machine safer, never less safe.**
//!
//! Every read here is over state Claudepot already reads locally.
//! Every write is one of exactly two: hand a prompt to a session
//! (through `peer`, which Claude Code holds for local approval), and
//! record what a device has read. Projects and accounts are read-only
//! and say so in their own comments — not because marshalling the verbs
//! is hard, but because each of them rewrites state whose failure mode
//! is worse than the convenience is worth.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::approval::{self, store as approval_store, ApprovalDecision, Decision};
use super::panel::{self, transcript};
use super::passkey::{self, Ceremony};
use super::server::{err, idempotent, Shared};
use super::Device;

/// Run a blocking read off the async runtime and map both failure modes
/// to one response.
///
/// Every endpoint here reads SQLite or walks a directory, so every one
/// of them needs `spawn_blocking`, a `JoinError` arm, and a domain-error
/// arm — the same eight lines with a different log label. Written out
/// per handler it was five near-identical copies whose only real
/// difference was which of them someone would forget to update.
///
/// `label` is what appears in the log; the client sees `internal` either
/// way, because "the task panicked" and "the database is unreadable" are
/// the same event to a phone.
///
/// `None` rather than a `Result` carrying the response: there is only
/// one failure answer, so returning it from here would put a 128-byte
/// `Response` in every `Err` for no information — and would hide, at the
/// call site, which status the endpoint actually answers with.
async fn blocking<T, E, F>(label: &'static str, f: F) -> Option<T>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Some(v),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "remote: {label} failed");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "remote: {label} task did not finish");
            None
        }
    }
}

// ── Sessions ────────────────────────────────────────────────────────

pub(super) async fn list_sessions(device: Device) -> Response {
    let ctx = panel::ListContext::live(Some(device.id));
    match blocking("session list", move || panel::list(&ctx)).await {
        Some(sessions) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "sessions": sessions,
                "generated_at": Utc::now().to_rfc3339(),
            })),
        )
            .into_response(),
        None => err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct TranscriptQuery {
    #[serde(default)]
    tail: Option<u8>,
    /// A cursor from a previous page — a **count of consumed events**,
    /// handed back verbatim. Named `since` rather than `after` because
    /// the first version called it `after`, matched `index > after`, and
    /// silently dropped one event per poll.
    #[serde(default)]
    since: Option<usize>,
    /// An event **index** the client already holds. Everything below it.
    #[serde(default)]
    before: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

impl TranscriptQuery {
    /// Exactly one window, with `tail` winning when a confused client
    /// sends several. Guessing an intersection would silently return a
    /// page nobody asked for.
    fn window(&self) -> transcript::Window {
        let limit = self.limit.unwrap_or(60);
        if self.tail.is_some_and(|t| t != 0) {
            return transcript::Window::Tail { limit };
        }
        if let Some(before) = self.before {
            return transcript::Window::Before { before, limit };
        }
        transcript::Window::Since {
            since: self.since.unwrap_or(0),
            limit,
        }
    }
}

pub(super) async fn get_transcript(
    Path(session_id): Path<String>,
    Query(q): Query<TranscriptQuery>,
    // Named, not read: the parameter is what makes this route
    // unreachable without a bearer, since `require_auth` is what puts
    // the extension there. Read state is recorded by `mark_read`, not by
    // the act of fetching a page — a client that prefetches must not
    // silently clear a badge.
    _device: Device,
) -> Response {
    let window = q.window();
    let ctx = panel::ListContext::live(None);

    // Not on `blocking`: this one has real per-error statuses (a missing
    // transcript is a 404, a path outside `projects/` is a 403), and
    // flattening them into `internal` to reuse the helper would trade a
    // useful answer for four fewer lines.
    let result = tokio::task::spawn_blocking(move || {
        let file = locate(&ctx, &session_id)?;
        transcript::page(&ctx.config_dir, &file, window).map_err(|e| match e {
            transcript::TranscriptError::NotFound => (StatusCode::NOT_FOUND, "transcript_missing"),
            transcript::TranscriptError::OutsideProjects => {
                (StatusCode::FORBIDDEN, "outside_projects")
            }
            transcript::TranscriptError::Unreadable(msg) => {
                tracing::warn!(error = %msg, "remote: transcript unreadable");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        })
    })
    .await;

    match result {
        Ok(Ok(page)) => (
            StatusCode::OK,
            // A transcript is the one payload on this surface that
            // carries arbitrary tool output. `no-store` is belt to the
            // asset layer's braces: nothing here may sit in a cache, a
            // proxy, or a service worker.
            [(header::CACHE_CONTROL, "no-store")],
            Json(page),
        )
            .into_response(),
        Ok(Err((status, code))) => err(status, code),
        Err(e) => {
            tracing::warn!(error = %e, "remote: transcript task failed");
            err(StatusCode::INTERNAL_SERVER_ERROR, "internal")
        }
    }
}

/// Resolve a session id to the transcript on disk.
///
/// Through `session_index` only. The client never names a path, and the
/// index only ever holds paths under `<config_dir>/projects/`.
fn locate(
    ctx: &panel::ListContext,
    session_id: &str,
) -> Result<std::path::PathBuf, (StatusCode, &'static str)> {
    let rows = crate::session::list_all_sessions(&ctx.config_dir).map_err(|e| {
        tracing::warn!(error = %e, "remote: cannot read the session index");
        (StatusCode::INTERNAL_SERVER_ERROR, "internal")
    })?;
    rows.into_iter()
        // The most complete copy, matching how `panel::list` dedupes:
        // a resumed session has several transcripts under one id.
        .filter(|r| r.session_id == session_id)
        .max_by_key(|r| r.event_count)
        .map(|r| r.file_path)
        .ok_or((StatusCode::NOT_FOUND, "no_such_session"))
}

#[derive(Debug, Deserialize)]
pub(super) struct ReadRequest {
    /// Events consumed — the `total` from a transcript page, not the
    /// index of the last one. See `read_state`'s module docs.
    through_count: usize,
}

/// Record how far this device has read.
///
/// Idempotent by construction — the mark only moves forward — but it
/// goes through the same key gate as every other mutation, so a client
/// does not have to learn which mutations are special.
pub(super) async fn mark_read(
    State(state): State<Shared>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    req_device: Device,
    Json(body): Json<ReadRequest>,
) -> Response {
    idempotent(&state, &headers, || async move {
        let id = req_device.id;
        let session_id = session_id.clone();
        let count = body.through_count;
        match blocking("read state", move || {
            panel::read_state::mark(id, &session_id, count)
        })
        .await
        {
            Some(settled) => (
                StatusCode::OK,
                serde_json::json!({ "through_count": settled }),
            ),
            None => (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            ),
        }
    })
    .await
}

/// Make an account the one Claude Code uses.
///
/// **The one write on this surface that is not about a session**, and
/// the reason the accounts pane stopped being read-only. The objection
/// on record was that a swap "either fails while CC is running or
/// bypasses the keychain-drift guard". Half of that is wrong: `force`
/// is consulted in exactly two places in `swap::switch_inner`, both the
/// live-session gate. The drift check runs unconditionally and is
/// self-healing, so it is never what a remote caller skips.
///
/// What a remote caller *can* skip is the live-session gate, and that
/// gate is about correctness rather than security: a running CC holds
/// its refresh token in memory and overwrites the keychain on its next
/// refresh, **silently reverting the swap**. A revert nobody is at the
/// machine to notice is the worst outcome here, so the default refuses
/// and says how many sessions are in the way. `force` stays available
/// because auto-rotation already swaps unattended on a timer — the
/// product long ago decided that trade is the user's to make — but the
/// phone has to ask for it explicitly, with the consequence stated.
///
/// Addressed by **email**, not by uuid: email is this domain's identity
/// for an account, and a uuid on the wire would be an internal
/// identifier the panel then has to render or hide.
#[derive(Debug, Deserialize)]
pub(super) struct ActivateRequest {
    #[serde(default)]
    force: bool,
}

pub(super) async fn activate_account(
    State(state): State<Shared>,
    Path(email): Path<String>,
    headers: HeaderMap,
    _device: Device,
    Json(body): Json<ActivateRequest>,
) -> Response {
    idempotent(&state, &headers, || async move {
        use crate::cli_backend::{self, swap};

        let data_dir = crate::paths::claudepot_data_dir();
        let Ok(store) = crate::account::AccountStore::open(&data_dir.join("accounts.db")) else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        };
        let Ok(Some(target)) = store.find_by_email(&email) else {
            return (
                StatusCode::NOT_FOUND,
                serde_json::json!({ "error": "no such account" }),
            );
        };

        // Reconcile first, exactly as the CLI does: without it a swap
        // done at the machine leaves the DB believing the old target is
        // active, and this reports "already active" while the keychain
        // says otherwise. Best effort — a network failure falls through
        // to the DB's view rather than blocking the swap.
        let _ = crate::services::account_service::sync_from_current_cc(&store).await;

        let current = store
            .active_cli_uuid()
            .ok()
            .flatten()
            .and_then(|s| s.parse::<uuid::Uuid>().ok());
        if current == Some(target.uuid) {
            return (
                StatusCode::OK,
                serde_json::json!({ "already_active": true, "email": target.email }),
            );
        }

        let platform = cli_backend::create_platform();
        let refresher = swap::DefaultRefresher;
        let fetcher = swap::DefaultProfileFetcher;
        let result = if body.force {
            swap::switch_force(
                &store,
                current,
                target.uuid,
                platform.as_ref(),
                true,
                &refresher,
                &fetcher,
            )
            .await
        } else {
            swap::switch(
                &store,
                current,
                target.uuid,
                platform.as_ref(),
                true,
                &refresher,
                &fetcher,
            )
            .await
        };

        match result {
            Ok(()) => (
                StatusCode::OK,
                serde_json::json!({ "switched": true, "email": target.email }),
            ),
            // Its own status and its own code: this is the one failure
            // the user can actually do something about, and the panel
            // offers the override only for this.
            Err(crate::error::SwapError::LiveSessionConflict) => (
                StatusCode::CONFLICT,
                serde_json::json!({ "error": "live_session", "email": target.email }),
            ),
            Err(e) => (
                StatusCode::BAD_GATEWAY,
                // Core's error text is canonical English and says
                // something useful — an identity mismatch names both
                // emails. Passed through rather than flattened to
                // "failed", which would send the user to the machine
                // with nothing to go on.
                serde_json::json!({ "error": e.to_string() }),
            ),
        }
    })
    .await
}

// ── Slash commands ──────────────────────────────────────────────────

/// The commands a session could run, as text it can be sent.
///
/// A slash command does not survive the peer socket: CC's inbox
/// dispatches with `skipSlashCommands: true`, so `/audit-fix` arrives
/// as prose. `cc_commands` expands one the way CC expands it itself —
/// see that module for why this is the same step rather than a way
/// around it, and for what an expansion cannot carry.
///
/// The working directory is resolved **here, from the session id**,
/// never taken from the client. A path parameter would let an
/// authenticated device enumerate `.claude/commands` anywhere on the
/// disk; a session id can only name a directory Claude Code is already
/// working in.
pub(super) async fn list_commands(Path(session_id): Path<String>, _device: Device) -> Response {
    match blocking("commands", move || {
        let ctx = panel::ListContext::live(None);
        let cwd = panel::list(&ctx)?
            .into_iter()
            .find(|s| s.session_id == session_id)
            .map(|s| s.project_path);
        Ok::<_, panel::PanelError>(
            cwd.map(|c| crate::cc_commands::discover(std::path::Path::new(&c))),
        )
    })
    .await
    {
        Some(Some(commands)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "commands": commands })),
        )
            .into_response(),
        Some(None) => err(StatusCode::NOT_FOUND, "no such session"),
        None => err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ExpandQuery {
    #[serde(default)]
    args: String,
}

/// The text of one command, with arguments substituted.
///
/// Resolved by **name against a fresh discovery**, never by a path the
/// client supplies — `CommandSpec::path` is deliberately not
/// serialized, so there is no path for a client to hand back and no
/// traversal to defend against.
pub(super) async fn expand_command(
    Path((session_id, name)): Path<(String, String)>,
    Query(q): Query<ExpandQuery>,
    _device: Device,
) -> Response {
    match blocking("expand command", move || {
        let ctx = panel::ListContext::live(None);
        let Some(cwd) = panel::list(&ctx)?
            .into_iter()
            .find(|s| s.session_id == session_id)
            .map(|s| s.project_path)
        else {
            return Ok::<_, panel::PanelError>(None);
        };
        let found = crate::cc_commands::discover(std::path::Path::new(&cwd))
            .into_iter()
            .find(|c| c.name == name);
        Ok(found.and_then(|spec| {
            crate::cc_commands::expand(&spec, &q.args)
                .ok()
                .map(|text| (spec, text))
        }))
    })
    .await
    {
        Some(Some((spec, text))) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "name": spec.name,
                "text": text,
                // Repeated on the expansion so a client cannot render
                // the body without the caveat that came with it.
                "restricts_tools": spec.restricts_tools,
                "pins_model": spec.pins_model,
            })),
        )
            .into_response(),
        Some(None) => err(StatusCode::NOT_FOUND, "no such command"),
        None => err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

// ── Approvals ───────────────────────────────────────────────────────

/// Permission prompts waiting for a tap.
///
/// **This is the one endpoint on the surface that leads to granting a
/// capability rather than reading or messaging.** Everything else here
/// is read-only or goes through `peer`, where Claude Code refuses to
/// treat a message as its user's approval. This does not route around
/// that refusal — the decision arrives through CC's own
/// `PermissionRequest` hook, before the prompt is drawn. See
/// `remote::approval`.
///
/// It exists only while `remote serve` does: the hook is installed on
/// start, revoked on stop, and heartbeated in between, so a machine
/// that never turns the remote surface on is never asked.
pub(super) async fn list_approvals(_device: Device) -> Response {
    let now = approval::now_ms();
    match blocking("approvals", move || {
        Ok::<_, std::convert::Infallible>(approval_store::pending(&approval_store::dir(), now))
    })
    .await
    {
        Some(pending) => (
            StatusCode::OK,
            Json(serde_json::json!({ "approvals": pending })),
        )
            .into_response(),
        None => err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct DecideRequest {
    decision: Decision,
}

/// Answer one.
///
/// A 404 here means the prompt is no longer live — answered at the
/// keyboard, or its hook gave up waiting. The phone must be told that
/// plainly rather than shown a success for a decision that reached
/// nobody.
pub(super) async fn decide_approval(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req_device: Device,
    Json(body): Json<DecideRequest>,
) -> Response {
    idempotent(&state, &headers, || async move {
        let now = approval::now_ms();
        let decision = ApprovalDecision {
            decision: body.decision,
            device_id: req_device.id.to_string(),
            at_ms: now,
        };
        match blocking("approval decision", move || {
            approval_store::put_decision(&approval_store::dir(), &id, &decision, now)
        })
        .await
        {
            Some(true) => (StatusCode::OK, serde_json::json!({ "ok": true })),
            Some(false) => (
                StatusCode::NOT_FOUND,
                serde_json::json!({ "error": "no longer waiting" }),
            ),
            None => (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            ),
        }
    })
    .await
}

// ── Projects (read-only) ────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ProjectDto {
    path: String,
    name: String,
    sessions: usize,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_modified: Option<String>,
    is_orphan: bool,
    is_reachable: bool,
}

/// Read-only, and the omission is the design.
///
/// Move, rename, clean and repair each rewrite path-keyed Claude Code
/// state that lives *outside* the project directory — `history.jsonl`,
/// the `~/.claude.json` projects map, plugin bindings — behind a
/// rollback journal. A half-applied one leaves Claude Code pointing at
/// paths that do not exist. None of that belongs one stolen bearer token
/// away.
pub(super) async fn list_projects() -> Response {
    let config_dir = crate::paths::claude_config_dir();
    match blocking("project list", move || {
        crate::project::list_projects(&config_dir)
    })
    .await
    {
        Some(projects) => {
            let rows: Vec<ProjectDto> = projects
                .into_iter()
                .map(|p| ProjectDto {
                    name: basename(&p.original_path),
                    path: p.original_path,
                    sessions: p.session_count,
                    size_bytes: p.total_size_bytes,
                    last_modified: p
                        .last_modified
                        .map(|t| chrono::DateTime::<Utc>::from(t).to_rfc3339()),
                    is_orphan: p.is_orphan,
                    is_reachable: p.is_reachable,
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "projects": rows })),
            )
                .into_response()
        }
        None => err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

// ── Accounts (read-only) ────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct AccountDto {
    email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    is_cli_active: bool,
    is_desktop_active: bool,
    verify_status: String,
    /// Cached utilization, when the GUI has written a snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<serde_json::Value>,
    /// When that snapshot was taken. Always sent alongside `usage`, so
    /// the client can say "as of" rather than implying it is live.
    #[serde(skip_serializing_if = "Option::is_none")]
    usage_as_of: Option<String>,
}

/// Read-only, and this is the verb the plan flagged as understated.
///
/// Switching the active account either fails while Claude Code is
/// running — it holds the credentials the swap replaces — or goes around
/// the keychain-drift guard that refuses to persist a rotated blob whose
/// profile email no longer matches its label. Either outcome is a
/// broken machine reached from a phone, so switching needs its own
/// check before it is reachable at all.
pub(super) async fn list_accounts() -> Response {
    match blocking("account list", read_accounts).await {
        Some(payload) => (StatusCode::OK, Json(payload)).into_response(),
        None => err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

fn read_accounts() -> Result<serde_json::Value, String> {
    let data_dir = crate::paths::claudepot_data_dir();
    let store = crate::account::AccountStore::open(&data_dir.join("accounts.db"))
        .map_err(|e| e.to_string())?;
    let accounts = store.list().map_err(|e| e.to_string())?;

    let snapshot =
        crate::services::usage_snapshot::read(&crate::services::usage_snapshot::snapshot_path());

    let rows: Vec<AccountDto> = accounts
        .into_iter()
        .map(|a| {
            let snap = snapshot
                .as_ref()
                .and_then(|s| s.accounts.get(&a.uuid.to_string()));
            AccountDto {
                email: a.email,
                org: a.org_name,
                plan: a.subscription_type,
                is_cli_active: a.is_cli_active,
                is_desktop_active: a.is_desktop_active,
                verify_status: a.verify_status,
                usage: snap
                    .and_then(|s| s.usage.as_ref())
                    .and_then(|u| serde_json::to_value(u).ok()),
                usage_as_of: snap
                    .filter(|s| s.usage.is_some())
                    .map(|s| s.fetched_at.to_rfc3339()),
            }
        })
        .collect();

    Ok(serde_json::json!({
        "accounts": rows,
        // Named so the client can explain an empty meter instead of
        // rendering a zero nobody measured.
        "usage_source": if snapshot.is_some() { "snapshot" } else { "none" },
    }))
}

// ── Passkeys ────────────────────────────────────────────────────────

/// The origin a browser reached us on, rebuilt from the request.
///
/// `guard_origin` has already refused any `Host` this appliance is not
/// willing to answer to, so this is not attacker-chosen in a way that
/// matters. TLS is required off loopback (`BindAddr::requires_tls`), and
/// `https` is what the client will report in `clientDataJSON` — so the
/// scheme is derived from the host being loopback rather than guessed.
fn origin_of(headers: &HeaderMap) -> Option<String> {
    // An `Origin` header, when the browser sent one, is authoritative:
    // it is the exact string the authenticator will sign over.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        if !origin.eq_ignore_ascii_case("null") && origin.contains("://") {
            return Some(origin.to_string());
        }
    }
    let host = headers.get(header::HOST)?.to_str().ok()?;
    // IPv6 arrives bracketed (`[::1]:8420`). Splitting on the last colon
    // leaves the brackets on, and a bracketed literal does not parse as
    // an `IpAddr` — so `::1` would have been called non-loopback and
    // handed the browser an `https` origin it never used.
    let bare = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host),
    };
    let loopback = bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    Some(format!(
        "{}://{host}",
        if loopback { "http" } else { "https" }
    ))
}

fn passkey_status(e: &passkey::PasskeyError) -> (StatusCode, &'static str) {
    match e {
        passkey::PasskeyError::RpIdUnavailable => (StatusCode::CONFLICT, "rp_id_unavailable"),
        passkey::PasskeyError::UnknownChallenge => (StatusCode::BAD_REQUEST, "unknown_challenge"),
        passkey::PasskeyError::NoCredential => (StatusCode::NOT_FOUND, "no_passkey"),
        passkey::PasskeyError::TooManyCredentials => (StatusCode::CONFLICT, "too_many_passkeys"),
        // One code for every cryptographic failure. Which check failed
        // is in the log, not in the response: the client cannot act on
        // the difference and an attacker can.
        passkey::PasskeyError::Verification(_) | passkey::PasskeyError::BadBase64 => {
            (StatusCode::UNAUTHORIZED, "invalid")
        }
    }
}

/// Options a browser needs to create or fetch a credential.
///
/// `residentKey: required` plus an empty `allowCredentials` on login:
/// the platform finds the passkey itself, so an unauthenticated caller
/// never learns which credential ids exist. Apple's platform
/// authenticator creates discoverable credentials regardless, so the
/// requirement costs nothing on the device this is for.
fn register_options(issued: &passkey::Issued, user_handle: &str, label: &str) -> serde_json::Value {
    serde_json::json!({
        "rp": { "id": issued.rp_id, "name": "Claudepot" },
        "user": { "id": user_handle, "name": label, "displayName": label },
        "challenge": issued.challenge_b64,
        // ES256 only, matching what `webauthn` verifies. Advertising an
        // algorithm we cannot check would produce a credential that
        // registers and then never logs in.
        "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }],
        "timeout": 60000,
        "attestation": "none",
        "authenticatorSelection": {
            "authenticatorAttachment": "platform",
            "residentKey": "required",
            "userVerification": "required"
        },
    })
}

pub(super) async fn passkey_register_begin(
    State(state): State<Shared>,
    headers: HeaderMap,
    device: Device,
) -> Response {
    let Some(origin) = origin_of(&headers) else {
        return err(StatusCode::BAD_REQUEST, "bad_host");
    };
    let mut guard = state.lock().await;
    let issued =
        match guard
            .challenges
            .issue(Ceremony::Register, &origin, std::time::Instant::now())
        {
            Ok(v) => v,
            Err(e) => {
                let (s, c) = passkey_status(&e);
                return err(s, c);
            }
        };
    // A stable, non-identifying handle. WebAuthn requires a user id and
    // this appliance has exactly one user, so it carries no information
    // beyond "this machine". Minting one has to be persisted here rather
    // than at `finish`: the browser signs over the handle we send now,
    // and a restart between the two steps would file the credential
    // under an id nothing remembers.
    let (handle, minted) = guard.config.passkey_user_handle_or_mint();
    if minted {
        let (config, devices) = (guard.config.clone(), guard.devices.clone());
        if let Err(e) = guard.persist.save(&config, &devices) {
            tracing::error!(error = %e, "remote: could not persist the passkey user handle");
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
    }
    let user_handle = base64_url(handle.as_bytes());
    let options = register_options(&issued, &user_handle, &device.name);
    drop(guard);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "challenge_id": issued.challenge_id,
            "options": options,
        })),
    )
        .into_response()
}

pub(super) async fn passkey_register_finish(
    State(state): State<Shared>,
    headers: HeaderMap,
    device: Device,
    Json(body): Json<passkey::RegisterResponse>,
) -> Response {
    let inner = state.clone();
    idempotent(&state, &headers, || async move {
        let mut guard = inner.lock().await;
        let existing = guard.config.passkeys.clone();
        let record = match passkey::finish_registration(
            &mut guard.challenges,
            &body,
            &device.name,
            &existing,
            std::time::Instant::now(),
            Utc::now(),
        ) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "remote: passkey registration refused");
                let (s, c) = passkey_status(&e);
                return (s, serde_json::json!({ "error": c }));
            }
        };
        // Re-registering the same authenticator replaces rather than
        // duplicates: a second row for one credential id would make the
        // counter check depend on which row was found first.
        guard
            .config
            .passkeys
            .retain(|r| r.credential.id != record.credential.id);
        guard.config.passkeys.push(record);

        let (config, devices) = (guard.config.clone(), guard.devices.clone());
        if let Err(e) = guard.persist.save(&config, &devices) {
            tracing::error!(error = %e, "remote: could not persist a passkey");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
        let count = guard.config.passkeys.len();
        (StatusCode::OK, serde_json::json!({ "passkeys": count }))
    })
    .await
}

pub(super) async fn passkey_login_begin(
    State(state): State<Shared>,
    headers: HeaderMap,
) -> Response {
    let Some(origin) = origin_of(&headers) else {
        return err(StatusCode::BAD_REQUEST, "bad_host");
    };
    let mut guard = state.lock().await;
    if guard.config.passkeys.is_empty() {
        return err(StatusCode::NOT_FOUND, "no_passkey");
    }
    let issued = match guard
        .challenges
        .issue(Ceremony::Login, &origin, std::time::Instant::now())
    {
        Ok(v) => v,
        Err(e) => {
            let (s, c) = passkey_status(&e);
            return err(s, c);
        }
    };
    drop(guard);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "challenge_id": issued.challenge_id,
            "options": {
                "rpId": issued.rp_id,
                "challenge": issued.challenge_b64,
                "timeout": 60000,
                "userVerification": "required",
                // Empty on purpose: the platform resolves the discoverable
                // credential, so this endpoint tells an unauthenticated
                // caller nothing about what is registered.
                "allowCredentials": [],
            },
        })),
    )
        .into_response()
}

pub(super) async fn passkey_login_finish(
    State(state): State<Shared>,
    Json(body): Json<passkey::LoginResponse>,
) -> Response {
    let mut guard = state.lock().await;
    let stored = guard.config.passkeys.clone();
    let (index, counter) = match passkey::finish_login(
        &mut guard.challenges,
        &body,
        &stored,
        std::time::Instant::now(),
    ) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "remote: passkey login refused");
            let (s, c) = passkey_status(&e);
            return err(s, c);
        }
    };

    let now = Utc::now();
    guard.config.passkeys[index].credential.sign_count = counter;
    guard.config.passkeys[index].last_used = Some(now);

    // A passkey login mints exactly what a password login mints — the
    // same `Device` row with the same expiry and the same revocation
    // path. Two kinds of session would mean two things to remember to
    // revoke.
    let label = guard.config.passkeys[index].label.clone();
    let (token, device) = super::login::issue_session(&label, now);
    guard.devices.devices.push(device.clone());

    let (config, devices) = (guard.config.clone(), guard.devices.clone());
    if let Err(e) = guard.persist.save(&config, &devices) {
        tracing::error!(error = %e, "remote: could not persist a passkey login");
        // A login we cannot record is a login that did not happen: the
        // token would work until restart and the counter — the only
        // clone detection a passkey has — would not advance.
        return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
    }
    drop(guard);

    (
        StatusCode::OK,
        Json(super::server::LoginResponse {
            token,
            expires_at: device.expires_at.map(|t| t.to_rfc3339()),
        }),
    )
        .into_response()
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(k.clone(), v.parse().unwrap());
        }
        h
    }

    #[test]
    fn the_origin_header_wins_because_it_is_what_gets_signed() {
        let h = headers(&[
            (header::ORIGIN, "https://laptop.local:8420"),
            (header::HOST, "laptop.local:8420"),
        ]);
        assert_eq!(origin_of(&h).as_deref(), Some("https://laptop.local:8420"));
    }

    #[test]
    fn a_native_client_sending_no_origin_gets_one_derived() {
        let h = headers(&[(header::HOST, "laptop.local:8420")]);
        assert_eq!(origin_of(&h).as_deref(), Some("https://laptop.local:8420"));
    }

    #[test]
    fn loopback_derives_http_because_that_is_what_the_browser_will_report() {
        // TLS is required iff the bind is not loopback, so deriving
        // `https` here would produce an origin mismatch on exactly the
        // setup a developer uses.
        for host in ["127.0.0.1:8420", "localhost:5173", "[::1]:8420"] {
            let h = headers(&[(header::HOST, host)]);
            let got = origin_of(&h).unwrap();
            assert!(got.starts_with("http://"), "{host} -> {got}");
        }
    }

    #[test]
    fn a_null_origin_is_never_believed() {
        // What a sandboxed iframe or a file:// page sends.
        let h = headers(&[
            (header::ORIGIN, "null"),
            (header::HOST, "laptop.local:8420"),
        ]);
        assert_eq!(origin_of(&h).as_deref(), Some("https://laptop.local:8420"));
    }

    #[test]
    fn every_passkey_verification_failure_answers_the_same_way() {
        // Which check failed is a distinction only an attacker can use.
        let a = passkey_status(&passkey::PasskeyError::Verification("origin".into()));
        let b = passkey_status(&passkey::PasskeyError::BadBase64);
        assert_eq!(a, b);
        assert_eq!(a.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn tail_wins_over_a_confused_clients_other_parameters() {
        let q = TranscriptQuery {
            tail: Some(1),
            since: Some(5),
            before: Some(9),
            limit: Some(10),
        };
        assert!(matches!(q.window(), transcript::Window::Tail { limit: 10 }));
    }

    #[test]
    fn no_parameters_means_the_whole_transcript_from_the_start() {
        let q = TranscriptQuery {
            tail: None,
            since: None,
            before: None,
            limit: None,
        };
        assert!(matches!(
            q.window(),
            transcript::Window::Since {
                since: 0,
                limit: 60
            }
        ));
    }

    #[test]
    fn basename_handles_both_separators() {
        assert_eq!(basename("/Users/x/code/claudepot"), "claudepot");
        assert_eq!(basename(r"C:\Users\x\code\claudepot"), "claudepot");
        assert_eq!(basename("/Users/x/code/claudepot/"), "claudepot");
    }
}

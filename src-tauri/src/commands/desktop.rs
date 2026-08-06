//! Tauri commands for Claude Desktop: switch, identity probes, adopt,
//! clear, sync, launch.
//!
//! `current_desktop_identity` is the cheap fast-path probe (fast enough
//! to run on every tick). `verified_desktop_identity` runs the slow
//! Decrypted path and is the only method whose email the UI can trust
//! for mutation. Plan v2 §D6 / §VerifiedIdentity.
//!
//! Errors use the `ErrorDto` shape (i18n plan §2.5; `commands/keys.rs`
//! is the reference). `DesktopSwapError` and `DesktopIdentityError`
//! carry their own codes, so those lift with `ErrorDto::from`. The
//! `services::desktop_service` errors (`SwitchError` / `AdoptError` /
//! `ClearError`) do not yet, so they ride a command code with core's
//! own English in `message` — promote them to `from_core` when the
//! services slice converts them.

use super::open_account_store;
use crate::dto;
use crate::dto_error::{codes, ErrorDto};
use claudepot_core::desktop_backend;
use claudepot_core::services;
use serde_json::json;
use uuid::Uuid;

/// `create_platform()` returned `None`. Three call sites share one
/// English string already; this keeps them sharing one code.
fn desktop_unsupported() -> ErrorDto {
    ErrorDto::new(
        codes::DESKTOP_NOT_SUPPORTED_ON_PLATFORM,
        "Desktop not supported on this platform",
    )
}

#[tauri::command]
pub async fn desktop_use(
    email: String,
    no_launch: bool,
    lock: tauri::State<'_, crate::state::DesktopOpState>,
) -> Result<(), ErrorDto> {
    // Codex follow-up review: desktop_use was bypassing the operation
    // lock, letting switch race with adopt/clear across GUI + tray +
    // CLI. The async mutex guards in-process; the core flock guards
    // cross-process (CLI vs GUI running simultaneously).
    let _guard = lock.0.lock().await;

    let store = open_account_store()?;
    // `resolve_target` lives in `commands/mod.rs` and still composes its
    // own English ("resolve failed: …" / "account not found: …"). Not
    // this file's prefix to drop, so it rides through unchanged.
    let target = super::resolve_target(&store, &email)
        .map_err(|m| ErrorDto::detail(codes::DESKTOP_RESOLVE_TARGET_FAILED, m))?;

    let platform = desktop_backend::create_platform().ok_or_else(desktop_unsupported)?;

    services::desktop_service::switch(&*platform, &store, target.uuid, no_launch)
        .await
        .map(|_| ())
        .map_err(ErrorDto::from)
}

/// Ground-truth "who is Claude Desktop signed in as right now".
///
/// Mirrors [`crate::commands::current_cc_identity`]: reads the live
/// data dir, probes the signed-in identity, returns a DTO that never
/// fails at the Tauri boundary — failures ride in `error` so the UI
/// can render visible banners.
///
/// Phase 1: returns only the fast-path ("OrgUuidCandidate") result or
/// `None`. The `probe_method` field is the UI's trust gate — only
/// `Decrypted` (Phase 2+) is authoritative for mutation. See
/// `desktop_identity` module docs for the rationale.
#[tauri::command]
pub async fn current_desktop_identity() -> Result<dto::DesktopIdentity, ErrorDto> {
    let now = chrono::Utc::now();
    let Some(platform) = desktop_backend::create_platform() else {
        return Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: Some("Desktop not supported on this platform".to_string()),
        });
    };
    let store = open_account_store()?;

    match claudepot_core::desktop_identity::probe_live_identity(
        &*platform,
        &store,
        claudepot_core::desktop_identity::ProbeOptions::default(),
    ) {
        Ok(None) => Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: None,
        }),
        Ok(Some(live)) => Ok(dto::DesktopIdentity {
            email: Some(live.email),
            org_uuid: Some(live.org_uuid),
            probe_method: match live.probe_method {
                claudepot_core::desktop_identity::ProbeMethod::OrgUuidCandidate => {
                    dto::DesktopProbeMethod::OrgUuidCandidate
                }
                claudepot_core::desktop_identity::ProbeMethod::Decrypted => {
                    dto::DesktopProbeMethod::Decrypted
                }
            },
            verified_at: now,
            error: None,
        }),
        Err(e) => Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: Some(e.to_string()),
        }),
    }
}

/// Strict Desktop identity probe — runs the async Decrypted path
/// (`probe_live_identity_async` with `strict=true`) so callers that
/// mutate disk or DB (Bind, switch) can trust the returned email.
/// Fast-path [`current_desktop_identity`] is intentionally NOT a
/// valid source for those actions because it returns
/// `OrgUuidCandidate` only and the UI must not light up
/// mutation affordances from candidates.
///
/// Note the two shapes of failure this command reports, which are not
/// interchangeable: opening `accounts.db` rejects the call, while a
/// *probe* that fails resolves `Ok` with the text in `error` — the card
/// needs to keep rendering its identity row either way.
#[tauri::command]
pub async fn verified_desktop_identity() -> Result<dto::DesktopIdentity, ErrorDto> {
    let now = chrono::Utc::now();
    let Some(platform) = desktop_backend::create_platform() else {
        return Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: Some("Desktop not supported on this platform".to_string()),
        });
    };
    let store = open_account_store()?;
    let fetcher = claudepot_core::desktop_identity::DefaultProfileFetcher;

    match claudepot_core::desktop_identity::probe_live_identity_async(
        &*platform,
        &store,
        claudepot_core::desktop_identity::ProbeOptions { strict: true },
        &fetcher,
    )
    .await
    {
        Ok(None) => Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: None,
        }),
        Ok(Some(live)) => Ok(dto::DesktopIdentity {
            email: Some(live.email),
            org_uuid: Some(live.org_uuid),
            probe_method: match live.probe_method {
                claudepot_core::desktop_identity::ProbeMethod::OrgUuidCandidate => {
                    dto::DesktopProbeMethod::OrgUuidCandidate
                }
                claudepot_core::desktop_identity::ProbeMethod::Decrypted => {
                    dto::DesktopProbeMethod::Decrypted
                }
            },
            verified_at: now,
            error: None,
        }),
        Err(e) => Ok(dto::DesktopIdentity {
            email: None,
            org_uuid: None,
            probe_method: dto::DesktopProbeMethod::None,
            verified_at: now,
            error: Some(e.to_string()),
        }),
    }
}

/// Adopt the live Desktop session into `uuid`'s snapshot directory.
/// Verifies the live identity via the slow-path probe before mutating
/// anything — per plan v2 §D6+§VerifiedIdentity, fast-path candidate
/// identities cannot drive adoption.
#[tauri::command]
pub async fn desktop_adopt(
    uuid: String,
    overwrite: bool,
    lock: tauri::State<'_, crate::state::DesktopOpState>,
    app: tauri::AppHandle,
) -> Result<dto::DesktopAdoptOutcome, ErrorDto> {
    use tauri::Emitter;

    let _guard = lock.0.lock().await;

    let target_uuid = Uuid::parse_str(&uuid).map_err(|e| {
        ErrorDto::with_params(
            codes::ACCOUNTS_INVALID_UUID,
            json!({ "uuid": uuid }),
            format!("bad uuid: {e}"),
        )
    })?;
    let store = open_account_store()?;
    let platform =
        claudepot_core::desktop_backend::create_platform().ok_or_else(desktop_unsupported)?;

    // Verify identity: the authoritative Decrypted path. Fails here
    // if the live session isn't signed in, if the keychain secret
    // can't be read, or if /profile rejects the token.
    // `DesktopIdentityError` carries its own code — the "identity probe
    // failed: " prefix this site used to add is the UI's to write.
    let verified = claudepot_core::desktop_identity::verify_live_identity(&*platform, &store)
        .await
        .map_err(ErrorDto::from)?
        .ok_or_else(|| {
            ErrorDto::new(
                codes::DESKTOP_NO_LIVE_IDENTITY,
                "no live Desktop identity — sign in via Desktop first",
            )
        })?;

    let outcome = services::desktop_service::adopt_current(
        &*platform,
        &store,
        target_uuid,
        &verified,
        overwrite,
    )
    .await
    .map_err(ErrorDto::from)?;

    let _ = app.emit(crate::events::DESKTOP_ADOPTED, &outcome.account_email);
    Ok(dto::DesktopAdoptOutcome {
        account_email: outcome.account_email,
        captured_items: outcome.captured_items,
        size_bytes: outcome.size_bytes,
    })
}

/// Sign Desktop out. Stashes the live session into the active
/// account's snapshot dir by default (`keep_snapshot=true`) so the
/// user can swap back in later.
#[tauri::command]
pub async fn desktop_clear(
    keep_snapshot: bool,
    lock: tauri::State<'_, crate::state::DesktopOpState>,
    app: tauri::AppHandle,
) -> Result<dto::DesktopClearOutcome, ErrorDto> {
    use tauri::Emitter;

    let _guard = lock.0.lock().await;

    let store = open_account_store()?;
    let platform =
        claudepot_core::desktop_backend::create_platform().ok_or_else(desktop_unsupported)?;

    let outcome = services::desktop_service::clear_session(&*platform, &store, keep_snapshot)
        .await
        .map_err(ErrorDto::from)?;

    let _ = app.emit(crate::events::DESKTOP_CLEARED, &outcome.email);
    Ok(dto::DesktopClearOutcome {
        email: outcome.email,
        snapshot_kept: outcome.snapshot_kept,
        items_deleted: outcome.items_deleted,
    })
}

/// Startup/window-focus sync. Never mutates the filesystem — at most
/// caches the `active_desktop` pointer when the live identity maps to
/// a registered account that already has a snapshot. UI subscribes
/// to the returned `DesktopSyncOutcome` variants (AdoptionAvailable,
/// Stranger, CandidateOnly) to surface banners.
#[tauri::command]
pub async fn sync_from_current_desktop(
    lock: tauri::State<'_, crate::state::DesktopOpState>,
) -> Result<dto::DesktopSyncOutcome, ErrorDto> {
    let _guard = lock.0.lock().await;

    let store = open_account_store()?;
    let platform = match claudepot_core::desktop_backend::create_platform() {
        Some(p) => p,
        None => return Ok(dto::DesktopSyncOutcome::NoLive),
    };
    // `sync_from_current` returns `DesktopIdentityError`, which carries
    // its own code; the "sync failed: " framing belongs to the UI.
    let outcome = services::desktop_service::sync_from_current(&*platform, &store)
        .await
        .map_err(ErrorDto::from)?;
    Ok(match outcome {
        services::desktop_service::SyncOutcome::NoLive => dto::DesktopSyncOutcome::NoLive,
        services::desktop_service::SyncOutcome::Verified { email } => {
            dto::DesktopSyncOutcome::Verified { email }
        }
        services::desktop_service::SyncOutcome::AdoptionAvailable { email } => {
            dto::DesktopSyncOutcome::AdoptionAvailable { email }
        }
        services::desktop_service::SyncOutcome::Stranger { email } => {
            dto::DesktopSyncOutcome::Stranger { email }
        }
        services::desktop_service::SyncOutcome::CandidateOnly { email } => {
            dto::DesktopSyncOutcome::CandidateOnly { email }
        }
    })
}

#[tauri::command]
pub async fn desktop_launch(
    lock: tauri::State<'_, crate::state::DesktopOpState>,
    app: tauri::AppHandle,
) -> Result<(), ErrorDto> {
    use tauri::Emitter;
    let _guard = lock.0.lock().await;
    let platform =
        claudepot_core::desktop_backend::create_platform().ok_or_else(desktop_unsupported)?;
    // `DesktopSwapError` carries its own code.
    platform.launch().await.map_err(ErrorDto::from)?;
    let _ = app.emit(crate::events::DESKTOP_RUNNING_CHANGED, true);
    Ok(())
}

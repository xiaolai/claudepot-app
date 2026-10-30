//! Tauri commands for Settings → Relay, and the read-only list of
//! paired relay servers the Providers tab shows (`relay_client_servers`).
//!
//! Every verb is `claudepot_core::relay::service`, which `claudepot
//! relay …` also calls. What lives here is marshalling and the
//! process-local server handle ([`RelayServerState`]).
//!
//! ## Secret direction
//!
//! No credential crosses this bridge in either direction. Device tokens
//! are minted on the client during pairing and stored there; the server
//! keeps a hash. The pairing **code** does return to the renderer — it
//! is shown once so the user can type it on the other machine, it is
//! single-use and expires in minutes, and it is not logged here.

use claudepot_core::relay::client_ops::{self, ClientCtx, ServerView};
use claudepot_core::relay::devices::Revocation;
use claudepot_core::relay::revoke_socket::{self, LiveRevoke, LiveRevokeError};
use claudepot_core::relay::runtime::Reset;
use claudepot_core::relay::select::{Ineligible as SelectIneligible, Ranked, Tier};
use claudepot_core::relay::service::{self, DeviceSummary, Ineligible, PublicationView};
use claudepot_core::relay::target::UpstreamId;
use claudepot_core::remote::service::exposure_word;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::dto_error::{codes, ErrorDto};
use crate::relay_server::{RelayServerState, RelayStatusEvent};

fn map_err(e: impl std::fmt::Display) -> ErrorDto {
    ErrorDto::detail(codes::RELAY_FAILED, e)
}

fn upstream(id: &str) -> Result<UpstreamId, ErrorDto> {
    UpstreamId::parse(id).ok_or_else(|| map_err(format!("not a valid upstream id: {id:?}")))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayDayUsageDto {
    pub requests: u64,
    pub failed: u64,
    /// Always an estimate; render with `≈`. Meaningless when every
    /// request was unpriced (`unpriced == requests`).
    pub cost_usd: f64,
    pub unpriced: u64,
    pub family_estimate: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayDeviceDto {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
    pub include_future: bool,
    /// Upstream ids the device has accepted.
    pub grants: Vec<String>,
    pub last_seen: Option<String>,
    pub today: RelayDayUsageDto,
}

/// What a publication serves. The store holds exactly one of an
/// account and a route per publication (`relay::store` checks it), so
/// the kind is the route id's presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayPublicationKind {
    /// A subscription account (`relay publish`).
    Account,
    /// A provider route served by pass-through (`relay publish-route`).
    Route,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayPublicationDto {
    pub id: String,
    pub generation: u64,
    pub kind: RelayPublicationKind,
    /// `kind == account` only. Null when the account is no longer
    /// registered here.
    pub account_email: Option<String>,
    /// `kind == route` only.
    pub route_id: Option<String>,
    /// `kind == route` only. Null when the route is no longer in the
    /// route store.
    pub route_name: Option<String>,
}

fn to_publication_dto(p: &PublicationView) -> RelayPublicationDto {
    RelayPublicationDto {
        id: p.id.as_str().to_owned(),
        generation: p.generation,
        kind: if p.route_id.is_some() {
            RelayPublicationKind::Route
        } else {
            RelayPublicationKind::Account
        },
        account_email: p.account_email.clone(),
        route_id: p.route_id.map(|id| id.to_string()),
        route_name: p.route_name.clone(),
    }
}

/// One registered account, as the publish picker sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayAccountDto {
    pub email: String,
    /// `cc_active` | `no_credentials`, or null when it may be published.
    /// The picker states this reason inline rather than hiding the row.
    pub ineligible: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayWarningDto {
    pub upstream: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatusDto {
    /// A relay serves on this machine — this process or a terminal.
    pub serving: bool,
    /// This process serves. `serving && !running_here` means a CLI
    /// relay owns it; Stop here cannot touch it, and config changes are
    /// refused until it stops.
    pub running_here: bool,
    pub addr: Option<String>,
    pub bind: String,
    pub port: u16,
    /// `loopback` | `private_network` | `every_interface`, or null when
    /// the stored address is refused.
    pub exposure: Option<String>,
    pub bind_error: Option<String>,
    pub ca_fingerprint: Option<String>,
    pub config_recovered: bool,
    pub devices_recovered: bool,
    /// The usage ledger could not be read, so per-device `today` is unknown, not zero.
    pub usage_unavailable: bool,
    pub publications: Vec<RelayPublicationDto>,
    pub devices: Vec<RelayDeviceDto>,
    pub accounts: Vec<RelayAccountDto>,
    /// Why the last start here failed.
    pub last_error: Option<String>,
    /// `relay_ca_key_unavailable` and the other TLS codes, when the
    /// start was refused for its certificate material.
    pub last_error_code: Option<String>,
    /// Per-upstream problems found at start.
    pub warnings: Vec<RelayWarningDto>,
    /// Live state when serving here (same shape as `relay-status`).
    pub live: RelayStatusEvent,
    /// Serving here, and a terminal's `relay revoke` cannot reach this
    /// relay: the `revoke_socket::Unavailable` code saying why. Null when
    /// it can, or when nothing serves here.
    pub live_revoke_unavailable: Option<String>,
    /// A terminal relay serves and has a revoke socket, so Revoke here
    /// can go through it. A hint read from the filesystem; the revoke
    /// itself reports a socket that does not answer.
    pub revoke_socket_present: bool,
}

fn to_device_dto(d: &DeviceSummary) -> RelayDeviceDto {
    RelayDeviceDto {
        id: d.id.clone(),
        name: d.name.clone(),
        created_at: d.created_at.to_rfc3339(),
        revoked_at: d.revoked_at.map(|t| t.to_rfc3339()),
        include_future: d.include_future,
        grants: d
            .grants
            .iter()
            .map(|g| g.upstream_id.as_str().to_owned())
            .collect(),
        last_seen: d.last_seen.map(|t| t.to_rfc3339()),
        today: RelayDayUsageDto {
            requests: d.today.requests,
            failed: d.today.failed,
            cost_usd: d.today.cost_usd,
            unpriced: d.today.unpriced,
            family_estimate: d.today.family_estimate,
        },
    }
}

fn ineligible_code(r: Ineligible) -> String {
    r.code().to_owned()
}

#[tauri::command]
pub async fn relay_status(server: State<'_, RelayServerState>) -> Result<RelayStatusDto, ErrorDto> {
    // One snapshot: this process's relay and the machine-wide read, with
    // no start or stop here in between (`RelayServerState::snapshot`).
    let (local, read) = server
        .snapshot(|| {
            let accounts = crate::commands::open_store()
                .and_then(|s| s.list().map_err(|e| format!("account list failed: {e}")))?;
            let dir = service::default_dir();
            let st =
                service::status(&dir, &accounts, chrono::Utc::now()).map_err(|e| e.to_string())?;
            let socket = revoke_socket::socket_present(&dir);
            Ok::<_, String>((st, accounts, socket))
        })
        .await
        .map_err(ErrorDto::task_join)?;
    let (st, accounts, socket) = read.map_err(map_err)?;

    Ok(RelayStatusDto {
        serving: st.serving,
        running_here: local.running_here,
        addr: local.addr,
        bind: st.bind.to_string(),
        port: st.port,
        exposure: st.exposure.map(|e| exposure_word(e).to_owned()),
        bind_error: st.bind_error,
        ca_fingerprint: st.ca_fingerprint,
        // The start that set a store aside consumed the evidence, so the
        // status read alone would report nothing recovered.
        config_recovered: st.config_recovered || local.resets.contains(&Reset::Config),
        devices_recovered: st.devices_recovered || local.resets.contains(&Reset::Devices),
        usage_unavailable: st.usage_unavailable,
        publications: st.publications.iter().map(to_publication_dto).collect(),
        devices: st.devices.iter().map(to_device_dto).collect(),
        accounts: accounts
            .iter()
            .map(|a| RelayAccountDto {
                email: a.email.clone(),
                ineligible: service::eligibility(a).err().map(ineligible_code),
            })
            .collect(),
        last_error: local.last_error,
        last_error_code: local.last_error_code.map(str::to_owned),
        warnings: local
            .warnings
            .iter()
            .map(|w| RelayWarningDto {
                upstream: w.upstream.as_str().to_owned(),
                code: w.code.clone(),
            })
            .collect(),
        live: match &local.live {
            Some(l) => RelayStatusEvent::from_live(l),
            None => RelayStatusEvent::stopped(),
        },
        live_revoke_unavailable: match local.live_revoke {
            Some(LiveRevoke::Unavailable(why)) => Some(why.code().to_owned()),
            Some(LiveRevoke::Listening) | None => None,
        },
        revoke_socket_present: st.serving && !local.running_here && socket,
    })
}

/// Start serving in this process. Returns the listening address.
#[tauri::command]
pub async fn relay_start(
    app: AppHandle,
    server: State<'_, RelayServerState>,
) -> Result<String, ErrorDto> {
    server.start(app).await.map_err(map_err)
}

/// Stop the relay this process serves. `false` when it served nothing.
#[tauri::command]
pub async fn relay_stop(
    app: AppHandle,
    server: State<'_, RelayServerState>,
) -> Result<bool, ErrorDto> {
    Ok(server.stop(&app).await)
}

/// Change where the relay listens. Refused while it serves. Returns the
/// exposure word the pane states next to the address.
#[tauri::command]
pub async fn relay_set_bind(address: String, port: Option<u16>) -> Result<String, ErrorDto> {
    tokio::task::spawn_blocking(move || {
        service::set_bind(&service::default_dir(), &address, port, None)
    })
    .await
    .map_err(ErrorDto::task_join)?
    .map(|e| exposure_word(e).to_owned())
    .map_err(map_err)
}

/// Publish an account as an upstream. Refuses the CC-active account.
#[tauri::command]
pub async fn relay_publish(email: String, id: Option<String>) -> Result<String, ErrorDto> {
    let id = id.as_deref().map(upstream).transpose()?;
    let account = tokio::task::spawn_blocking(move || {
        crate::commands::open_store()?
            .find_by_email(&email)
            .map_err(|e| format!("account lookup failed: {e}"))?
            .ok_or_else(|| format!("no registered account {email}"))
    })
    .await
    .map_err(ErrorDto::task_join)?
    .map_err(map_err)?;
    let (id, _generation) = service::publish(&service::default_dir(), &account, id)
        .await
        .map_err(map_err)?;
    Ok(id.as_str().to_owned())
}

#[tauri::command]
pub async fn relay_unpublish(id: String) -> Result<(), ErrorDto> {
    let id = upstream(&id)?;
    tokio::task::spawn_blocking(move || service::unpublish(&service::default_dir(), &id))
        .await
        .map_err(ErrorDto::task_join)?
        .map_err(map_err)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayRevokeReceiptDto {
    /// It was revoked before this call; nothing was written.
    pub already_revoked: bool,
}

/// Revoke a device. Through the live relay when it serves here, or
/// through a terminal relay's revoke socket, so its streams end now;
/// offline when nothing serves.
#[tauri::command]
pub async fn relay_revoke_device(
    server: State<'_, RelayServerState>,
    id: String,
) -> Result<RelayRevokeReceiptDto, ErrorDto> {
    match server.revoke(&id).await {
        Ok(r) => Ok(RelayRevokeReceiptDto {
            already_revoked: r == Revocation::AlreadyRevoked,
        }),
        Err(e) => Err(revoke_error(e)),
    }
}

/// The one revoke failure whose outcome is unknown gets its own code: the
/// pane must not say "failed" about a device that may be revoked, and the
/// core text points at a CLI verb.
fn revoke_error(e: service::ServiceError) -> ErrorDto {
    match e {
        service::ServiceError::LiveRevoke(e @ LiveRevokeError::NoAnswer) => {
            ErrorDto::new(codes::RELAY_REVOKE_OUTCOME_UNKNOWN, e.to_string())
        }
        e => map_err(e),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayPairOfferDto {
    pub code: String,
    pub expires_in_secs: u64,
    pub ca_fingerprint: String,
}

/// Mint a one-time pairing code on the relay serving here.
#[tauri::command]
pub async fn relay_pair_code(
    server: State<'_, RelayServerState>,
    include_future: bool,
) -> Result<RelayPairOfferDto, ErrorDto> {
    let offer = server.pair_code(include_future).await.map_err(map_err)?;
    Ok(RelayPairOfferDto {
        code: offer.code,
        expires_in_secs: offer.expires_in.as_secs(),
        ca_fingerprint: offer.ca_fingerprint,
    })
}

/// One upstream of a paired relay server, as the client sees it now.
///
/// Every reason is a fixed code: an ineligibility's own text can carry
/// the probe's error or the health report's credential error, both of
/// which the relay supplied, and relay text never reaches the renderer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayClientUpstreamDto {
    pub id: String,
    /// A run could use this upstream on this server now.
    pub eligible: bool,
    /// `measured` | `connect` when eligible.
    pub tier: Option<&'static str>,
    /// `tier == measured` only: the model the TTFB samples are for.
    pub tier_model: Option<String>,
    /// `tier == measured` only.
    pub ttfb_ms: Option<u64>,
    /// Compared only within a tier.
    pub score_ms: Option<u64>,
    /// `select::Ineligible::code` when not eligible.
    pub reason: Option<&'static str>,
    /// `reason == cooldown` only.
    pub cooldown_secs: Option<u64>,
    /// `reason == saturated` only: `5-hour` | `7-day`.
    pub saturated_window: Option<&'static str>,
    /// `reason == saturated` only, in percent.
    pub saturated_utilization: Option<f64>,
}

/// One paired relay server from `relay-servers.json`, probed now —
/// what `claudepot relay servers --json` prints, with codes for text.
/// No device token is here: [`client_ops::ServerView`] holds none, and
/// only the listed fields are mapped.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayClientServerDto {
    /// Internal; a stable key for the renderer, never shown.
    pub server_id: String,
    pub name: String,
    pub base_url: String,
    pub reachable: bool,
    /// [`client_ops::ProbeFailure::code`] when unreachable.
    pub error: Option<&'static str>,
    pub rtt_ms: Option<u64>,
    pub default_upstream: Option<String>,
    pub upstreams: Vec<RelayClientUpstreamDto>,
}

fn to_client_upstream_dto(
    id: &UpstreamId,
    verdict: &Result<Ranked, SelectIneligible>,
) -> RelayClientUpstreamDto {
    let mut dto = RelayClientUpstreamDto {
        id: id.as_str().to_owned(),
        eligible: verdict.is_ok(),
        tier: None,
        tier_model: None,
        ttfb_ms: None,
        score_ms: None,
        reason: None,
        cooldown_secs: None,
        saturated_window: None,
        saturated_utilization: None,
    };
    match verdict {
        Ok(r) => {
            dto.score_ms = Some(r.score_ms);
            match &r.tier {
                Tier::Measured { model, ttfb_ms } => {
                    dto.tier = Some("measured");
                    dto.tier_model = Some(model.clone());
                    dto.ttfb_ms = Some(*ttfb_ms);
                }
                Tier::Connect => dto.tier = Some("connect"),
            }
        }
        Err(why) => {
            dto.reason = Some(why.code());
            match why {
                SelectIneligible::Cooldown(s) => dto.cooldown_secs = Some(*s),
                SelectIneligible::Saturated {
                    window,
                    utilization,
                } => {
                    dto.saturated_window = Some(window);
                    dto.saturated_utilization = Some(*utilization);
                }
                _ => {}
            }
        }
    }
    dto
}

fn to_client_server_dto(v: &ServerView) -> RelayClientServerDto {
    RelayClientServerDto {
        server_id: v.record.server_id.clone(),
        name: v.record.name.clone(),
        base_url: v.record.base_url.clone(),
        reachable: v.probe.is_ok(),
        error: v.probe.as_ref().err().map(|f| f.code),
        rtt_ms: v.probe.as_ref().ok().map(|p| p.rtt_ms),
        default_upstream: v
            .record
            .default_upstream
            .as_ref()
            .map(|u| u.as_str().to_owned()),
        upstreams: v
            .upstreams
            .iter()
            .map(|(id, verdict)| to_client_upstream_dto(id, verdict))
            .collect(),
    }
}

/// The relay servers this machine has paired with, each probed now.
/// Read-only: pairing, acceptance and launching stay `claudepot relay
/// pair | accept | run`. Probes run concurrently, each request bounded
/// by the relay client's own timeout, so one stalled server cannot hold
/// the list.
#[tauri::command]
pub async fn relay_client_servers() -> Result<Vec<RelayClientServerDto>, ErrorDto> {
    let views = client_ops::list(&ClientCtx::production(), chrono::Utc::now())
        .await
        .map_err(map_err)?;
    Ok(views.iter().map(to_client_server_dto).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(account: Option<&str>, route: Option<(uuid::Uuid, Option<&str>)>) -> PublicationView {
        PublicationView {
            id: UpstreamId::parse("u1").unwrap(),
            generation: 3,
            account_id: account.map(|_| uuid::Uuid::new_v4()),
            account_email: account.map(str::to_owned),
            route_id: route.map(|(id, _)| id),
            route_name: route.and_then(|(_, n)| n.map(str::to_owned)),
        }
    }

    #[test]
    fn route_publication_keeps_its_kind_and_name() {
        let id = uuid::Uuid::new_v4();
        let dto = to_publication_dto(&view(None, Some((id, Some("DeepSeek")))));
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["kind"], "route");
        assert_eq!(json["routeId"], id.to_string());
        assert_eq!(json["routeName"], "DeepSeek");
        assert!(json["accountEmail"].is_null());
    }

    #[test]
    fn route_publication_whose_route_is_gone_is_still_a_route() {
        let dto = to_publication_dto(&view(None, Some((uuid::Uuid::new_v4(), None))));
        assert_eq!(dto.kind, RelayPublicationKind::Route);
        assert!(dto.route_name.is_none());
    }

    #[test]
    fn account_publication_keeps_its_email() {
        let json =
            serde_json::to_value(to_publication_dto(&view(Some("a@example.com"), None))).unwrap();
        assert_eq!(json["kind"], "account");
        assert_eq!(json["accountEmail"], "a@example.com");
        assert!(json["routeId"].is_null());
        assert!(json["routeName"].is_null());
    }

    fn server_view(probe: Result<u64, &'static str>) -> ServerView {
        use claudepot_core::relay::client::Probe;
        use claudepot_core::relay::health::HealthReport;
        use claudepot_core::relay::servers::ServerRecord;
        let up = |s: &str| UpstreamId::parse(s).unwrap();
        ServerView {
            record: ServerRecord {
                server_id: "srv-internal-id".into(),
                name: "studio".into(),
                base_url: "https://studio.example:8443".into(),
                ca_pem: Some("-----BEGIN CERTIFICATE-----".into()),
                ca_fingerprint: Some("ab:cd".into()),
                device_id: "dev-internal-id".into(),
                default_upstream: Some(up("max")),
                upstreams: Vec::new(),
                paired_at: chrono::Utc::now(),
            },
            probe: probe
                .map(|rtt_ms| Probe {
                    connect_ms: 30,
                    rtt_ms,
                    health: HealthReport {
                        ok: true,
                        server_id: "srv-internal-id".into(),
                        version: "1".into(),
                        upstreams: Vec::new(),
                    },
                })
                .map_err(|code| client_ops::ProbeFailure {
                    code,
                    // What a relay could put in a refusal body.
                    message: "the relay refused: Bearer sk-ant-oat01-SECRET (HTTP 403)".into(),
                }),
            upstreams: vec![
                (
                    up("max"),
                    Ok(Ranked {
                        server_id: "srv-internal-id".into(),
                        name: "studio".into(),
                        tier: Tier::Measured {
                            model: "claude-opus-4".into(),
                            ttfb_ms: 400,
                        },
                        rtt_ms: 12,
                        score_ms: 412,
                    }),
                ),
                (
                    up("pro"),
                    Err(SelectIneligible::CredentialNotReady(Some(
                        "upstream said: token sk-ant-oat01-SECRET".into(),
                    ))),
                ),
                (up("cool"), Err(SelectIneligible::Cooldown(42))),
                (
                    up("full"),
                    Err(SelectIneligible::Saturated {
                        window: "5-hour",
                        utilization: 97.0,
                    }),
                ),
                (
                    up("conn"),
                    Ok(Ranked {
                        server_id: "srv-internal-id".into(),
                        name: "studio".into(),
                        tier: Tier::Connect,
                        rtt_ms: 12,
                        score_ms: 42,
                    }),
                ),
            ],
        }
    }

    #[test]
    fn reachable_server_maps_every_listed_field() {
        let json = serde_json::to_value(to_client_server_dto(&server_view(Ok(12)))).unwrap();
        assert_eq!(json["serverId"], "srv-internal-id");
        assert_eq!(json["name"], "studio");
        assert_eq!(json["baseUrl"], "https://studio.example:8443");
        assert_eq!(json["reachable"], true);
        assert!(json["error"].is_null());
        assert_eq!(json["rttMs"], 12);
        assert_eq!(json["defaultUpstream"], "max");
        let ups = json["upstreams"].as_array().unwrap();
        assert_eq!(ups.len(), 5);
        assert_eq!(ups[0]["id"], "max");
        assert_eq!(ups[0]["eligible"], true);
        assert_eq!(ups[0]["tier"], "measured");
        assert_eq!(ups[0]["tierModel"], "claude-opus-4");
        assert_eq!(ups[0]["ttfbMs"], 400);
        assert_eq!(ups[0]["scoreMs"], 412);
        assert!(ups[0]["reason"].is_null());
        assert_eq!(ups[1]["eligible"], false);
        assert_eq!(ups[1]["reason"], "credential_not_ready");
        assert!(ups[1]["scoreMs"].is_null());
        assert_eq!(ups[2]["reason"], "cooldown");
        assert_eq!(ups[2]["cooldownSecs"], 42);
        assert_eq!(ups[3]["reason"], "saturated");
        assert_eq!(ups[3]["saturatedWindow"], "5-hour");
        assert_eq!(ups[3]["saturatedUtilization"], 97.0);
        assert_eq!(ups[4]["tier"], "connect");
        assert!(ups[4]["tierModel"].is_null());
    }

    #[test]
    fn unreachable_server_sends_the_code_not_the_message() {
        let json =
            serde_json::to_value(to_client_server_dto(&server_view(Err("refused")))).unwrap();
        assert_eq!(json["reachable"], false);
        assert_eq!(json["error"], "refused");
        assert!(json["rttMs"].is_null());
    }

    #[test]
    fn no_relay_supplied_text_or_record_secret_crosses() {
        for probe in [Ok(12), Err("refused")] {
            let text = serde_json::to_string(&to_client_server_dto(&server_view(probe))).unwrap();
            for needle in [
                "sk-ant",
                "SECRET",
                "upstream said",
                "dev-internal-id",
                "BEGIN CERTIFICATE",
                "ab:cd",
            ] {
                assert!(!text.contains(needle), "{needle} crossed: {text}");
            }
        }
    }
}

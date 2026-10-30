//! The one implementation of every server-side relay verb.
//!
//! `claudepot relay serve|status|bind|publish|unpublish|devices|revoke`
//! and the GUI's Settings → Relay pane both go through here, for the
//! reason `remote::service` gives: a second implementation drops a
//! refusal. The refusals worth protecting are these:
//!
//! - **Never the CC-active account** (plan §5.2). [`eligibility`] is what
//!   a picker shows; [`publish_with`] re-checks through the lease, which
//!   asks Claude Code's own credential store, so a stale `is_cli_active`
//!   cannot let it through.
//! - **One process owns the device list while serving.** A serving relay
//!   holds `relay-devices.json` in memory and writes it from that copy,
//!   so a revocation written by another process would be invisible to it
//!   and then overwritten by its next write. `relay-serve.lock` is held
//!   for as long as a relay serves; every offline mutation checks it and
//!   refuses with [`ServiceError::Serving`], and the serving process
//!   mutates through its live [`RelayState`] instead ([`revoke_live`]).
//!   Revoking is the one mutation another process can still ask for:
//!   [`revoke_anywhere`] hands it to the serving relay over its revoke
//!   socket ([`revoke_socket`]), which revokes through that live state.
//! - **A recovered device file revokes nothing.** It already lost every
//!   revocation, so a revoke against it would report success while every
//!   earlier revoked token is live again; it is refused.
//!
//! Presentation stays with each caller.

use std::collections::HashMap;
use std::fs::File;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use chrono::{DateTime, Local, Utc};
use uuid::Uuid;

use super::devices::{DeviceRecord, DeviceRegistry, Grant, Revocation};
use super::forward::{SubscriptionHandler, ANTHROPIC_ORIGIN};
use super::health::{self, UpstreamStatus};
use super::identity::{TargetKind, TargetSpec};
use super::lease::{LeaseBackend, LeaseManager, StoreBackend};
use super::ledger::{Ledger, LedgerLine};
use super::listen::{self, ListenError};
use super::passthrough::{route_target_spec, PassThroughHandler};
use super::refusal::Refusal;
use super::revoke_socket::{self, LiveRevokeError};
use super::route_secret::{self, RouteSecretError};
use super::runtime::{self, RelayPaths, Reset, Running, Serving, StartError};
use super::serve_lock::ServeLock;
use super::server::RelayState;
use super::store::{self, ConfigFile, Publication, StoreError};
use super::target::UpstreamId;
use super::tls;
use super::translate::codex::{self, CodexAuth, CodexAuthError};
use super::translate::continuation::{self as continuation, ContinuationStore};
use super::translate::handler::TranslateHandler;
use super::translate::{self, TranslateRouteError, Translation};
use super::upstream::{OutcomeKind, TargetIdentity, UpstreamError, UpstreamHandler};
use crate::account::Account;
use crate::remote::bind::Exposure;
use crate::routes::{Route, RouteStore};

pub const SERVE_LOCK_FILENAME: &str = "relay-serve.lock";
/// The upstream id a subscription is published under unless told
/// otherwise.
pub const DEFAULT_SUBSCRIPTION_ID: &str = "sub";
/// The route store a relay in `dir` publishes from. In production `dir`
/// is the data dir, so this is `routes::store::routes_file_path()`.
pub const ROUTES_FILENAME: &str = "routes.json";

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Listen(#[from] ListenError),
    #[error(transparent)]
    Start(#[from] StartError),
    #[error("not an IP address: {0}")]
    BadBind(String),
    #[error(
        "a relay is serving on this machine — stop it first, or make this change from \
         the process that serves"
    )]
    Serving,
    #[error("relay serve lock {path}: {source}")]
    Lock {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the relay device file could not be read and was reset, so every earlier \
         revocation is already lost; this cannot honestly revoke anything. Re-pair the \
         devices you still want, and treat any token from before the reset as live."
    )]
    DevicesRecovered,
    #[error("no such relay device: {0}")]
    NoSuchDevice(String),
    #[error(transparent)]
    LiveRevoke(#[from] LiveRevokeError),
    #[error("{email} cannot be served: {reason}")]
    Ineligible { email: String, reason: Ineligible },
    #[error("upstream {0} is not published")]
    NotPublished(UpstreamId),
    #[error("could not reach {email}'s credential: {error}")]
    Credential { email: String, error: UpstreamError },
    #[error("the relay is not serving pairing (no TLS listener)")]
    NoPairing,
    #[error("not a usable upstream id: {0:?} (lowercase letters, digits and '-')")]
    BadUpstreamId(String),
    #[error("routes store: {0}")]
    Routes(String),
    #[error("no route named {0:?}")]
    NoSuchRoute(String),
    #[error("{0:?} names more than one route; use the route's id")]
    AmbiguousRoute(String),
    #[error("route {route} cannot be served: {error}")]
    RouteSecret {
        route: String,
        error: RouteSecretError,
    },
    #[error("route {route} cannot be translated: {error}")]
    Translate {
        route: String,
        error: TranslateRouteError,
    },
    #[error("route {route} cannot be served through codex: {error}")]
    Codex {
        route: String,
        error: CodexAuthError,
    },
}

impl ServiceError {
    /// The stable code of a start refused for its TLS material, so a UI
    /// can raise the one banner `relay_ca_key_unavailable` needs (the CA
    /// key is locked or gone, and renewing the leaf is impossible)
    /// without matching on `Display` text. `None` for every other error.
    pub fn trust_code(&self) -> Option<&'static str> {
        match self {
            ServiceError::Start(StartError::Trust(e)) => Some(e.code()),
            _ => None,
        }
    }
}

/// Why an account cannot be published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Ineligible {
    /// Claude Code on this machine is signed into it; serving it would
    /// rotate the refresh token out from under CC (plan §5.2, P0b).
    #[error("it is Claude Code's active login on this machine")]
    CcActive,
    #[error("Claudepot holds no CLI credential for it")]
    NoCredentials,
}

impl Ineligible {
    pub fn code(self) -> &'static str {
        match self {
            Ineligible::CcActive => "cc_active",
            Ineligible::NoCredentials => "no_credentials",
        }
    }
}

/// What a picker shows. Advisory: [`publish_with`] decides.
pub fn eligibility(a: &Account) -> Result<(), Ineligible> {
    if a.is_cli_active {
        return Err(Ineligible::CcActive);
    }
    if !a.has_cli_credentials {
        return Err(Ineligible::NoCredentials);
    }
    Ok(())
}

/// The directory every relay store lives in.
pub fn default_dir() -> PathBuf {
    crate::paths::claudepot_data_dir()
}

// ---- the serve lock -------------------------------------------------------

fn lock_path(dir: &Path) -> PathBuf {
    dir.join(SERVE_LOCK_FILENAME)
}

fn open_lock(dir: &Path) -> Result<File, ServiceError> {
    let path = lock_path(dir);
    std::fs::create_dir_all(dir).map_err(|source| ServiceError::Lock {
        path: path.display().to_string(),
        source,
    })?;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|source| ServiceError::Lock {
            path: path.display().to_string(),
            source,
        })
}

/// Take the lock, or [`ServiceError::Serving`] if any process holds it.
pub fn acquire_serve_lock(dir: &Path) -> Result<ServeLock, ServiceError> {
    let file = open_lock(dir)?;
    match file.try_lock() {
        Ok(()) => Ok(ServeLock::new(file, dir.to_path_buf())),
        Err(std::fs::TryLockError::WouldBlock) => Err(ServiceError::Serving),
        Err(std::fs::TryLockError::Error(source)) => Err(ServiceError::Lock {
            path: lock_path(dir).display().to_string(),
            source,
        }),
    }
}

/// Is a relay serving on this machine — in any process, this one
/// included?
pub fn is_serving(dir: &Path) -> Result<bool, ServiceError> {
    match acquire_serve_lock(dir) {
        Ok(_probe) => Ok(false),
        Err(ServiceError::Serving) => Ok(true),
        Err(e) => Err(e),
    }
}

fn require_stopped(dir: &Path) -> Result<ServeLock, ServiceError> {
    acquire_serve_lock(dir)
}

// ---- status ---------------------------------------------------------------

/// One day of a device's traffic.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DayUsage {
    pub requests: u64,
    pub failed: u64,
    /// Sum over priced requests; always an estimate (`≈`).
    pub cost_usd: f64,
    /// Requests with no price (a route upstream, or an unknown model).
    pub unpriced: u64,
    /// Some cost came from a model family's rate, not the model's own.
    pub family_estimate: bool,
}

#[derive(Debug, Clone)]
pub struct DeviceSummary {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub include_future: bool,
    pub grants: Vec<Grant>,
    /// The last request the ledger recorded for it.
    pub last_seen: Option<DateTime<Utc>>,
    pub today: DayUsage,
}

#[derive(Debug, Clone)]
pub struct PublicationView {
    pub id: UpstreamId,
    pub generation: u64,
    pub account_id: Option<Uuid>,
    /// The account's email, when it is still registered here.
    pub account_email: Option<String>,
    /// The route a pass-through upstream serves.
    pub route_id: Option<Uuid>,
    /// The route's name, when it is still in the route store.
    pub route_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub server_id: String,
    pub bind: IpAddr,
    pub port: u16,
    pub names: Vec<String>,
    /// `None` when the stored address is refused (`bind_error` says why).
    pub exposure: Option<Exposure>,
    pub bind_error: Option<String>,
    pub publications: Vec<PublicationView>,
    pub devices: Vec<DeviceSummary>,
    /// The CA a client pins, once one has been minted.
    pub ca_fingerprint: Option<String>,
    pub config_recovered: bool,
    pub devices_recovered: bool,
    /// The usage ledger exists but could not be read: every device's
    /// `last_seen` and `today` are unknown, not zero.
    pub usage_unavailable: bool,
    /// A relay serves on this machine (any process).
    pub serving: bool,
}

pub fn bind_of(config: &ConfigFile) -> IpAddr {
    config.bind.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

/// Per-device rollups from the ledger: last seen, and `day`'s usage.
type Rollup = HashMap<String, (DateTime<Utc>, DayUsage)>;

/// Fold one ledger line into `out`. One line at a time, so a status read
/// holds the per-device totals and never the (unpruned) ledger itself.
fn roll(out: &mut Rollup, l: &LedgerLine, day: chrono::NaiveDate) {
    let e = out
        .entry(l.device.clone())
        .or_insert_with(|| (l.ts, DayUsage::default()));
    if l.ts > e.0 {
        e.0 = l.ts;
    }
    if l.ts.with_timezone(&Local).date_naive() != day {
        return;
    }
    let u = &mut e.1;
    u.requests += 1;
    if l.outcome == OutcomeKind::Failed {
        u.failed += 1;
    }
    match &l.cost {
        Some(c) => {
            u.cost_usd += c.usd;
            u.family_estimate |= c.family_estimate;
        }
        None => u.unpriced += 1,
    }
}

/// Everything a surface shows about this machine's relay server.
pub fn status(
    dir: &Path,
    accounts: &[Account],
    now: DateTime<Utc>,
) -> Result<ServerStatus, ServiceError> {
    // A route store that cannot be read only costs the route names.
    let routes = load_routes(dir).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "routes store unreadable; route names omitted");
        Vec::new()
    });
    let paths = RelayPaths::in_dir(dir);
    let stores = read_stores(dir, &paths)?;
    let config = stores.config;
    // An unreadable ledger is unknown usage, not zero usage: say so.
    let day = now.with_timezone(&Local).date_naive();
    let mut rolled = Rollup::new();
    let usage_unavailable = match Ledger::for_each(&paths.usage, |l| roll(&mut rolled, &l, day)) {
        Ok(()) => false,
        Err(e) => {
            tracing::warn!(error = %e, "relay usage ledger unreadable; usage unavailable");
            // A read that failed part-way is not a partial total.
            rolled.clear();
            true
        }
    };

    let bind = bind_of(&config);
    let (exposure, bind_error) = match listen::check(bind, true) {
        Ok(b) => (Some(b.exposure()), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let publications = config
        .published()
        .map(|p| PublicationView {
            id: p.id.clone(),
            generation: p.generation,
            account_id: p.account_id,
            account_email: p
                .account_id
                .and_then(|id| accounts.iter().find(|a| a.uuid == id))
                .map(|a| a.email.clone()),
            route_id: p.route_id,
            route_name: p
                .route_id
                .and_then(|id| routes.iter().find(|r| r.id == id))
                .map(|r| r.name.clone()),
        })
        .collect();
    let devices = stores
        .devices
        .into_iter()
        .map(|d| {
            let (last_seen, today) = rolled
                .get(&d.id)
                .map(|(t, u)| (Some(*t), u.clone()))
                .unwrap_or_default();
            DeviceSummary {
                id: d.id,
                name: d.name,
                created_at: d.created_at,
                revoked_at: d.revoked_at,
                include_future: d.include_future,
                grants: d.grants,
                last_seen,
                today,
            }
        })
        .collect();
    let ca_fingerprint = std::fs::read(&paths.trust.ca_cert)
        .ok()
        .and_then(|pem| tls::fingerprint_pem(&pem));
    Ok(ServerStatus {
        server_id: config.server_id.clone(),
        bind,
        port: config.port,
        names: config.names.clone(),
        exposure,
        bind_error,
        publications,
        devices,
        ca_fingerprint,
        config_recovered: stores.config_recovered,
        devices_recovered: stores.devices_recovered,
        usage_unavailable,
        serving: stores.serving,
    })
}

/// The two stores as [`status`] reads them.
struct StatusStores {
    config: ConfigFile,
    devices: Vec<DeviceRecord>,
    config_recovered: bool,
    devices_recovered: bool,
    serving: bool,
}

/// Read both stores, recovering a corrupt one **only while holding the
/// serve lock**. A serving relay — another process, or this one — owns
/// the live copy: setting its file aside would make status report no
/// devices while the live registry still authorizes them, and lose every
/// record at the next start. So while anyone serves, the read does not
/// mutate, and a corrupt file is an error rather than an empty list. The
/// lock is held only for the two reads, as [`is_serving`]'s probe is.
fn read_stores(dir: &Path, paths: &RelayPaths) -> Result<StatusStores, ServiceError> {
    match acquire_serve_lock(dir) {
        Ok(_stopped) => {
            let config = store::load_config(&paths.config)?;
            let devices = store::load_devices(&paths.devices)?;
            Ok(StatusStores {
                config: config.value,
                devices: devices.value.devices,
                config_recovered: config.recovery.is_some(),
                devices_recovered: devices.recovery.is_some(),
                serving: false,
            })
        }
        Err(ServiceError::Serving) => Ok(StatusStores {
            config: store::peek_config(&paths.config)?,
            devices: store::peek_devices(&paths.devices)?.devices,
            config_recovered: false,
            devices_recovered: false,
            serving: true,
        }),
        Err(e) => Err(e),
    }
}

// ---- configuration --------------------------------------------------------

/// Set the listen address (and optionally port and extra leaf names).
/// Refused while serving: the running relay would not see it.
pub fn set_bind(
    dir: &Path,
    bind: &str,
    port: Option<u16>,
    names: Option<Vec<String>>,
) -> Result<Exposure, ServiceError> {
    let addr: IpAddr = bind
        .trim()
        .parse()
        .map_err(|_| ServiceError::BadBind(bind.to_owned()))?;
    // Always TLS: the relay serves TLS on loopback too, so pairing works
    // there and the check is the off-loopback one.
    let checked = listen::check(addr, true)?;
    let _lock = require_stopped(dir)?;
    let paths = RelayPaths::in_dir(dir);
    let mut config = store::load_config(&paths.config)?.value;
    // `None` is `127.0.0.1` (`bind_of`) and nothing else: `::1` or
    // `127.0.0.2` stored as `None` would be served on `127.0.0.1`.
    config.bind = (addr != IpAddr::V4(Ipv4Addr::LOCALHOST)).then_some(addr);
    if let Some(p) = port {
        config.port = p;
    }
    if let Some(n) = names {
        config.names = n
            .into_iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
    }
    store::save_config(&paths.config, &config)?;
    Ok(checked.exposure())
}

/// The digest a subscription target carries for `identity`.
fn subscription_digest(identity: &super::lease::Identity) -> String {
    TargetSpec::subscription(ANTHROPIC_ORIGIN, &identity.account_uuid, &identity.org_uuid).digest()
}

/// Acquire a lease once, which is the owner check and the identity pin
/// in one place.
async fn probe_lease(
    backend: Arc<dyn LeaseBackend>,
) -> (
    Arc<LeaseManager>,
    Result<super::lease::Identity, UpstreamError>,
) {
    let lease = Arc::new(LeaseManager::new(backend));
    let r = match lease.current().await {
        Ok(_) => lease
            .readiness()
            .identity
            .ok_or(UpstreamError::CredentialUnavailable),
        Err(e) => Err(e),
    };
    (lease, r)
}

/// A serving lease for a publication accepted for `published_digest`.
/// When the start probe cannot confirm the identity, the lease is told
/// which one the publication was accepted for: a later first acquisition
/// that finds another account stops as `identity_changed` instead of
/// serving it under the old fingerprint and generation.
async fn subscription_lease(
    backend: Arc<dyn LeaseBackend>,
    published_digest: &str,
) -> (
    Arc<LeaseManager>,
    Result<super::lease::Identity, UpstreamError>,
) {
    let (lease, identity) = probe_lease(backend).await;
    if identity.is_err() {
        let expected = published_digest.to_owned();
        lease
            .expect_identity(Arc::new(move |i: &super::lease::Identity| {
                subscription_digest(i) == expected
            }))
            .await;
    }
    (lease, identity)
}

/// Publish `account`'s subscription as `id` (default
/// [`DEFAULT_SUBSCRIPTION_ID`]). Returns the generation. Publishing a
/// different account under an existing id bumps its generation, so
/// every device must accept the change before it is served.
pub async fn publish(
    dir: &Path,
    account: &Account,
    id: Option<UpstreamId>,
) -> Result<(UpstreamId, u64), ServiceError> {
    let backend = Arc::new(StoreBackend::new(account.uuid, account.email.clone()));
    publish_with(dir, account, id, backend).await
}

pub async fn publish_with(
    dir: &Path,
    account: &Account,
    id: Option<UpstreamId>,
    backend: Arc<dyn LeaseBackend>,
) -> Result<(UpstreamId, u64), ServiceError> {
    eligibility(account).map_err(|reason| ServiceError::Ineligible {
        email: account.email.clone(),
        reason,
    })?;
    let id = match id {
        Some(id) => id,
        None => UpstreamId::parse(DEFAULT_SUBSCRIPTION_ID)
            .ok_or_else(|| ServiceError::BadBind(DEFAULT_SUBSCRIPTION_ID.into()))?,
    };
    let _lock = require_stopped(dir)?;
    let (_lease, identity) = probe_lease(backend).await;
    let identity = identity.map_err(|error| match error {
        UpstreamError::OwnerConflict => ServiceError::Ineligible {
            email: account.email.clone(),
            reason: Ineligible::CcActive,
        },
        error => ServiceError::Credential {
            email: account.email.clone(),
            error,
        },
    })?;
    let paths = RelayPaths::in_dir(dir);
    let mut config = store::load_config(&paths.config)?.value;
    let generation = config.publish(&id, &subscription_digest(&identity), Some(account.uuid));
    store::save_config(&paths.config, &config)?;
    Ok((id, generation))
}

// ---- routes ---------------------------------------------------------------

/// The routes a relay in `dir` can publish.
pub fn load_routes(dir: &Path) -> Result<Vec<Route>, ServiceError> {
    RouteStore::open_at(dir.join(ROUTES_FILENAME))
        .map(|s| s.list().to_vec())
        .map_err(|e| ServiceError::Routes(e.to_string()))
}

/// The one route `query` names: its id, its name or its wrapper name,
/// exactly. No prefix or fuzzy match — a route decides who pays.
pub fn find_route<'a>(routes: &'a [Route], query: &str) -> Result<&'a Route, ServiceError> {
    let q = query.trim();
    let hits: Vec<&Route> = routes
        .iter()
        .filter(|r| {
            r.id.to_string() == q.to_ascii_lowercase() || r.name == q || r.wrapper_name == q
        })
        .collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(ServiceError::NoSuchRoute(q.to_owned())),
        _ => Err(ServiceError::AmbiguousRoute(q.to_owned())),
    }
}

/// The upstream id a route is published under unless told otherwise:
/// its wrapper name without the `claude-` convention prefix.
pub fn default_route_upstream_id(route: &Route) -> Option<UpstreamId> {
    let slug: String = route
        .wrapper_name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    let slug = slug
        .strip_prefix("claude-")
        .filter(|s| !s.is_empty())
        .unwrap_or(slug);
    UpstreamId::parse(slug)
}

/// The target a route upstream serves: passed through, or translated.
fn route_spec(
    route: &Route,
    translate: Option<Translation>,
) -> Result<TargetSpec, TranslateRouteError> {
    match translate {
        None => route_target_spec(route).map_err(TranslateRouteError::Secret),
        Some(t) => translate::route_target_spec(route, t),
    }
}

fn translate_err(route: &Route, error: TranslateRouteError) -> ServiceError {
    match error {
        TranslateRouteError::Secret(error) => ServiceError::RouteSecret {
            route: route.name.clone(),
            error,
        },
        error => ServiceError::Translate {
            route: route.name.clone(),
            error,
        },
    }
}

/// Publish a gateway route as `id`, passed through or (`translate`)
/// through a protocol translation. Refused for a non-gateway route and
/// for one whose credential this machine cannot read now (plan §5.5) — a
/// published route with an unreadable key would refuse every request. A
/// Codex translation pays with the relay's own Codex login
/// (`relay-codex/`), so that login is what is checked.
/// Returns the generation; a changed origin, model, key location or
/// translation bumps it, so every device must accept before anything
/// reaches the new target.
pub async fn publish_route(
    dir: &Path,
    route: &Route,
    id: Option<UpstreamId>,
    translate: Option<Translation>,
) -> Result<(UpstreamId, u64), ServiceError> {
    let id = match id {
        Some(id) => id,
        None => default_route_upstream_id(route)
            .ok_or_else(|| ServiceError::BadUpstreamId(route.wrapper_name.clone()))?,
    };
    let spec = route_spec(route, translate).map_err(|e| translate_err(route, e))?;
    let _lock = require_stopped(dir)?;
    if translate == Some(Translation::Codex) {
        CodexAuth::shared("codex", codex::default_home())
            .token()
            .await
            .map_err(|error| ServiceError::Codex {
                route: route.name.clone(),
                error,
            })?;
    } else {
        route_secret::check_for_publish(route)
            .await
            .map_err(|error| ServiceError::RouteSecret {
                route: route.name.clone(),
                error,
            })?;
    }
    let paths = RelayPaths::in_dir(dir);
    let mut config = store::load_config(&paths.config)?.value;
    let generation = config.publish_route(&id, &spec.digest(), route.id, translate);
    store::save_config(&paths.config, &config)?;
    Ok((id, generation))
}

pub fn unpublish(dir: &Path, id: &UpstreamId) -> Result<(), ServiceError> {
    let _lock = require_stopped(dir)?;
    let paths = RelayPaths::in_dir(dir);
    let mut config = store::load_config(&paths.config)?.value;
    if !config.unpublish(id) {
        return Err(ServiceError::NotPublished(id.clone()));
    }
    store::save_config(&paths.config, &config)?;
    Ok(())
}

// ---- devices --------------------------------------------------------------

/// What [`rotate_ca`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RotatedCa {
    pub ca_fingerprint: String,
    /// Live devices revoked by the rotation.
    pub devices_revoked: usize,
}

/// Replace the relay's CA and leaf while no relay serves. Every paired
/// client pinned the old CA, so every pairing ends here: the devices are
/// revoked too, so a token that outlives its pin cannot simply fetch the
/// new CA and carry on. Each device must pair again.
pub fn rotate_ca(dir: &Path, now: std::time::SystemTime) -> Result<RotatedCa, ServiceError> {
    let _lock = require_stopped(dir)?;
    let paths = RelayPaths::in_dir(dir);
    let config = store::load_config(&paths.config)?.value;
    let (devices, _recovery) = DeviceRegistry::open(paths.devices)?;
    // Revoke before minting: a failure in between leaves the old CA with
    // no live device, never a new CA beside live old tokens.
    let mut devices_revoked = 0;
    for d in devices.list() {
        if devices.revoke(&d.id)? == Some(Revocation::Revoked) {
            devices_revoked += 1;
        }
    }
    let names = super::runtime::leaf_names(bind_of(&config), &config);
    let trust = super::tls::rotate_ca(&paths.trust, &names, now).map_err(StartError::Trust)?;
    Ok(RotatedCa {
        ca_fingerprint: trust.ca_fingerprint,
        devices_revoked,
    })
}

/// Revoke while no relay serves. Revoking a device twice is not an error:
/// the second reports [`Revocation::AlreadyRevoked`] and writes nothing.
pub fn revoke(dir: &Path, device_id: &str) -> Result<Revocation, ServiceError> {
    let _lock = require_stopped(dir)?;
    let (devices, recovery) = DeviceRegistry::open(RelayPaths::in_dir(dir).devices)?;
    if recovery.is_some() {
        return Err(ServiceError::DevicesRecovered);
    }
    devices
        .revoke(device_id)?
        .ok_or_else(|| ServiceError::NoSuchDevice(device_id.to_owned()))
}

/// Revoke through the serving relay: blocks the next request and ends
/// the device's in-flight streams.
pub fn revoke_live(state: &RelayState, device_id: &str) -> Result<Revocation, ServiceError> {
    state
        .devices()
        .revoke(device_id)?
        .ok_or_else(|| ServiceError::NoSuchDevice(device_id.to_owned()))
}

/// Where [`revoke_anywhere`] made a revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// No relay was serving: the device file was written here.
    Offline,
    /// The serving relay made it, so it also refuses the device's next
    /// request and has ended its open streams.
    ServingRelay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revoked {
    pub via: Via,
    pub revocation: Revocation,
}

/// Revoke from any process: offline when no relay serves, through the
/// serving relay's revoke socket when one does. The serving process
/// itself uses [`revoke_live`].
pub async fn revoke_anywhere(dir: &Path, device_id: &str) -> Result<Revoked, ServiceError> {
    let offline = |revocation| Revoked {
        via: Via::Offline,
        revocation,
    };
    match revoke(dir, device_id) {
        Err(ServiceError::Serving) => {}
        done => return done.map(offline),
    }
    match revoke_socket::revoke(dir, device_id).await {
        Ok(Some(revocation)) => Ok(Revoked {
            via: Via::ServingRelay,
            revocation,
        }),
        Ok(None) => Err(ServiceError::NoSuchDevice(device_id.to_owned())),
        // The relay may have stopped since the lock said it served, and a
        // stopped relay leaves no socket: the lock decides again.
        Err(e @ (LiveRevokeError::NoSocket { .. } | LiveRevokeError::Unreachable { .. })) => {
            match revoke(dir, device_id) {
                Err(ServiceError::Serving) => Err(e.into()),
                done => done.map(offline),
            }
        }
        Err(e) => Err(e.into()),
    }
}

/// A pairing code to show once, with the fingerprint the client checks.
#[derive(Debug, Clone)]
pub struct PairOffer {
    pub code: String,
    pub expires_in: std::time::Duration,
    pub ca_fingerprint: String,
}

/// Open a pairing window on the serving relay, replacing any open one.
pub fn pair_code(state: &RelayState, include_future: bool) -> Result<PairOffer, ServiceError> {
    let control = state.control().ok_or(ServiceError::NoPairing)?;
    let offered = control.pairing.offer(include_future, Instant::now());
    Ok(PairOffer {
        code: offered.code,
        expires_in: offered.expires_in,
        ca_fingerprint: control.ca_fingerprint.clone(),
    })
}

// ---- serving --------------------------------------------------------------

/// A publication that is installed but not ready at start. It is still
/// served — health reports it not-ready and clients route around it —
/// and the caller surfaces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartWarning {
    pub upstream: UpstreamId,
    /// `owner_conflict`, `identity_changed`, `credential_unavailable`,
    /// …, or `account_missing` when the account is no longer registered.
    pub code: String,
}

/// A serving relay and what its start found. The serve lock is held by
/// `running.state`, so the relay counts as serving until
/// [`Running::stop_and_drain`] has returned and every other reference to
/// that state is gone.
pub struct Served {
    pub running: Running,
    pub ca_fingerprint: Option<String>,
    pub resets: Vec<Reset>,
    pub warnings: Vec<StartWarning>,
    /// Publications whose target changed since publishing — an account's
    /// identity, or a route's origin, models or key location; their
    /// generation was bumped, so devices must accept again.
    pub republished: Vec<UpstreamId>,
}

impl std::fmt::Debug for Served {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Served")
            .field("addr", &self.running.addr)
            .field("warnings", &self.warnings)
            .field("republished", &self.republished)
            .finish_non_exhaustive()
    }
}

/// Serve this machine's relay with production credentials.
pub async fn serve(dir: &Path, accounts: &[Account]) -> Result<Served, ServiceError> {
    serve_with(dir, |account_id| {
        accounts
            .iter()
            .find(|a| a.uuid == account_id)
            .map(|a| Arc::new(StoreBackend::new(a.uuid, a.email.clone())) as Arc<dyn LeaseBackend>)
    })
    .await
}

/// Serve with an injected credential backend per account; routes come
/// from `dir`'s route store.
pub async fn serve_with(
    dir: &Path,
    backend_for: impl Fn(Uuid) -> Option<Arc<dyn LeaseBackend>>,
) -> Result<Served, ServiceError> {
    serve_with_routes(
        dir,
        backend_for,
        |target, route, translate, store| match translate {
            None => PassThroughHandler::new(target, route.clone(), dir.join(ROUTES_FILENAME))
                .ok()
                .map(RouteHandler::PassThrough),
            Some(t) => TranslateHandler::new(
                target,
                route.clone(),
                t,
                store.clone(),
                dir.join(ROUTES_FILENAME),
            )
            .ok()
            .map(RouteHandler::Translate),
        },
    )
    .await
}

/// The handler a route upstream is served by.
pub enum RouteHandler {
    PassThrough(PassThroughHandler),
    Translate(TranslateHandler),
}

impl RouteHandler {
    async fn check_credential(&self) -> Result<(), UpstreamError> {
        match self {
            RouteHandler::PassThrough(h) => h.check_credential().await,
            RouteHandler::Translate(h) => h.check_credential().await,
        }
    }

    fn into_upstream(self) -> Arc<dyn UpstreamHandler> {
        match self {
            RouteHandler::PassThrough(h) => Arc::new(h),
            RouteHandler::Translate(h) => Arc::new(h),
        }
    }
}

/// [`serve_with`] with an injected route handler factory, for tests that
/// point a route at a loopback gateway. Every translated upstream of one
/// serve shares one continuation store (it is keyed by device and target),
/// and it lives in memory only: a restart loses it, and a replay that
/// needed it is refused as `continuation_state_lost`.
pub async fn serve_with_routes(
    dir: &Path,
    backend_for: impl Fn(Uuid) -> Option<Arc<dyn LeaseBackend>>,
    route_handler: impl Fn(
        TargetIdentity,
        &Route,
        Option<Translation>,
        &Arc<ContinuationStore>,
    ) -> Option<RouteHandler>,
) -> Result<Served, ServiceError> {
    let plan = plan_serve(dir, backend_for, route_handler).await?;
    let ca_fingerprint = plan
        .prepared
        .trust
        .as_ref()
        .map(|(t, _)| t.ca_fingerprint.clone());
    let resets = plan.prepared.resets.clone();
    let running = runtime::start(plan.prepared).await?;
    Ok(Served {
        running,
        ca_fingerprint,
        resets,
        warnings: plan.warnings,
        republished: plan.republished,
    })
}

/// A relay ready to bind: everything [`serve_with_routes`] does short of
/// listening. The serve lock is already inside `prepared.state`.
struct ServePlan {
    prepared: runtime::Prepared,
    warnings: Vec<StartWarning>,
    republished: Vec<UpstreamId>,
}

async fn plan_serve(
    dir: &Path,
    backend_for: impl Fn(Uuid) -> Option<Arc<dyn LeaseBackend>>,
    route_handler: impl Fn(
        TargetIdentity,
        &Route,
        Option<Translation>,
        &Arc<ContinuationStore>,
    ) -> Option<RouteHandler>,
) -> Result<ServePlan, ServiceError> {
    let lock = acquire_serve_lock(dir)?;
    let paths = RelayPaths::in_dir(dir);
    let loaded = store::load_config(&paths.config)?;
    // This load is what moves a corrupt file aside, so `runtime::prepare`
    // finds no file and no reset: the reset is carried from here.
    let config_reset = loaded.recovery.is_some();
    let mut config = loaded.value;
    let serving = Serving {
        bind: bind_of(&config),
        port: config.port,
        tls: true,
    };
    // Before any credential is touched: a refused address does nothing.
    listen::check(serving.bind, serving.tls)?;

    let PreparedUpstreams {
        handlers,
        warnings,
        republished,
    } = prepare_upstreams(dir, &mut config, backend_for, route_handler).await?;
    if !republished.is_empty() {
        store::save_config(&paths.config, &config)?;
    }
    let mut prepared = runtime::prepare(lock, serving, handlers, SystemTime::now())?;
    if config_reset && !prepared.resets.contains(&Reset::Config) {
        prepared.resets.insert(0, Reset::Config);
    }
    Ok(ServePlan {
        prepared,
        warnings,
        republished,
    })
}

/// Every published upstream, made ready to install.
#[derive(Default)]
struct PreparedUpstreams {
    /// In install order: `runtime::prepare` makes the first the default.
    handlers: Vec<Arc<dyn UpstreamHandler>>,
    warnings: Vec<StartWarning>,
    republished: Vec<UpstreamId>,
}

/// What preparing one publication produced.
struct UpstreamPrep {
    /// `None` when it is not served; `warning` then says why.
    handler: Option<Arc<dyn UpstreamHandler>>,
    warning: Option<StartWarning>,
    /// Its target changed since publishing, and its generation was bumped.
    republished: bool,
}

impl UpstreamPrep {
    fn not_served(upstream: &UpstreamId, code: &str) -> Self {
        Self {
            handler: None,
            warning: Some(start_warning(upstream, code)),
            republished: false,
        }
    }
}

fn start_warning(upstream: &UpstreamId, code: &str) -> StartWarning {
    StartWarning {
        upstream: upstream.clone(),
        code: code.to_owned(),
    }
}

async fn prepare_upstreams(
    dir: &Path,
    config: &mut ConfigFile,
    backend_for: impl Fn(Uuid) -> Option<Arc<dyn LeaseBackend>>,
    route_handler: impl Fn(
        TargetIdentity,
        &Route,
        Option<Translation>,
        &Arc<ContinuationStore>,
    ) -> Option<RouteHandler>,
) -> Result<PreparedUpstreams, ServiceError> {
    let mut out = PreparedUpstreams::default();
    let publications: Vec<Publication> = config.published().cloned().collect();
    // Read only when a route is published: an unreadable route store
    // must not stop a relay that serves subscriptions alone.
    let mut routes: Option<Vec<Route>> = None;
    let continuations = Arc::new(ContinuationStore::new(continuation::Limits::default()));
    for p in publications {
        let prep = if let Some(route_id) = p.route_id {
            let routes = match &routes {
                Some(r) => r,
                None => routes.insert(load_routes(dir)?),
            };
            prepare_route(&p, route_id, routes, config, &route_handler, &continuations).await?
        } else if let Some(account_id) = p.account_id {
            prepare_subscription(&p, account_id, backend_for(account_id), config).await?
        } else {
            continue;
        };
        out.handlers.extend(prep.handler);
        out.warnings.extend(prep.warning);
        if prep.republished {
            out.republished.push(p.id.clone());
        }
    }
    // Bare `/v1/…` means the subscription upstream (`target.rs`), and
    // `runtime::prepare` makes the first handler the default: subscriptions
    // go first whatever the publication order. Stable, so each kind keeps
    // publication order.
    out.handlers
        .sort_by_key(|h| h.target().kind != TargetKind::Subscription);
    Ok(out)
}

async fn prepare_route(
    p: &Publication,
    route_id: Uuid,
    routes: &[Route],
    config: &mut ConfigFile,
    route_handler: &impl Fn(
        TargetIdentity,
        &Route,
        Option<Translation>,
        &Arc<ContinuationStore>,
    ) -> Option<RouteHandler>,
    continuations: &Arc<ContinuationStore>,
) -> Result<UpstreamPrep, ServiceError> {
    let Some(route) = routes.iter().find(|r| r.id == route_id) else {
        return Ok(UpstreamPrep::not_served(&p.id, "route_missing"));
    };
    let Ok(spec) = route_spec(route, p.translate) else {
        return Ok(UpstreamPrep::not_served(&p.id, "route_not_relay_capable"));
    };
    let (mut generation, mut digest) = (p.generation, p.digest.clone());
    let now_digest = spec.digest();
    let republished = now_digest != p.digest;
    if republished {
        generation = config.publish_route(&p.id, &now_digest, route_id, p.translate);
        digest = now_digest;
    }
    let target = TargetIdentity {
        id: p.id.clone(),
        fingerprint: digest,
        generation,
        kind: if p.translate.is_some() {
            TargetKind::Translated
        } else {
            TargetKind::PassThrough
        },
    };
    let handler = route_handler(target, route, p.translate, continuations).ok_or_else(|| {
        ServiceError::RouteSecret {
            route: route.name.clone(),
            error: RouteSecretError::Backend("http client".into()),
        }
    })?;
    // Served either way, like an account that is not ready: health
    // reports it and clients route around it.
    let warning = handler
        .check_credential()
        .await
        .err()
        .map(|e| start_warning(&p.id, Refusal::Upstream(e).code()));
    Ok(UpstreamPrep {
        handler: Some(handler.into_upstream()),
        warning,
        republished,
    })
}

async fn prepare_subscription(
    p: &Publication,
    account_id: Uuid,
    backend: Option<Arc<dyn LeaseBackend>>,
    config: &mut ConfigFile,
) -> Result<UpstreamPrep, ServiceError> {
    let Some(backend) = backend else {
        return Ok(UpstreamPrep::not_served(&p.id, "account_missing"));
    };
    let (lease, identity) = subscription_lease(backend, &p.digest).await;
    let (mut generation, mut digest) = (p.generation, p.digest.clone());
    let mut republished = false;
    let mut warning = None;
    match identity {
        Ok(identity) => {
            let now_digest = subscription_digest(&identity);
            if now_digest != p.digest {
                generation = config.publish(&p.id, &now_digest, Some(account_id));
                digest = now_digest;
                republished = true;
            }
        }
        Err(e) => warning = Some(start_warning(&p.id, Refusal::Upstream(e).code())),
    }
    let target = TargetIdentity {
        id: p.id.clone(),
        fingerprint: digest,
        generation,
        kind: TargetKind::Subscription,
    };
    let handler = SubscriptionHandler::new(target, lease)
        .map_err(|_| ServiceError::Credential {
            email: account_id.to_string(),
            error: UpstreamError::Transport,
        })?
        .with_utilization(health::snapshot_source(account_id));
    Ok(UpstreamPrep {
        handler: Some(Arc::new(handler)),
        warning,
        republished,
    })
}

/// What the status event carries about a serving relay.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveStatus {
    pub active_streams: usize,
    pub upstreams: Vec<(UpstreamId, UpstreamStatus)>,
    /// A leaf renewal that failed while serving (`relay_ca_key_unavailable`
    /// and the other `TrustError` codes). The current leaf still serves
    /// until it expires, so this is the user's warning to act first.
    pub trust_fault: Option<&'static str>,
}

impl LiveStatus {
    /// The worst blocking condition across upstreams, if any: these stop
    /// serving and need the user (plan §10, one blocking banner).
    pub fn blocking(&self) -> Option<UpstreamError> {
        let errs = self
            .upstreams
            .iter()
            .filter_map(|(_, s)| s.credential_error);
        let mut found = None;
        for e in errs {
            match e {
                UpstreamError::OwnerConflict => return Some(e),
                UpstreamError::IdentityChanged => found = Some(e),
                _ => {}
            }
        }
        found
    }

    /// The first non-blocking error, for the notification log.
    pub fn last_error(&self) -> Option<UpstreamError> {
        self.upstreams
            .iter()
            .filter_map(|(_, s)| s.credential_error)
            .find(|e| {
                !matches!(
                    e,
                    UpstreamError::OwnerConflict | UpstreamError::IdentityChanged
                )
            })
    }
}

pub async fn live_status(state: &RelayState) -> LiveStatus {
    let mut upstreams = Vec::new();
    for h in state.handlers() {
        upstreams.push((h.target().id.clone(), h.status().await));
    }
    upstreams.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    LiveStatus {
        active_streams: state.active_streams(),
        upstreams,
        trust_fault: state.trust_fault(),
    }
}

#[cfg(test)]
#[path = "tests_service.rs"]
mod tests;

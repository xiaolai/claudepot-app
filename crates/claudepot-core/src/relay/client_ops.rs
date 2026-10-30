//! The client verbs — pair, list, accept, run — as core operations, so
//! the CLI and the GUI share one implementation (`rules/architecture.md`:
//! no business logic outside core).
//!
//! Each verb loads `relay-servers.json`, talks to the relay through
//! [`super::client`] (pinned TLS), and keeps the device token in the OS
//! credential store ([`super::token_store`]); the token never reaches the
//! JSON file.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use super::client::{self, Acceptance, ClientError, FetchedCa, Probe};
use super::launch::{self, LaunchEnv};
use super::secret::Secret;
use super::select::{self, Excluded, Observation, Ranked, Selection};
use super::servers::{self, ServerRecord, ServersFile};
use super::store::{StoreError, DEFAULT_PORT};
use super::target::UpstreamId;
use super::token_store::{self, TokenStoreError};

/// Where device tokens are kept.
#[derive(Clone)]
pub enum Vault {
    /// The OS credential store, under this service name.
    Os { service: String },
    /// In memory. For tests, which must not touch the login keychain.
    Memory(Arc<Mutex<HashMap<String, String>>>),
    /// In memory, but a write of `fail_on` lands and then reports
    /// failure, as a write whose verification read-back failed does.
    #[cfg(test)]
    WriteThenFail {
        tokens: Arc<Mutex<HashMap<String, String>>>,
        fail_on: String,
    },
}

impl Vault {
    async fn read(&self, server_id: &str) -> Result<Option<Secret>, TokenStoreError> {
        match self {
            Vault::Os { service } => token_store::read(service, server_id).await,
            Vault::Memory(m) => Ok(m.lock().get(server_id).cloned().map(Secret::new)),
            #[cfg(test)]
            Vault::WriteThenFail { tokens, .. } => {
                Ok(tokens.lock().get(server_id).cloned().map(Secret::new))
            }
        }
    }

    async fn write(&self, server_id: &str, token: &Secret) -> Result<(), TokenStoreError> {
        match self {
            Vault::Os { service } => token_store::write(service, server_id, token).await,
            Vault::Memory(m) => {
                m.lock()
                    .insert(server_id.to_owned(), token.expose().to_owned());
                Ok(())
            }
            #[cfg(test)]
            Vault::WriteThenFail { tokens, fail_on } => {
                tokens
                    .lock()
                    .insert(server_id.to_owned(), token.expose().to_owned());
                if token.expose() == fail_on.as_str() {
                    Err(TokenStoreError::NotVerified)
                } else {
                    Ok(())
                }
            }
        }
    }

    async fn delete(&self, server_id: &str) -> Result<(), TokenStoreError> {
        match self {
            Vault::Os { service } => token_store::delete(service, server_id).await,
            Vault::Memory(m) => {
                m.lock().remove(server_id);
                Ok(())
            }
            #[cfg(test)]
            Vault::WriteThenFail { tokens, .. } => {
                tokens.lock().remove(server_id);
                Ok(())
            }
        }
    }
}

pub struct ClientCtx {
    pub servers_path: PathBuf,
    /// Parent of the per-upstream CC profiles.
    pub data_dir: PathBuf,
    pub vault: Vault,
}

impl ClientCtx {
    pub fn production() -> Self {
        Self {
            servers_path: servers::servers_path(),
            data_dir: crate::paths::claudepot_data_dir(),
            vault: Vault::Os {
                service: token_store::SERVICE.to_owned(),
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientOpError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Token(#[from] TokenStoreError),
    #[error("{0} is not a relay address (host, host:port, or https://host:port)")]
    BadAddress(String),
    #[error("no paired relay is named {0}")]
    UnknownServer(String),
    #[error("{0} is not an upstream id")]
    BadUpstream(String),
    #[error("no upstream given and no paired relay has a default — pass --upstream")]
    NoUpstream,
    #[error("no device token is stored for relay {0} — pair it again")]
    NoToken(String),
    #[error("no paired relays — run `claudepot relay pair <host>` first")]
    NoServers,
    #[error("no relay can serve {upstream} now:{}", reasons(.excluded))]
    NoEligible {
        upstream: UpstreamId,
        excluded: Vec<Excluded>,
    },
    #[error("the relay recorded no acceptance")]
    NotAccepted,
    /// The pairing's token write or record save failed, and putting the
    /// previous device token back failed too: the credential store may
    /// now hold the new token for a record that does not reference it.
    #[error("{cause}; restoring the previous device token also failed: {restore}")]
    Rollback {
        cause: Box<ClientOpError>,
        restore: TokenStoreError,
    },
    #[error("Claude Code is not installed (no `claude` binary found)")]
    NoClaude,
    #[error(transparent)]
    Spec(#[from] crate::cc_env::errors::CcEnvError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

fn reasons(excluded: &[Excluded]) -> String {
    excluded
        .iter()
        .map(|e| format!("\n  {}: {}", e.name, e.reason))
        .collect()
}

/// A refused address as it may appear in an error: scheme and host:port
/// only. Userinfo becomes `…@` and anything after the authority `/…`,
/// because a rejected address is exactly the one that may carry a
/// password or a token in its query, and errors reach terminals, logs
/// and toasts.
fn redacted_address(s: &str) -> String {
    let (scheme, rest) = match s.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, s),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let mut out = String::new();
    if let Some(scheme) = scheme {
        out.push_str(scheme);
        out.push_str("://");
    }
    match authority.rsplit_once('@') {
        Some((_, host)) => {
            out.push_str("…@");
            out.push_str(host);
        }
        None => out.push_str(authority),
    }
    if !tail.is_empty() {
        out.push_str("/…");
    }
    out
}

fn bad_address(s: &str) -> ClientOpError {
    ClientOpError::BadAddress(redacted_address(s))
}

/// `host`, `host:port`, `https://host[:port]`, or `http://` on loopback.
/// A bare host gets the default port.
pub fn parse_address(s: &str) -> Result<reqwest::Url, ClientOpError> {
    let bad = || bad_address(s);
    let with_scheme = if s.contains("://") {
        s.to_owned()
    } else {
        format!("https://{s}")
    };
    let mut url = reqwest::Url::parse(&with_scheme).map_err(|_| bad())?;
    let ok_scheme = url.scheme() == "https" || servers::is_loopback_http(&url);
    let bare = (url.path() == "/" || url.path().is_empty())
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if !ok_scheme || !bare || url.host_str().is_none() {
        return Err(bad());
    }
    // `Url` drops a scheme's default port, so `https://h:443` reads as
    // port-less; only an address that named no port gets the relay's.
    if url.port().is_none() && !names_a_port(&with_scheme) {
        url.set_port(Some(DEFAULT_PORT)).map_err(|_| bad())?;
    }
    Ok(url)
}

/// Does the authority of `scheme://authority[/…]` carry `:port`?
fn names_a_port(with_scheme: &str) -> bool {
    let rest = with_scheme
        .split_once("://")
        .map_or(with_scheme, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_end = authority.rfind(']').map_or(0, |i| i + 1);
    authority[host_end..].contains(':')
}

/// The name a server shows for this device: the host name.
pub fn default_device_name() -> String {
    whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_owned())
}

/// One paired server as `relay servers` shows it: its probe, and per
/// approved upstream whether a run could use it now and why not.
pub struct ServerView {
    pub record: ServerRecord,
    pub probe: Result<Probe, ProbeFailure>,
    pub upstreams: Vec<(UpstreamId, Result<Ranked, select::Ineligible>)>,
}

pub async fn list(ctx: &ClientCtx, now: DateTime<Utc>) -> Result<Vec<ServerView>, ClientOpError> {
    let (file, probes) = probe_all(ctx).await?;
    Ok(file
        .servers
        .iter()
        .zip(probes)
        .map(|(record, probe)| {
            let obs = [Observation {
                server: record,
                probe: probe.clone().map_err(|f| f.message),
            }];
            let upstreams = record
                .upstreams
                .iter()
                .map(|u| {
                    let mut sel = select::select(&obs, &u.id, None, now);
                    let verdict = match sel.ranked.pop() {
                        Some(r) => Ok(r),
                        None => Err(sel
                            .excluded
                            .pop()
                            .map(|e| e.reason)
                            .unwrap_or(select::Ineligible::NotOffered)),
                    };
                    (u.id.clone(), verdict)
                })
                .collect();
            ServerView {
                record: record.clone(),
                probe,
                upstreams,
            }
        })
        .collect())
}

/// What the user must compare before pairing.
pub struct CaOffer {
    pub base: reqwest::Url,
    pub fetched: FetchedCa,
    /// Computed from the PEM, never the server's claim.
    pub fingerprint: String,
}

/// Step 1 of pairing: fetch the CA (public material) over an unverified
/// connection. Nothing secret crosses it.
pub async fn fetch_ca(address: &str) -> Result<CaOffer, ClientOpError> {
    let base = parse_address(address)?;
    if base.scheme() != "https" {
        // Loopback enrollment is written by the server, not paired.
        return Err(bad_address(address));
    }
    let fetched = client::fetch_ca(&base).await?;
    let fingerprint = fetched.computed_fingerprint()?;
    Ok(CaOffer {
        base,
        fetched,
        fingerprint,
    })
}

/// Step 2: redeem `code` over a connection pinned to the CA the user
/// approved, store the token, record the server and its offered
/// upstreams as approved.
pub async fn complete_pair(
    ctx: &ClientCtx,
    offer: &CaOffer,
    approved_fingerprint: &str,
    code: &str,
    device_name: &str,
    name: Option<&str>,
) -> Result<ServerRecord, ClientOpError> {
    let path = ctx.servers_path.clone();
    let salt = off_runtime(move || servers::persisted_salt(&path)).await?;
    let paired = client::pair(
        &offer.base,
        &offer.fetched,
        approved_fingerprint,
        code,
        device_name,
        &salt,
    )
    .await?;
    let name = name
        .map(str::to_owned)
        .or_else(|| {
            offer
                .base
                .host_str()
                .map(|h| h.trim_matches(['[', ']']).to_owned())
        })
        .unwrap_or_else(|| paired.server_id.clone());
    let mut record = ServerRecord {
        server_id: paired.server_id.clone(),
        name,
        base_url: offer.base.as_str().trim_end_matches('/').to_owned(),
        ca_pem: Some(offer.fetched.ca_pem.clone()),
        ca_fingerprint: Some(offer.fingerprint.clone()),
        device_id: paired.device_id.clone(),
        default_upstream: paired
            .default_upstream
            .as_deref()
            .and_then(UpstreamId::parse),
        upstreams: Vec::new(),
        paired_at: Utc::now(),
    };
    for o in &paired.upstreams {
        if let Some(id) = UpstreamId::parse(&o.id) {
            record.approve(id, o.fingerprint.clone(), o.generation);
        }
    }
    persist_pairing(ctx, record, &paired.token).await
}

/// Run a blocking store call off the async runtime. The servers lock is
/// held across awaits by [`persist_pairing`], so blocking a runtime
/// thread on it could starve the holder.
async fn off_runtime<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, ClientOpError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ClientOpError::Io(std::io::Error::other(format!("store task: {e}"))))?
        .map_err(Into::into)
}

/// Store `token` and `record` as one step under the servers lock: the
/// token swap, the save, and any rollback all happen while the lock is
/// held, so a concurrent pairing cannot have its token restored over or
/// deleted by this one's failure. The record is validated before the
/// token is touched.
async fn persist_pairing(
    ctx: &ClientCtx,
    record: ServerRecord,
    token: &Secret,
) -> Result<ServerRecord, ClientOpError> {
    let path = ctx.servers_path.clone();
    let lock = off_runtime(move || servers::ServersLock::acquire(&path)).await?;
    let mut file = lock.load()?;
    file.upsert(record.clone());
    servers::check(&ctx.servers_path, &file)?;
    // A re-pair overwrites the token of a record that still exists; keep
    // the old one so a failed save puts it back rather than leaving that
    // record with no token at all.
    let previous = ctx.vault.read(&record.server_id).await?;
    // A failed write is not proof nothing was written — a verified write
    // can land and then fail its read-back — so it is rolled back exactly
    // like a failed save.
    let failure: ClientOpError = match ctx.vault.write(&record.server_id, token).await {
        Err(write) => write.into(),
        Ok(()) => match lock.save(&file) {
            Ok(()) => {
                drop(lock);
                return Ok(record);
            }
            Err(save) => save.into(),
        },
    };
    let restored = match &previous {
        Some(old) => ctx.vault.write(&record.server_id, old).await,
        // The token without its record is unreachable; do not leave it.
        None => ctx.vault.delete(&record.server_id).await,
    };
    Err(match restored {
        Ok(()) => failure,
        Err(restore) => ClientOpError::Rollback {
            cause: Box::new(failure),
            restore,
        },
    })
}

fn base_of(record: &ServerRecord) -> Result<reqwest::Url, ClientOpError> {
    reqwest::Url::parse(&record.base_url).map_err(|_| bad_address(&record.base_url))
}

async fn token_for(ctx: &ClientCtx, record: &ServerRecord) -> Result<Secret, ClientOpError> {
    ctx.vault
        .read(&record.server_id)
        .await?
        .ok_or_else(|| ClientOpError::NoToken(record.name.clone()))
}

/// A probe that failed.
///
/// `code` is stable and built from the error's variant alone, so it
/// carries no text the relay or the credential store supplied — a
/// `Refused` holds whatever the server wrote in its error body, and a
/// store backend error holds the tool's output. Surfaces that must not
/// echo such text (the GUI) send `code`; `message` is the `Display` text
/// the CLI and the selector show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeFailure {
    pub code: &'static str,
    pub message: String,
}

impl ProbeFailure {
    fn new(code: &'static str, e: impl std::fmt::Display) -> Self {
        Self {
            code,
            message: e.to_string(),
        }
    }
}

impl std::fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The code for a [`token_for`] failure.
fn token_failure_code(e: &ClientOpError) -> &'static str {
    match e {
        ClientOpError::NoToken(_) => "no_token",
        ClientOpError::Token(TokenStoreError::Locked) => "credential_store_locked",
        _ => "credential_store",
    }
}

async fn probe_one(
    ctx: &ClientCtx,
    salt: &str,
    record: &ServerRecord,
) -> Result<Probe, ProbeFailure> {
    let base = base_of(record).map_err(|e| ProbeFailure::new("bad_address", e))?;
    let token = token_for(ctx, record)
        .await
        .map_err(|e| ProbeFailure::new(token_failure_code(&e), e))?;
    client::probe(&base, record.ca_pem.as_deref(), &token, salt)
        .await
        .map_err(|e| ProbeFailure::new(e.code(), e))
}

/// Every paired server, in pinned order, with its probe. Probed
/// concurrently.
pub async fn probe_all(
    ctx: &ClientCtx,
) -> Result<(ServersFile, Vec<Result<Probe, ProbeFailure>>), ClientOpError> {
    let file = servers::load(&ctx.servers_path)?.value;
    let probes = futures_util::future::join_all(
        file.servers
            .iter()
            .map(|r| probe_one(ctx, &file.client_salt, r)),
    )
    .await;
    Ok((file, probes))
}

/// `server`'s default upstream, for verbs whose `--upstream` is optional.
pub fn default_upstream_for(ctx: &ClientCtx, server: &str) -> Result<String, ClientOpError> {
    let file = servers::load(&ctx.servers_path)?.value;
    let record = file
        .find(server)
        .ok_or_else(|| ClientOpError::UnknownServer(server.to_owned()))?;
    record
        .default_upstream
        .as_ref()
        .map(|u| u.as_str().to_owned())
        .ok_or(ClientOpError::NoUpstream)
}

/// One `POST /relay/accept` for `upstream` on `server`: a query when
/// `expected` is `None`, a recorded acceptance of exactly that value
/// otherwise. Returns the record it went to, for the caller's own check.
async fn request_acceptance(
    ctx: &ClientCtx,
    server: &str,
    upstream: &str,
    expected: Option<&str>,
) -> Result<(ServerRecord, UpstreamId, Acceptance), ClientOpError> {
    let file = servers::load(&ctx.servers_path)?.value;
    let id =
        UpstreamId::parse(upstream).ok_or_else(|| ClientOpError::BadUpstream(upstream.into()))?;
    let record = file
        .find(server)
        .ok_or_else(|| ClientOpError::UnknownServer(server.to_owned()))?
        .clone();
    let pem = record.ca_pem.as_deref().ok_or(ClientError::Unpinned)?;
    let token = token_for(ctx, &record).await?;
    let a = client::accept(
        &base_of(&record)?,
        pem,
        &token,
        id.as_str(),
        expected,
        &file.client_salt,
    )
    .await?;
    Ok((record, id, a))
}

/// Accepted means accepted on **both** sides: the server's grant covers
/// the current target, and this client's own approval — what `relay run`
/// checks — holds the same fingerprint. Either alone is not enough: a
/// remote acceptance whose local save failed leaves `run` refusing a
/// target the server already admits.
fn accepted_on_both_sides(record: &ServerRecord, id: &UpstreamId, a: &Acceptance) -> bool {
    a.accepted
        && record
            .approved(id)
            .is_some_and(|u| u.fingerprint == a.fingerprint)
}

/// Ask the server for `upstream`'s current target (records nothing).
/// `accepted` is true only when the server *and* this client's record
/// already accept it; otherwise the caller should confirm and call
/// [`record_acceptance`], which repairs whichever side is behind.
pub async fn show_acceptance(
    ctx: &ClientCtx,
    server: &str,
    upstream: &str,
) -> Result<Acceptance, ClientOpError> {
    let (record, id, mut a) = request_acceptance(ctx, server, upstream, None).await?;
    a.accepted = accepted_on_both_sides(&record, &id, &a);
    Ok(a)
}

/// Record acceptance of exactly `fingerprint` — the value the user was
/// shown — on the server, then as this client's approval.
pub async fn record_acceptance(
    ctx: &ClientCtx,
    server: &str,
    upstream: &str,
    fingerprint: &str,
) -> Result<Acceptance, ClientOpError> {
    let (record, id, a) = request_acceptance(ctx, server, upstream, Some(fingerprint)).await?;
    if !a.accepted || a.fingerprint != fingerprint {
        return Err(ClientOpError::NotAccepted);
    }
    let (path, server_id) = (ctx.servers_path.clone(), record.server_id);
    let (fp, generation) = (a.fingerprint.clone(), a.generation);
    off_runtime(move || {
        servers::mutate(&path, |f| {
            if let Some(r) = f.find_mut(&server_id) {
                r.approve(id, fp, generation);
            }
        })
    })
    .await?;
    Ok(a)
}

/// `--model X` or `--model=X` among CC's arguments.
pub fn model_hint(args: &[OsString]) -> Option<String> {
    let mut it = args.iter().filter_map(|a| a.to_str());
    while let Some(a) = it.next() {
        if a == "--model" {
            return it.next().map(str::to_owned);
        }
        if let Some(v) = a.strip_prefix("--model=") {
            return Some(v.to_owned());
        }
    }
    None
}

pub struct RunPlan {
    pub upstream: UpstreamId,
    pub pick: Ranked,
    pub selection: Selection,
    pub env: LaunchEnv,
    /// Keeps the CA bundle on disk until CC exits.
    pub ca_bundle: Option<tempfile::NamedTempFile>,
}

/// Probe, score, and build the launch environment. `server` bypasses
/// scoring but not eligibility.
pub async fn plan_run(
    ctx: &ClientCtx,
    server: Option<&str>,
    upstream: Option<&str>,
    model: Option<&str>,
    now: DateTime<Utc>,
) -> Result<RunPlan, ClientOpError> {
    let (file, probes) = probe_all(ctx).await?;
    if file.servers.is_empty() {
        return Err(ClientOpError::NoServers);
    }
    let forced = match server {
        Some(key) => Some(
            file.find(key)
                .ok_or_else(|| ClientOpError::UnknownServer(key.to_owned()))?
                .server_id
                .clone(),
        ),
        None => None,
    };
    let upstream = match upstream {
        Some(u) => UpstreamId::parse(u).ok_or_else(|| ClientOpError::BadUpstream(u.into()))?,
        None => file
            .servers
            .iter()
            .filter(|s| forced.as_ref().is_none_or(|f| &s.server_id == f))
            .find_map(|s| s.default_upstream.clone())
            .ok_or(ClientOpError::NoUpstream)?,
    };
    let observed: Vec<Observation<'_>> = file
        .servers
        .iter()
        .zip(probes)
        .filter(|(s, _)| forced.as_ref().is_none_or(|f| &s.server_id == f))
        .map(|(server, probe)| Observation {
            server,
            probe: probe.map_err(|f| f.message),
        })
        .collect();
    let selection = select::select(&observed, &upstream, model, now);
    let Some(pick) = selection.pick().cloned() else {
        return Err(ClientOpError::NoEligible {
            upstream,
            excluded: selection.excluded,
        });
    };
    let record = file
        .find(&pick.server_id)
        .ok_or_else(|| ClientOpError::UnknownServer(pick.server_id.clone()))?;
    let token = token_for(ctx, record).await?;
    let ca_bundle = match record.ca_pem.as_deref() {
        Some(pem) => Some(launch::write_ca_bundle(pem)?),
        None => None,
    };
    let profile = launch::profile_dir(&ctx.data_dir, &upstream);
    launch::ensure_profile_dir(&profile)?;
    let spec = crate::cc_env::spec::try_spec()?;
    let env = launch::build(
        spec,
        &launch::base_url_for(&base_of(record)?, &upstream),
        token,
        ca_bundle.as_ref().map(|f| f.path()),
        &profile,
    );
    Ok(RunPlan {
        upstream,
        pick,
        selection,
        env,
        ca_bundle,
    })
}

/// Launch Claude Code under `plan` and return its exit code.
pub async fn run(plan: &RunPlan, args: &[OsString]) -> Result<i32, ClientOpError> {
    let binary = crate::fs_utils::find_claude_binary().ok_or(ClientOpError::NoClaude)?;
    Ok(launch::run(&binary, args, &plan.env).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_get_https_and_the_default_port() {
        assert_eq!(
            parse_address("relay.test").unwrap().as_str(),
            "https://relay.test:8787/"
        );
        assert_eq!(
            parse_address("relay.test:9000").unwrap().as_str(),
            "https://relay.test:9000/"
        );
        assert_eq!(
            parse_address("[::1]:9000").unwrap().as_str(),
            "https://[::1]:9000/"
        );
        assert!(parse_address("http://127.0.0.1:8787").is_ok());
        // An explicit default port is kept, not replaced with the relay's.
        let explicit = parse_address("relay.test:443").unwrap();
        assert_eq!(explicit.port_or_known_default(), Some(443));
        let explicit = parse_address("https://[::1]:443").unwrap();
        assert_eq!(explicit.port_or_known_default(), Some(443));
        assert_eq!(
            parse_address("[::1]").unwrap().port_or_known_default(),
            Some(DEFAULT_PORT)
        );
        for bad in [
            "http://relay.test",
            "ftp://relay.test",
            "https://relay.test/path",
            "https://u:p@relay.test",
            "https://relay.test/?q",
            "",
        ] {
            assert!(parse_address(bad).is_err(), "{bad:?} must be refused");
        }
    }

    fn record(id: &str, name: &str, fingerprint: &str) -> ServerRecord {
        let mut r = ServerRecord {
            server_id: id.into(),
            name: name.into(),
            base_url: "https://relay.test:8787".into(),
            ca_pem: Some("-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----\n".into()),
            ca_fingerprint: Some("ab".into()),
            device_id: "dev-1".into(),
            default_upstream: UpstreamId::parse("sub"),
            upstreams: Vec::new(),
            paired_at: Utc::now(),
        };
        r.approve(UpstreamId::parse("sub").unwrap(), fingerprint.into(), 1);
        r
    }

    fn memory_ctx(dir: &std::path::Path) -> (ClientCtx, Arc<Mutex<HashMap<String, String>>>) {
        let tokens = Arc::new(Mutex::new(HashMap::new()));
        let ctx = ClientCtx {
            servers_path: dir.join(servers::SERVERS_FILENAME),
            data_dir: dir.to_owned(),
            vault: Vault::Memory(tokens.clone()),
        };
        (ctx, tokens)
    }

    fn shown(fingerprint: &str, accepted: bool) -> Acceptance {
        Acceptance {
            upstream: "sub".into(),
            fingerprint: fingerprint.into(),
            generation: 2,
            accepted,
        }
    }

    #[test]
    fn a_server_side_acceptance_with_a_stale_local_approval_is_not_accepted() {
        let id = UpstreamId::parse("sub").unwrap();
        // The server recorded it but the local save failed: `run` would
        // still refuse, so `accept` must not report "already accepted".
        assert!(!accepted_on_both_sides(
            &record("s1", "a", "old"),
            &id,
            &shown("new", true)
        ));
        assert!(accepted_on_both_sides(
            &record("s1", "a", "new"),
            &id,
            &shown("new", true)
        ));
        assert!(!accepted_on_both_sides(
            &record("s1", "a", "new"),
            &id,
            &shown("new", false)
        ));
        let mut none = record("s1", "a", "new");
        none.upstreams.clear();
        assert!(!accepted_on_both_sides(&none, &id, &shown("new", true)));
    }

    #[tokio::test]
    async fn a_server_with_no_token_lists_as_unreachable_with_a_stable_code() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _tokens) = memory_ctx(dir.path());
        {
            let lock = servers::ServersLock::acquire(&ctx.servers_path).unwrap();
            let mut file = lock.load().unwrap();
            file.upsert(record("s1", "studio", "fp"));
            lock.save(&file).unwrap();
        }
        let views = list(&ctx, Utc::now()).await.unwrap();
        assert_eq!(views.len(), 1);
        let failure = views[0].probe.as_ref().unwrap_err();
        assert_eq!(failure.code, "no_token");
        // The CLI keeps the full sentence.
        assert_eq!(
            failure.to_string(),
            ClientOpError::NoToken("studio".into()).to_string()
        );
        let (id, verdict) = &views[0].upstreams[0];
        assert_eq!(id.as_str(), "sub");
        assert_eq!(verdict.as_ref().unwrap_err().code(), "unreachable");
    }

    #[test]
    fn a_refusal_code_never_carries_the_relays_text() {
        let e = ClientError::Refused {
            status: 403,
            code: "Bearer sk-ant-oat01-SECRET".into(),
        };
        assert_eq!(e.code(), "refused");
        assert_eq!(
            select::Ineligible::CredentialNotReady(Some("sk-ant-oat01-SECRET".into())).code(),
            "credential_not_ready"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pairing_touches_no_token_until_it_holds_the_servers_lock() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tokens) = memory_ctx(dir.path());
        tokens.lock().insert("s1".into(), "cprl_previous".into());
        let held = servers::ServersLock::acquire(&ctx.servers_path).unwrap();
        let ctx = Arc::new(ctx);
        let task = {
            let ctx = ctx.clone();
            tokio::spawn(async move {
                persist_pairing(&ctx, record("s1", "a", "fp"), &Secret::new("cprl_new")).await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(
            tokens.lock().get("s1").map(String::as_str),
            Some("cprl_previous"),
            "the token was replaced while another holder had the servers lock"
        );
        drop(held);
        let stored = task.await.unwrap().unwrap();
        assert_eq!(stored.server_id, "s1");
        assert_eq!(
            tokens.lock().get("s1").map(String::as_str),
            Some("cprl_new")
        );
        assert!(servers::load(&ctx.servers_path)
            .unwrap()
            .value
            .find("s1")
            .is_some());
    }

    #[tokio::test]
    async fn a_pairing_whose_record_is_invalid_leaves_the_tokens_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tokens) = memory_ctx(dir.path());
        servers::mutate(&ctx.servers_path, |f| f.upsert(record("s1", "taken", "fp"))).unwrap();
        tokens.lock().insert("s1".into(), "cprl_s1".into());
        // A second relay asking for a name another record owns.
        let err = persist_pairing(&ctx, record("s2", "taken", "fp"), &Secret::new("cprl_s2"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClientOpError::Store(StoreError::Invalid { .. })),
            "{err}"
        );
        assert_eq!(tokens.lock().len(), 1);
        assert_eq!(tokens.lock().get("s1").map(String::as_str), Some("cprl_s1"));
    }

    #[test]
    fn a_refused_address_never_echoes_its_credentials_or_query() {
        for bad in [
            "https://alice:hunter2@relay.test",
            "alice:hunter2@relay.test:9000",
            "https://relay.test/?token=hunter2",
            "https://relay.test/hunter2#hunter2",
            "ftp://alice:hunter2@relay.test",
        ] {
            let msg = parse_address(bad).unwrap_err().to_string();
            assert!(!msg.contains("hunter2"), "{bad:?} leaked: {msg}");
            assert!(!msg.contains("alice"), "{bad:?} leaked: {msg}");
            assert!(msg.contains("relay.test"), "{bad:?} lost its host: {msg}");
        }
        // Nothing to hide: the address is shown as typed.
        let msg = parse_address("ftp://relay.test").unwrap_err().to_string();
        assert!(msg.starts_with("ftp://relay.test is not"), "{msg}");
    }

    /// A credential store that applies a write and then reports it failed
    /// — the shape of a `security -i` write whose read-back did not match.
    fn faulty_ctx(
        dir: &std::path::Path,
        fail_on: &str,
    ) -> (ClientCtx, Arc<Mutex<HashMap<String, String>>>) {
        let tokens = Arc::new(Mutex::new(HashMap::new()));
        let ctx = ClientCtx {
            servers_path: dir.join(servers::SERVERS_FILENAME),
            data_dir: dir.to_owned(),
            vault: Vault::WriteThenFail {
                tokens: tokens.clone(),
                fail_on: fail_on.to_owned(),
            },
        };
        (ctx, tokens)
    }

    #[tokio::test]
    async fn a_token_write_that_fails_after_landing_restores_the_previous_token() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tokens) = faulty_ctx(dir.path(), "cprl_new");
        servers::mutate(&ctx.servers_path, |f| f.upsert(record("s1", "a", "fp"))).unwrap();
        tokens.lock().insert("s1".into(), "cprl_previous".into());
        let err = persist_pairing(&ctx, record("s1", "a", "fp2"), &Secret::new("cprl_new"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClientOpError::Token(TokenStoreError::NotVerified)),
            "{err}"
        );
        assert_eq!(
            tokens.lock().get("s1").map(String::as_str),
            Some("cprl_previous"),
            "the existing record lost its working token"
        );
        let saved = servers::load(&ctx.servers_path).unwrap().value;
        assert_eq!(
            saved
                .find("s1")
                .unwrap()
                .approved(&UpstreamId::parse("sub").unwrap())
                .unwrap()
                .fingerprint,
            "fp",
            "the record was saved although its token was not"
        );
    }

    #[tokio::test]
    async fn a_first_pairing_whose_token_write_fails_after_landing_leaves_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, tokens) = faulty_ctx(dir.path(), "cprl_new");
        let err = persist_pairing(&ctx, record("s1", "a", "fp"), &Secret::new("cprl_new"))
            .await
            .unwrap_err();
        assert!(matches!(err, ClientOpError::Token(_)), "{err}");
        assert!(tokens.lock().is_empty(), "an unreachable token was left");
        assert!(servers::load(&ctx.servers_path)
            .unwrap()
            .value
            .find("s1")
            .is_none());
    }

    #[test]
    fn the_model_hint_is_read_from_either_form() {
        let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            model_hint(&a(&["-p", "--model", "opus"])),
            Some("opus".into())
        );
        assert_eq!(model_hint(&a(&["--model=haiku"])), Some("haiku".into()));
        assert_eq!(model_hint(&a(&["--resume"])), None);
    }
}

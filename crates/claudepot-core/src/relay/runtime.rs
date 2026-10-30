//! Assembling a relay server from its stores: the one start path.
//!
//! Order matters and is the point of this module. The bind address is
//! checked before anything is minted or opened, so a refused address
//! leaves no files behind; the config is saved on first start so the
//! `server_id` and `epoch` a client pairs against survive a restart; the
//! CA is loaded, never silently re-minted (`tls::ensure`); and the
//! pairing endpoints exist only when TLS is served, because a pairing
//! code in plaintext is a device token in plaintext.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::cancel::CancelToken;
use super::devices::DeviceRegistry;
use super::ledger::{self, Ledger};
use super::listen::{self, ListenError};
use super::pairing::Pairing;
use super::revoke_socket::{self, LiveRevoke};
use super::serve_lock::ServeLock;
use super::server::{self, ControlPlane, Limits, RelayState};
use super::store::{self, ConfigFile, StoreError};
use super::tls::{self, Ensured, Trust, TrustError, TrustPaths};
use super::upstream::UpstreamHandler;

/// Every file a relay server reads or writes.
#[derive(Debug, Clone)]
pub struct RelayPaths {
    pub config: PathBuf,
    pub devices: PathBuf,
    pub trust: TrustPaths,
    /// The append-only usage ledger.
    pub usage: PathBuf,
}

impl RelayPaths {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            config: dir.join(store::CONFIG_FILENAME),
            devices: dir.join(store::DEVICES_FILENAME),
            trust: TrustPaths::in_dir(dir),
            usage: dir.join(ledger::USAGE_FILENAME),
        }
    }

    pub fn default_location() -> Self {
        Self::in_dir(&crate::paths::claudepot_data_dir())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Listen(#[from] ListenError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Trust(#[from] TrustError),
    #[error("could not listen on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
}

/// What the caller asks for. `tls: false` is refused on every address
/// (`listen::check`); the field stays so a caller cannot mean plaintext
/// silently.
#[derive(Debug, Clone, Copy)]
pub struct Serving {
    pub bind: IpAddr,
    pub port: u16,
    pub tls: bool,
}

/// A store that was unreadable at start and has been moved aside. The
/// caller must surface it: each one means paired devices lost something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    Config,
    Devices,
}

pub struct Prepared {
    pub state: Arc<RelayState>,
    pub config: ConfigFile,
    /// Always `Some` once `prepare` succeeds: plain HTTP is refused.
    pub trust: Option<(Trust, Ensured)>,
    pub resets: Vec<Reset>,
    serving: Serving,
    /// What the serving relay re-checks its leaf against; set with `trust`.
    renewal: Option<Renewal>,
    /// The data directory, where the revoke socket goes.
    dir: PathBuf,
}

/// How often a serving relay checks whether its leaf is due. The margin is
/// thirty days, so a daily-ish check is ample; this one costs a file read.
const LEAF_CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

#[derive(Debug, Clone)]
struct Renewal {
    paths: TrustPaths,
    names: Vec<String>,
    pinned_ca: String,
}

/// One renewal check: swap in a fresh leaf when one was due. `Ok(false)`
/// means the current leaf is still good. On error nothing is swapped.
/// The outcome is also recorded on `state`, where `live_status` reports
/// it, so a failure reaches the user while the old leaf still serves.
fn renew_once(
    state: &RelayState,
    leaf: &tls::LeafResolver,
    renewal: &Renewal,
    now: SystemTime,
) -> Result<bool, TrustError> {
    let result = tls::renew_serving(&renewal.paths, &renewal.names, &renewal.pinned_ca, now)
        .and_then(|fresh| match fresh {
            Some(trust) => leaf.replace(&trust).map(|()| true),
            None => Ok(false),
        });
    state.set_trust_fault(result.as_ref().err().map(TrustError::code));
    result
}

async fn renew_while_serving(
    state: Arc<RelayState>,
    leaf: Arc<tls::LeafResolver>,
    renewal: Renewal,
    shutdown: CancelToken,
) {
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(LEAF_CHECK_EVERY) => {}
        }
        match renew_once(&state, &leaf, &renewal, SystemTime::now()) {
            Ok(true) => tracing::info!("relay TLS leaf renewed while serving"),
            Ok(false) => {}
            // The current leaf keeps serving; `live_status` carries the
            // code to the blocking banner.
            Err(e) => tracing::warn!(code = e.code(), "relay TLS leaf renewal failed"),
        }
    }
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prepared")
            .field("server_id", &self.config.server_id)
            .field("serving", &self.serving)
            .field("resets", &self.resets)
            .finish_non_exhaustive()
    }
}

/// The names the leaf must carry: loopback always (local tools and the
/// health check), the bind address when it is a concrete one — a
/// loopback bind other than `127.0.0.1` included, or a verified client
/// connecting to `::1` would find no matching SAN — and the configured
/// extra names.
pub(crate) fn leaf_names(bind: IpAddr, config: &ConfigFile) -> Vec<String> {
    let mut names = vec!["127.0.0.1".to_owned(), "localhost".to_owned()];
    let bound = bind.to_string();
    if !bind.is_unspecified() && !names.contains(&bound) {
        names.push(bound);
    }
    for n in &config.names {
        if !names.contains(n) {
            names.push(n.clone());
        }
    }
    names
}

/// Load the stores and trust of `lock`'s data dir for `serving`, with
/// `upstreams` installed. The first upstream is the default for bare
/// `/v1/…` paths, so the caller orders them: `service::plan_serve` puts
/// subscriptions first.
///
/// The lock moves into the prepared state and is released with its last
/// reference ([`RelayState`] says why).
pub fn prepare(
    lock: ServeLock,
    serving: Serving,
    upstreams: Vec<Arc<dyn UpstreamHandler>>,
    now: SystemTime,
) -> Result<Prepared, StartError> {
    listen::check(serving.bind, serving.tls)?;

    let dir = lock.dir().to_path_buf();
    let paths = RelayPaths::in_dir(&dir);
    let mut resets = Vec::new();
    let loaded = store::load_config(&paths.config)?;
    if loaded.recovery.is_some() {
        resets.push(Reset::Config);
    }
    let config = loaded.value;
    // Idempotent: persists the identity a fresh default just minted.
    store::save_config(&paths.config, &config)?;

    let (devices, recovery) = DeviceRegistry::open(paths.devices.clone())?;
    if recovery.is_some() {
        resets.push(Reset::Devices);
    }

    let ledger = Arc::new(Ledger::new(
        paths.usage.clone(),
        crate::pricing::PriceBook::load(),
    ));
    let mut state = RelayState::new(devices, Limits::default())
        .with_serve_lock(lock)
        .with_epoch(config.epoch.clone())
        .with_server_id(config.server_id.clone())
        .with_sink(Arc::new(move |rec| {
            // A lost line costs accounting, not service; say so and go on.
            if let Err(e) = ledger.append(&rec, chrono::Utc::now()) {
                tracing::warn!(error = %e, "relay usage line not written");
            }
        }));
    let mut renewal = None;
    let trust = if serving.tls {
        let names = leaf_names(serving.bind, &config);
        let (trust, ensured) = tls::ensure(&paths.trust, &names, now)?;
        renewal = Some(Renewal {
            paths: paths.trust.clone(),
            names,
            pinned_ca: trust.ca_fingerprint.clone(),
        });
        state = state.with_control(Arc::new(ControlPlane {
            server_id: config.server_id.clone(),
            ca_pem: trust.ca_pem.clone(),
            ca_fingerprint: trust.ca_fingerprint.clone(),
            pairing: Pairing::new(),
        }));
        Some((trust, ensured))
    } else {
        None
    };
    for (i, u) in upstreams.into_iter().enumerate() {
        state = state.with_upstream(u, i == 0);
    }
    Ok(Prepared {
        state: Arc::new(state),
        config,
        trust,
        resets,
        serving,
        renewal,
        dir,
    })
}

/// A serving relay.
///
/// [`stop_and_drain`](Self::stop_and_drain) is the stop to report on:
/// it returns once no connection is left. [`stop`](Self::stop) and
/// dropping only signal — connections then close in the background
/// within [`Limits::shutdown_drain`], which is fine for a process that is
/// exiting and wrong for one that goes on to say "stopped" or to release
/// the serve lock.
pub struct Running {
    pub addr: SocketAddr,
    pub state: Arc<RelayState>,
    /// Whether `relay revoke` in another process reaches this relay. The
    /// caller must surface `Unavailable`: revoking then means stopping it.
    pub live_revoke: LiveRevoke,
    shutdown: CancelToken,
    serve_task: Option<tokio::task::JoinHandle<()>>,
    /// Joined by `stop_and_drain` too: a renewal mid-write must finish
    /// before the caller releases the serve lock, or it races a restart
    /// or a CA rotation for the same files.
    renewal_task: Option<tokio::task::JoinHandle<()>>,
    /// Joined for the same reason as `renewal_task`, and so the socket
    /// file is gone by the time the relay reports itself stopped.
    revoke_task: Option<tokio::task::JoinHandle<()>>,
}

impl Running {
    pub fn stop(&self) {
        self.shutdown.cancel();
    }

    /// Stop, and wait until every connection has closed: idle ones at
    /// once, an in-flight response within [`Limits::shutdown_drain`].
    /// Idempotent, and cancellation-safe: each handle is awaited in place
    /// and cleared only once its task has ended, so a drain dropped
    /// mid-wait detaches nothing and the next call waits for the rest.
    pub async fn stop_and_drain(&mut self) {
        self.shutdown.cancel();
        if let Some(task) = self.revoke_task.as_mut() {
            let ended = task.await;
            self.revoke_task = None;
            if let Err(e) = ended {
                tracing::warn!(error = %e, "relay revoke socket task ended abnormally");
            }
        }
        if let Some(task) = self.serve_task.as_mut() {
            let ended = task.await;
            self.serve_task = None;
            if let Err(e) = ended {
                tracing::warn!(error = %e, "relay listener task ended abnormally");
            }
        }
        if let Some(task) = self.renewal_task.as_mut() {
            let ended = task.await;
            self.renewal_task = None;
            if let Err(e) = ended {
                tracing::warn!(error = %e, "relay TLS renewal task ended abnormally");
            }
        }
    }

    #[cfg(test)]
    fn renewal_task_joined(&self) -> bool {
        self.renewal_task.is_none()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// Bind and serve a prepared relay.
pub async fn start(prepared: Prepared) -> Result<Running, StartError> {
    let addr = SocketAddr::new(prepared.serving.bind, prepared.serving.port);
    let leaf = match &prepared.trust {
        Some((trust, _)) => Some(tls::LeafResolver::new(trust)?),
        None => None,
    };
    let acceptor = match &leaf {
        Some(leaf) => Some(TlsAcceptor::from(tls::server_config_with(leaf.clone())?)),
        None => None,
    };
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| StartError::Bind { addr, source })?;
    let addr = listener
        .local_addr()
        .map_err(|source| StartError::Bind { addr, source })?;
    let shutdown = CancelToken::new();
    // Nothing below can fail, so no refused start leaves this listening.
    let (live_revoke, revoke_task) =
        revoke_socket::arm(&prepared.dir, prepared.state.clone(), shutdown.clone());
    let serve_task = tokio::spawn(server::serve(
        listener,
        server::router(prepared.state.clone()),
        acceptor,
        Limits::default(),
        shutdown.clone(),
    ));
    let renewal_task = match (leaf, prepared.renewal) {
        (Some(leaf), Some(renewal)) => Some(tokio::spawn(renew_while_serving(
            prepared.state.clone(),
            leaf,
            renewal,
            shutdown.clone(),
        ))),
        _ => None,
    };
    Ok(Running {
        addr,
        state: prepared.state,
        live_revoke,
        shutdown,
        serve_task: Some(serve_task),
        renewal_task,
        revoke_task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serving(bind: &str, tls: bool) -> Serving {
        Serving {
            bind: bind.parse().unwrap(),
            port: 0,
            tls,
        }
    }

    fn lock_in(dir: &Path) -> ServeLock {
        super::super::service::acquire_serve_lock(dir).unwrap()
    }

    /// What `prepare` left in `dir`. The serve lock is not among them: the
    /// caller takes it before `prepare` runs.
    fn left_behind(dir: &Path) -> Vec<std::ffi::OsString> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != super::super::service::SERVE_LOCK_FILENAME)
            .collect()
    }

    fn tls_prepared(dir: &Path, now: SystemTime) -> (RelayPaths, Prepared) {
        let p = prepare(lock_in(dir), serving("127.0.0.1", true), vec![], now).unwrap();
        (RelayPaths::in_dir(dir), p)
    }

    #[test]
    fn a_serving_relay_renews_its_leaf_under_the_same_ca() {
        let dir = tempfile::tempdir().unwrap();
        let start = SystemTime::now();
        let (_, p) = tls_prepared(dir.path(), start);
        let (trust, _) = p.trust.as_ref().unwrap();
        let leaf = tls::LeafResolver::new(trust).unwrap();
        let renewal = p.renewal.clone().unwrap();
        let before = leaf.leaf_fingerprint().unwrap();

        // Not due yet: nothing changes.
        assert!(!renew_once(&p.state, &leaf, &renewal, start).unwrap());
        assert_eq!(leaf.leaf_fingerprint().unwrap(), before);

        // Inside the renewal margin: a new leaf is served, the CA is the one
        // every paired device pinned.
        let later = start + tls::LEAF_VALIDITY - tls::LEAF_RENEW_MARGIN / 2;
        // A success clears an earlier fault, so the banner goes with it.
        p.state.set_trust_fault(Some("relay_ca_key_unavailable"));
        assert!(renew_once(&p.state, &leaf, &renewal, later).unwrap());
        assert_ne!(leaf.leaf_fingerprint().unwrap(), before);
        assert_eq!(p.state.trust_fault(), None);
        let (reloaded, how) = tls::ensure(&renewal.paths, &renewal.names, later).unwrap();
        assert_eq!(how, Ensured::Loaded);
        assert_eq!(reloaded.ca_fingerprint, renewal.pinned_ca);
    }

    #[test]
    fn a_renewal_without_the_ca_key_fails_closed_and_keeps_the_leaf() {
        let dir = tempfile::tempdir().unwrap();
        let start = SystemTime::now();
        let (paths, p) = tls_prepared(dir.path(), start);
        let (trust, _) = p.trust.as_ref().unwrap();
        let leaf = tls::LeafResolver::new(trust).unwrap();
        let renewal = p.renewal.clone().unwrap();
        let before = leaf.leaf_fingerprint().unwrap();
        std::fs::remove_file(&paths.trust.ca_key).unwrap();

        let later = start + tls::LEAF_VALIDITY - tls::LEAF_RENEW_MARGIN / 2;
        let e = renew_once(&p.state, &leaf, &renewal, later).unwrap_err();
        assert!(matches!(e, TrustError::CaKeyUnavailable { .. }), "{e}");
        assert_eq!(leaf.leaf_fingerprint().unwrap(), before);
        // What the GUI's blocking banner reads, while the old leaf serves.
        assert_eq!(p.state.trust_fault(), Some("relay_ca_key_unavailable"));
    }

    #[test]
    fn a_trust_directory_emptied_while_serving_is_never_reminted() {
        let dir = tempfile::tempdir().unwrap();
        let start = SystemTime::now();
        let (paths, p) = tls_prepared(dir.path(), start);
        let renewal = p.renewal.clone().unwrap();
        let (trust, _) = p.trust.as_ref().unwrap();
        let leaf = tls::LeafResolver::new(trust).unwrap();
        let t = &paths.trust;
        for f in [&t.ca_cert, &t.ca_key, &t.cert, &t.key] {
            std::fs::remove_file(f).unwrap();
        }
        let e = renew_once(&p.state, &leaf, &renewal, start).unwrap_err();
        assert!(matches!(e, TrustError::CaMissing { .. }), "{e}");
        assert!(!paths.trust.ca_cert.exists(), "a CA was minted mid-serve");
    }

    #[test]
    fn a_refused_bind_leaves_no_files_behind() {
        let dir = tempfile::tempdir().unwrap();
        for (bind, tls) in [("192.168.1.5", false), ("8.8.8.8", true)] {
            let lock = lock_in(dir.path());
            let e = prepare(lock, serving(bind, tls), vec![], SystemTime::now()).unwrap_err();
            assert!(matches!(e, StartError::Listen(_)), "{bind}: {e}");
        }
        assert_eq!(left_behind(dir.path()), Vec::<std::ffi::OsString>::new());
    }

    #[test]
    fn plain_loopback_is_refused_and_mints_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for bind in ["127.0.0.1", "::1"] {
            let lock = lock_in(dir.path());
            let e = prepare(lock, serving(bind, false), vec![], SystemTime::now()).unwrap_err();
            assert!(
                matches!(e, StartError::Listen(ListenError::TlsRequired { .. })),
                "{bind}: {e}"
            );
        }
        assert_eq!(left_behind(dir.path()), Vec::<std::ffi::OsString>::new());
    }

    /// `::1` is loopback, and was once left off the leaf because only
    /// `127.0.0.1` was added for loopback: the handshake then failed for
    /// every verifying client. Skipped where the host has no IPv6
    /// loopback (some CI containers).
    #[tokio::test]
    async fn a_relay_bound_to_ipv6_loopback_passes_a_verified_handshake() {
        if std::net::TcpListener::bind("[::1]:0").is_err() {
            eprintln!("no IPv6 loopback on this host; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = prepare(
            lock_in(dir.path()),
            serving("::1", true),
            vec![],
            SystemTime::now(),
        )
        .unwrap();
        let ca_pem = p.trust.as_ref().unwrap().0.ca_pem.clone();
        let mut running = start(p).await.unwrap();
        let url = format!("https://{}/relay/ca", running.addr);
        assert!(url.starts_with("https://[::1]:"), "{url}");
        let resp = super::super::client::pinned_client(&ca_pem)
            .unwrap()
            .get(&url)
            .send()
            .await
            .expect("verified TLS to [::1]");
        assert_eq!(resp.status(), 200);
        running.stop_and_drain().await;
    }

    /// Stop must mean stopped: an open connection is closed, not left to
    /// carry requests after the caller reports "stopped".
    #[tokio::test]
    async fn stop_and_drain_returns_only_after_open_connections_close() {
        use tokio::io::AsyncReadExt;
        let dir = tempfile::tempdir().unwrap();
        let (_, p) = tls_prepared(dir.path(), SystemTime::now());
        let mut running = start(p).await.unwrap();
        // A connection that never completes its handshake: before the
        // drain it would hold on for the whole handshake window.
        let mut open = tokio::net::TcpStream::connect(running.addr).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        tokio::time::timeout(std::time::Duration::from_secs(3), running.stop_and_drain())
            .await
            .expect("the drain is bounded");
        let mut buf = [0u8; 16];
        let n = tokio::time::timeout(std::time::Duration::from_millis(200), open.read(&mut buf))
            .await
            .expect("closed by the time stop returned")
            .unwrap_or(0);
        assert_eq!(n, 0, "the server end is closed");
        assert!(
            tokio::net::TcpStream::connect(running.addr).await.is_err(),
            "nothing listens after stop"
        );
        assert!(
            running.renewal_task_joined(),
            "no renewal can still be writing after stop"
        );
        // Idempotent.
        running.stop_and_drain().await;
    }

    /// A drain that is cancelled (a timed-out caller, a dropped future)
    /// must not detach the tasks it was waiting on: the next drain still
    /// waits for them, or it would report "stopped" while one serves.
    #[tokio::test]
    async fn a_cancelled_drain_leaves_the_next_one_still_waiting() {
        use std::time::Duration;
        let (revoke_done, revoke_rx) = tokio::sync::oneshot::channel::<()>();
        let (serve_done, serve_rx) = tokio::sync::oneshot::channel::<()>();
        let (renew_done, renew_rx) = tokio::sync::oneshot::channel::<()>();
        let mut running = Running {
            addr: "127.0.0.1:0".parse().unwrap(),
            state: Arc::new(RelayState::new(DeviceRegistry::new(), Limits::default())),
            live_revoke: LiveRevoke::Listening,
            shutdown: CancelToken::new(),
            serve_task: Some(tokio::spawn(async move {
                let _ = serve_rx.await;
            })),
            renewal_task: Some(tokio::spawn(async move {
                let _ = renew_rx.await;
            })),
            revoke_task: Some(tokio::spawn(async move {
                let _ = revoke_rx.await;
            })),
        };
        let short = Duration::from_millis(50);
        // Each call is cancelled by its timeout; the one after it must
        // still be waiting.
        for what in ["revoke socket (first)", "revoke socket (after a cancel)"] {
            let r = tokio::time::timeout(short, running.stop_and_drain()).await;
            assert!(r.is_err(), "{what}: returned while the revoke socket ran");
        }
        revoke_done.send(()).unwrap();
        for what in ["listener (first)", "listener (after a cancel)"] {
            let r = tokio::time::timeout(short, running.stop_and_drain()).await;
            assert!(r.is_err(), "{what}: returned while the listener ran");
        }
        serve_done.send(()).unwrap();
        for what in ["renewal (first)", "renewal (after a cancel)"] {
            let r = tokio::time::timeout(short, running.stop_and_drain()).await;
            assert!(r.is_err(), "{what}: returned while the renewal ran");
        }
        renew_done.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), running.stop_and_drain())
            .await
            .expect("drains once both tasks end");
        assert!(running.renewal_task_joined());
    }

    /// The revoke socket lives exactly as long as the relay serves: one
    /// left behind reads to `relay revoke` as a relay that stopped
    /// answering, and none at all would mean revoking needs a stop.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_revoke_socket_listens_while_serving_and_is_gone_after_stop() {
        let dir = tempfile::tempdir().unwrap();
        let (_, p) = tls_prepared(dir.path(), SystemTime::now());
        let socket = revoke_socket::socket_path(dir.path());
        assert!(!socket.exists(), "prepare listens on nothing");
        let mut running = start(p).await.unwrap();
        assert_eq!(running.live_revoke, LiveRevoke::Listening);
        assert!(socket.exists(), "no socket while serving");
        running.stop_and_drain().await;
        assert!(!socket.exists(), "the socket outlived the relay");
    }

    #[test]
    fn identity_survives_a_restart_and_a_reset_is_reported() {
        use super::super::service::is_serving;
        let dir = tempfile::tempdir().unwrap();
        let paths = RelayPaths::in_dir(dir.path());
        let first = prepare(
            lock_in(dir.path()),
            serving("127.0.0.1", true),
            vec![],
            SystemTime::now(),
        )
        .unwrap();
        let fp = first.trust.as_ref().unwrap().0.ca_fingerprint.clone();
        assert!(first.resets.is_empty());
        let (server_id, epoch) = (first.config.server_id.clone(), first.config.epoch.clone());
        assert!(
            is_serving(dir.path()).unwrap(),
            "the prepared state holds the lock"
        );
        drop(first);
        assert!(
            !is_serving(dir.path()).unwrap(),
            "and releases it with itself"
        );

        let again = prepare(
            lock_in(dir.path()),
            serving("127.0.0.1", true),
            vec![],
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(again.config.server_id, server_id);
        assert_eq!(again.config.epoch, epoch);
        assert_eq!(again.state.epoch(), epoch);
        let (trust, ensured) = again.trust.as_ref().unwrap();
        assert_eq!(
            (trust.ca_fingerprint.as_str(), *ensured),
            (fp.as_str(), Ensured::Loaded)
        );
        drop(again);

        std::fs::write(&paths.config, b"{ not json").unwrap();
        std::fs::write(&paths.devices, b"{ not json").unwrap();
        let reset = prepare(
            lock_in(dir.path()),
            serving("127.0.0.1", true),
            vec![],
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(reset.resets, vec![Reset::Config, Reset::Devices]);
        assert_ne!(reset.config.epoch, epoch, "a reset mints a new epoch");
    }

    #[test]
    fn the_leaf_carries_loopback_the_bind_address_and_configured_names() {
        let c = ConfigFile {
            names: vec!["relay.example.ts.net".into(), "localhost".into()],
            ..Default::default()
        };
        assert_eq!(
            leaf_names("100.64.0.9".parse().unwrap(), &c),
            [
                "127.0.0.1",
                "localhost",
                "100.64.0.9",
                "relay.example.ts.net"
            ]
        );
        assert_eq!(leaf_names("0.0.0.0".parse().unwrap(), &c).len(), 3);
        // Every loopback bind other than 127.0.0.1 carries its own SAN.
        for bind in ["::1", "127.0.0.2"] {
            let names = leaf_names(bind.parse().unwrap(), &ConfigFile::default());
            assert_eq!(names, ["127.0.0.1", "localhost", bind], "{bind}");
        }
        assert_eq!(
            leaf_names("127.0.0.1".parse().unwrap(), &ConfigFile::default()),
            ["127.0.0.1", "localhost"]
        );
    }
}

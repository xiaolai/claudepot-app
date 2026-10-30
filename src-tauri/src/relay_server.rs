//! Hosting the relay server inside this process.
//!
//! In-process for the same reason as `remote_server`: the relay serves
//! for as long as the user can see that it does, and **quitting
//! Claudepot stops it** — `stop_on_exit` is the enforcement, the pane's
//! copy is the disclosure. A terminal can still run `claudepot relay
//! serve`; the serve lock makes the two exclusive, and `serving &&
//! !running_here` in the status tells the pane it cannot stop that one.
//!
//! It holds no policy: every verb is `claudepot_core::relay::service`,
//! shared with the CLI.
//!
//! While serving, a ticker emits [`crate::events::RELAY_STATUS`] — the
//! ambient chip and the one blocking banner (`owner_conflict` /
//! `identity_changed`) both read it.

use std::sync::Arc;
use std::time::Duration;

use claudepot_core::relay::devices::Revocation;
use claudepot_core::relay::refusal::Refusal;
use claudepot_core::relay::revoke_socket::LiveRevoke;
use claudepot_core::relay::runtime::Reset;
use claudepot_core::relay::service::{self, LiveStatus, Served, StartWarning};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex as AsyncMutex;

/// How often a serving relay re-reports itself.
const TICK: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Inner {
    served: Option<Served>,
    ticker: Option<tokio::task::JoinHandle<()>>,
    /// Why the last start failed.
    last_error: Option<String>,
    /// Its stable code when the TLS material refused it — the
    /// `relay_ca_key_unavailable` banner reads this, never the text.
    last_error_code: Option<&'static str>,
    warnings: Vec<StartWarning>,
    /// Stores the last successful start found unreadable and set aside.
    /// Kept here because that start consumed the evidence: the file is
    /// gone, so every later `service::status` read reports no recovery.
    resets: Vec<Reset>,
}

/// Tauri-managed. One relay per process (and, by the serve lock, per
/// machine).
#[derive(Default)]
pub struct RelayServerState {
    inner: Arc<AsyncMutex<Inner>>,
}

/// The `relay-status` payload.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatusEvent {
    pub serving: bool,
    pub active_streams: usize,
    /// `owner_conflict` | `identity_changed`: serving is blocked until
    /// the user acts. One banner.
    pub blocking: Option<String>,
    /// Any other upstream error, as a stable code, for the notification
    /// log.
    pub last_error: Option<String>,
    pub upstreams: Vec<UpstreamLiveDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamLiveDto {
    pub id: String,
    pub ready: bool,
    pub error: Option<String>,
}

fn code(e: claudepot_core::relay::upstream::UpstreamError) -> String {
    Refusal::Upstream(e).code().to_owned()
}

impl RelayStatusEvent {
    pub fn stopped() -> Self {
        Self {
            serving: false,
            active_streams: 0,
            blocking: None,
            last_error: None,
            upstreams: Vec::new(),
        }
    }

    pub fn from_live(live: &LiveStatus) -> Self {
        Self {
            serving: true,
            active_streams: live.active_streams,
            blocking: live.blocking().map(code),
            last_error: live.last_error().map(code),
            upstreams: live
                .upstreams
                .iter()
                .map(|(id, s)| UpstreamLiveDto {
                    id: id.as_str().to_owned(),
                    ready: s.credential_ready,
                    error: s.credential_error.map(code),
                })
                .collect(),
        }
    }
}

/// Why a start was refused.
#[derive(Debug)]
pub enum StartFailure {
    Accounts(String),
    Service(service::ServiceError),
}

impl std::fmt::Display for StartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Accounts(m) => f.write_str(m),
            Self::Service(e) => e.fmt(f),
        }
    }
}

/// What the pane needs about this process's relay.
#[derive(Debug, Clone, Default)]
pub struct LocalRelay {
    pub running_here: bool,
    pub addr: Option<String>,
    pub last_error: Option<String>,
    pub last_error_code: Option<&'static str>,
    pub warnings: Vec<StartWarning>,
    /// What the last successful start here set aside; `relay_status`
    /// merges it into its recovery flags.
    pub resets: Vec<Reset>,
    pub live: Option<LiveStatus>,
    /// Whether another process can revoke through the relay serving
    /// here; `None` when nothing serves here.
    pub live_revoke: Option<LiveRevoke>,
}

fn emit(app: &AppHandle, payload: &RelayStatusEvent) {
    if let Err(e) = app.emit(crate::events::RELAY_STATUS, payload) {
        tracing::warn!(error = %e, "relay status emit failed");
    }
}

/// Where status payloads go: the app's event bus in production, a
/// recorder in tests.
type StatusSink = Arc<dyn Fn(&RelayStatusEvent) + Send + Sync>;

fn app_sink(app: AppHandle) -> StatusSink {
    Arc::new(move |payload| emit(&app, payload))
}

/// The production start: this machine's accounts, the default data dir.
async fn serve_production() -> Result<Served, StartFailure> {
    // An unreadable store is a refused start, not an empty one: an
    // empty list would serve nothing and report every publication as
    // `account_missing`, which names the wrong cause.
    let accounts = tokio::task::spawn_blocking(|| {
        crate::commands::open_store()
            .and_then(|s| s.list().map_err(|e| format!("account list failed: {e}")))
    })
    .await
    .map_err(|e| format!("account load task failed: {e}"))
    .and_then(|r| r)
    .map_err(StartFailure::Accounts)?;
    service::serve(&service::default_dir(), &accounts)
        .await
        .map_err(StartFailure::Service)
}

impl RelayServerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn describe(&self) -> LocalRelay {
        let inner = self.inner.lock().await;
        Self::describe_locked(&inner).await
    }

    async fn describe_locked(inner: &Inner) -> LocalRelay {
        let live = match &inner.served {
            Some(s) => Some(service::live_status(&s.running.state).await),
            None => None,
        };
        LocalRelay {
            running_here: inner.served.is_some(),
            addr: inner.served.as_ref().map(|s| s.running.addr.to_string()),
            last_error: inner.last_error.clone(),
            // A start refused for its trust, or a renewal that failed
            // while serving: the same blocking banner either way.
            last_error_code: inner
                .last_error_code
                .or(live.as_ref().and_then(|l| l.trust_fault)),
            warnings: inner.warnings.clone(),
            resets: inner.resets.clone(),
            live,
            live_revoke: inner.served.as_ref().map(|s| s.running.live_revoke),
        }
    }

    /// [`describe`](Self::describe) and a blocking read taken together
    /// under the lifecycle lock, so no start or stop here lands between
    /// them. `relay_status` pairs this process's state with the
    /// machine-wide read; taken apart, a start in between produced
    /// `serving && !running_here` — "a terminal owns it" — for our own
    /// relay.
    pub async fn snapshot<T, F>(&self, read: F) -> Result<(LocalRelay, T), tokio::task::JoinError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let inner = self.inner.lock().await;
        let local = Self::describe_locked(&inner).await;
        let read = tokio::task::spawn_blocking(read).await?;
        drop(inner);
        Ok((local, read))
    }

    /// Start serving. Starting while already running here is not an
    /// error; it returns the running address.
    pub async fn start(&self, app: AppHandle) -> Result<String, StartFailure> {
        self.start_with(serve_production, app_sink(app)).await
    }

    /// [`start`](Self::start) with the serve step and the status sink
    /// injected. The whole transition runs under `inner`'s lock, so a
    /// concurrent Start returns the address of the one relay and a
    /// concurrent Stop waits for it rather than racing it.
    async fn start_with<F, Fut>(&self, serve: F, sink: StatusSink) -> Result<String, StartFailure>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Served, StartFailure>>,
    {
        let mut inner = self.inner.lock().await;
        if let Some(s) = &inner.served {
            return Ok(s.running.addr.to_string());
        }
        let served = match serve().await {
            Ok(s) => s,
            Err(e) => {
                inner.last_error = Some(e.to_string());
                inner.last_error_code = match &e {
                    StartFailure::Service(se) => se.trust_code(),
                    StartFailure::Accounts(_) => None,
                };
                return Err(e);
            }
        };
        let addr = served.running.addr.to_string();
        let state = served.running.state.clone();
        inner.last_error = None;
        inner.last_error_code = None;
        inner.warnings = served.warnings.clone();
        inner.resets = served.resets.clone();
        inner.served = Some(served);
        inner.ticker = Some(tokio::spawn(async move {
            let mut last: Option<RelayStatusEvent> = None;
            loop {
                let now = RelayStatusEvent::from_live(&service::live_status(&state).await);
                // Re-emit on change, and every tick for the stream count.
                if last.as_ref() != Some(&now) || now.active_streams > 0 {
                    sink(&now);
                    last = Some(now);
                }
                tokio::time::sleep(TICK).await;
            }
        }));
        Ok(addr)
    }

    /// Stop serving. `false` when nothing was running here.
    pub async fn stop(&self, app: &AppHandle) -> bool {
        self.stop_with(app_sink(app.clone())).await
    }

    /// [`stop`](Self::stop) with the status sink injected.
    ///
    /// "Stopped" is reported, and the serve lock released, only after
    /// every connection has closed (`Running::stop_and_drain`, bounded by
    /// the listener's drain). Dropping `Served` alone ended the accept
    /// loop and nothing else: a keep-alive connection could go on
    /// carrying billable requests after the pane said "stopped" — and,
    /// with the lock gone, while a terminal relay started beside it.
    /// The drain runs under `inner`'s lock so a Start racing it waits
    /// instead of failing on the lock the draining relay still holds.
    ///
    /// Once the lifecycle lock is taken, the transition runs in an owned
    /// task: a caller that is cancelled mid-drain (a dropped future, a
    /// timeout) cannot drop `Served` — and with it the serve lock — while
    /// connections still drain, nor leave local state saying nothing
    /// runs here with no "stopped" ever reported.
    async fn stop_with(&self, sink: StatusSink) -> bool {
        let inner = self.inner.clone().lock_owned().await;
        let transition = tokio::spawn(async move {
            let mut inner = inner;
            if let Some(t) = inner.ticker.take() {
                t.abort();
                // Wait for it, so no "serving" payload lands after "stopped".
                let _ = t.await;
            }
            let Some(mut served) = inner.served.take() else {
                return false;
            };
            served.running.stop_and_drain().await;
            drop(served);
            // Reported before the lifecycle lock is released: a Start
            // queued behind this Stop emits "serving" only once it holds
            // the lock, so it can never be overtaken by this "stopped".
            sink(&RelayStatusEvent::stopped());
            drop(inner);
            true
        });
        match transition.await {
            Ok(stopped) => stopped,
            Err(e) => {
                tracing::warn!(error = %e, "relay stop task ended abnormally");
                false
            }
        }
    }

    /// Revoke a device so its in-flight streams end now: through the
    /// live relay when it serves here, through the revoke socket of a
    /// relay serving from a terminal, and offline when nothing serves.
    pub async fn revoke(&self, device_id: &str) -> Result<Revocation, service::ServiceError> {
        self.revoke_in(&service::default_dir(), device_id).await
    }

    /// [`revoke`](Self::revoke) against `dir`. Holding the lifecycle lock
    /// keeps a Start or Stop here from landing between the "who serves"
    /// decision and the revocation.
    async fn revoke_in(
        &self,
        dir: &std::path::Path,
        device_id: &str,
    ) -> Result<Revocation, service::ServiceError> {
        let inner = self.inner.lock().await;
        match &inner.served {
            Some(s) => service::revoke_live(&s.running.state, device_id),
            None => service::revoke_anywhere(dir, device_id)
                .await
                .map(|r| r.revocation),
        }
    }

    pub async fn pair_code(
        &self,
        include_future: bool,
    ) -> Result<service::PairOffer, service::ServiceError> {
        let inner = self.inner.lock().await;
        match &inner.served {
            Some(s) => service::pair_code(&s.running.state, include_future),
            None => Err(service::ServiceError::NoPairing),
        }
    }
}

/// Stop on application exit. The exit path may be past the runtime, so
/// this never awaits: a lock it cannot take now means a start or stop is
/// mid-flight, and process exit releases the socket and the serve lock
/// either way. For the same reason it signals the listener without
/// draining it: every open connection dies with the process.
pub fn stop_on_exit(state: &RelayServerState) {
    if let Ok(mut inner) = state.inner.try_lock() {
        if let Some(t) = inner.ticker.take() {
            t.abort();
        }
        inner.served = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Read;
    use std::path::Path;

    use std::sync::Mutex;

    /// A data dir whose relay listens on an ephemeral loopback port.
    fn data_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        service::set_bind(d.path(), "127.0.0.1", Some(0), None).unwrap();
        d
    }

    fn serve_in(dir: &Path) -> impl std::future::Future<Output = Result<Served, StartFailure>> {
        let dir = dir.to_owned();
        async move {
            service::serve_with(&dir, |_| None)
                .await
                .map_err(StartFailure::Service)
        }
    }

    fn recorder() -> (StatusSink, Arc<Mutex<Vec<RelayStatusEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (
            Arc::new(move |p: &RelayStatusEvent| s.lock().unwrap().push(p.clone())),
            seen,
        )
    }

    #[tokio::test]
    async fn a_failed_start_leaves_the_relay_stopped_with_its_reason() {
        let state = RelayServerState::new();
        let (sink, seen) = recorder();
        let e = state
            .start_with(
                || async { Err(StartFailure::Accounts("account list failed: x".into())) },
                sink.clone(),
            )
            .await
            .unwrap_err();
        assert!(matches!(e, StartFailure::Accounts(_)));
        let d = state.describe().await;
        assert!(!d.running_here);
        assert!(d.addr.is_none());
        assert_eq!(d.last_error.as_deref(), Some("account list failed: x"));
        assert_eq!(d.last_error_code, None);
        assert!(!state.stop_with(sink.clone()).await, "nothing to stop");
        assert!(
            seen.lock().unwrap().is_empty(),
            "a failed start reports nothing"
        );

        // The next good start clears the reason.
        let dir = data_dir();
        state
            .start_with(|| serve_in(dir.path()), sink.clone())
            .await
            .unwrap();
        let d = state.describe().await;
        assert!(d.running_here);
        assert_eq!(d.last_error, None);
        assert!(state.stop_with(sink.clone()).await);
    }

    /// The order this pins: the connection is closed and the serve lock
    /// free *before* "stopped" is reported — not merely the accept loop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_reports_stopped_only_after_connections_close_and_the_lock_is_free() {
        let dir = data_dir();
        let state = RelayServerState::new();
        let (sink, _) = recorder();
        let addr: std::net::SocketAddr = state
            .start_with(|| serve_in(dir.path()), sink.clone())
            .await
            .unwrap()
            .parse()
            .unwrap();
        assert!(service::is_serving(dir.path()).unwrap());
        // An accepted connection that never finishes its TLS handshake.
        let open = std::net::TcpStream::connect(addr).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        open.set_nonblocking(true).unwrap();

        let at_stop = Arc::new(Mutex::new(None));
        let record = {
            let at_stop = at_stop.clone();
            let dir = dir.path().to_owned();
            let open = open.try_clone().unwrap();
            move |p: &RelayStatusEvent| {
                let mut buf = [0u8; 8];
                let closed = match (&open).read(&mut buf) {
                    Ok(0) => true,
                    Err(e) => e.kind() == std::io::ErrorKind::ConnectionReset,
                    Ok(_) => false,
                };
                let locked = service::is_serving(&dir).unwrap();
                *at_stop.lock().unwrap() = Some((p.serving, closed, locked));
            }
        };
        let stopped =
            tokio::time::timeout(Duration::from_secs(10), state.stop_with(Arc::new(record)))
                .await
                .expect("stop is bounded");
        assert!(stopped);
        assert_eq!(
            *at_stop.lock().unwrap(),
            Some((false, true, false)),
            "(serving, connection closed, serve lock held) when stopped was emitted"
        );
        assert!(!state.describe().await.running_here);
    }

    /// A Stop whose caller goes away mid-transition still finishes it:
    /// the serve lock is not released, nor local state cleared, before
    /// the drain completes and "stopped" is reported. Each trial drops
    /// the Stop future after a different number of polls, so one of them
    /// lands inside the drain.
    #[tokio::test]
    async fn a_cancelled_stop_still_drains_and_reports_stopped() {
        use std::future::Future;
        use std::task::Poll;
        for polls in 1..=8 {
            let dir = data_dir();
            let state = RelayServerState::new();
            let (sink, seen) = recorder();
            let addr: std::net::SocketAddr = state
                .start_with(|| serve_in(dir.path()), sink.clone())
                .await
                .unwrap()
                .parse()
                .unwrap();
            let _open = std::net::TcpStream::connect(addr).unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;

            let mut stop = Box::pin(state.stop_with(sink.clone()));
            let mut finished = false;
            for _ in 0..polls {
                let ready =
                    std::future::poll_fn(|cx| Poll::Ready(stop.as_mut().poll(cx).is_ready())).await;
                if ready {
                    finished = true;
                    break;
                }
                tokio::task::yield_now().await;
            }
            drop(stop);
            if finished {
                continue;
            }

            // Waits on the lifecycle lock for the transition, if any.
            let d = tokio::time::timeout(Duration::from_secs(10), state.describe())
                .await
                .expect("the stop transition is bounded");
            assert!(
                !d.running_here,
                "after {polls} polls: a started stop must finish"
            );
            assert!(
                seen.lock().unwrap().iter().any(|p| !p.serving),
                "after {polls} polls: \"stopped\" was never reported"
            );
            assert!(
                !service::is_serving(dir.path()).unwrap(),
                "after {polls} polls: serve lock still held"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_starts_and_stops_never_leave_two_relays() {
        let dir = data_dir();
        let state = Arc::new(RelayServerState::new());
        let (sink, _) = recorder();
        // Two starts at once: one relay, one address.
        let (a, b) = tokio::join!(
            state.start_with(|| serve_in(dir.path()), sink.clone()),
            state.start_with(|| serve_in(dir.path()), sink.clone()),
        );
        assert_eq!(a.unwrap(), b.unwrap());

        let mut tasks = Vec::new();
        for i in 0..12 {
            let state = state.clone();
            let sink = sink.clone();
            let dir = dir.path().to_owned();
            tasks.push(tokio::spawn(async move {
                if i % 2 == 0 {
                    // `Serving` here would mean a second relay raced the
                    // first for the lock.
                    state
                        .start_with(|| serve_in(&dir), sink)
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                } else {
                    state.stop_with(sink.clone()).await;
                    Ok(())
                }
            }));
        }
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        state.stop_with(sink.clone()).await;
        assert!(!state.describe().await.running_here);
        assert!(!service::is_serving(dir.path()).unwrap());
    }

    /// A Start queued behind a Stop must not be overtaken by that Stop's
    /// "stopped": the subscriber keeps the last payload, so a late
    /// "stopped" would hide a relay that is serving.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_start_queued_behind_a_stop_reports_serving_last() {
        let dir = data_dir();
        let state = Arc::new(RelayServerState::new());
        let seen = Arc::new(Mutex::new(Vec::<bool>::new()));
        // "stopped" lands slowly, as a busy event bus would deliver it.
        let sink: StatusSink = {
            let seen = seen.clone();
            Arc::new(move |p: &RelayStatusEvent| {
                if !p.serving {
                    std::thread::sleep(Duration::from_millis(400));
                }
                seen.lock().unwrap().push(p.serving);
            })
        };
        state
            .start_with(|| serve_in(dir.path()), sink.clone())
            .await
            .unwrap();

        // Queue Stop, then Start, behind a held lock (tokio's mutex is
        // FIFO), so Start is waiting the moment Stop finishes.
        let guard = state.inner.lock().await;
        let stop = {
            let (state, sink) = (state.clone(), sink.clone());
            tokio::spawn(async move { state.stop_with(sink.clone()).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let start = {
            let (state, sink, dir) = (state.clone(), sink.clone(), dir.path().to_owned());
            tokio::spawn(async move { state.start_with(|| serve_in(&dir), sink).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(guard);
        assert!(stop.await.unwrap());
        start.await.unwrap().unwrap();
        // The new relay's ticker reports on its first iteration.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen.last(),
            Some(&true),
            "the restarted relay must be the last word: {seen:?}"
        );
        state.stop_with(sink.clone()).await;
    }

    /// `relay_status` pairs this process's state with the machine-wide
    /// read. A start landing between the two said a terminal owned our
    /// own relay (`serving && !running_here`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_snapshot_never_straddles_a_start() {
        let dir = data_dir();
        let state = Arc::new(RelayServerState::new());
        let (sink, _) = recorder();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let snap = {
            let state = state.clone();
            let d = dir.path().to_owned();
            tokio::spawn(async move {
                state
                    .snapshot(move || {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        service::is_serving(&d).unwrap()
                    })
                    .await
                    .unwrap()
            })
        };
        tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
            .await
            .unwrap();
        // Local state is taken; a Start now must wait for the read.
        let start = {
            let (state, sink, d) = (state.clone(), sink.clone(), dir.path().to_owned());
            tokio::spawn(async move { state.start_with(|| serve_in(&d), sink).await })
        };
        tokio::time::sleep(Duration::from_millis(300)).await;
        release_tx.send(()).unwrap();
        let (local, machine_serving) = snap.await.unwrap();
        assert_eq!(
            local.running_here, machine_serving,
            "local and machine-wide halves disagree"
        );
        start.await.unwrap().unwrap();
        state.stop_with(sink.clone()).await;
    }

    /// The start that sets an unreadable store aside is the only read
    /// that sees it; `service::status` afterwards finds a fresh store.
    /// The reset must outlive that start or the pane never says so.
    #[tokio::test]
    async fn a_store_set_aside_at_start_is_still_reported_after_it() {
        let dir = data_dir();
        let paths = claudepot_core::relay::runtime::RelayPaths::in_dir(dir.path());
        std::fs::write(&paths.devices, b"{ not json").unwrap();
        let state = RelayServerState::new();
        let (sink, _) = recorder();
        state
            .start_with(|| serve_in(dir.path()), sink.clone())
            .await
            .unwrap();

        let read = service::status(dir.path(), &[], chrono::Utc::now()).unwrap();
        assert!(!read.devices_recovered, "the start consumed the evidence");
        assert_eq!(state.describe().await.resets, vec![Reset::Devices]);
        state.stop_with(sink.clone()).await;
        assert_eq!(
            state.describe().await.resets,
            vec![Reset::Devices],
            "kept after stop, like the start's warnings"
        );

        // A clean start clears it.
        state
            .start_with(|| serve_in(dir.path()), sink.clone())
            .await
            .unwrap();
        assert!(state.describe().await.resets.is_empty());
        state.stop_with(sink.clone()).await;
    }

    /// The pane's Revoke works while a terminal relay serves: it goes
    /// through that relay, so the device is refused at once, and falls
    /// back to the file once nothing serves.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_revoke_here_reaches_a_relay_serving_from_a_terminal() {
        let dir = data_dir();
        // Served by core directly: this process's state knows nothing of it.
        let mut terminal = serve_in(dir.path()).await.unwrap();
        let devices = terminal.running.state.devices();
        let laptop = devices.issue("laptop", vec![]).unwrap();
        let phone = devices.issue("phone", vec![]).unwrap();
        let state = RelayServerState::new();
        assert!(!state.describe().await.running_here);

        assert_eq!(
            state
                .revoke_in(dir.path(), &laptop.device_id)
                .await
                .unwrap(),
            Revocation::Revoked
        );
        assert!(
            devices.authenticate(laptop.plaintext.expose()).is_none(),
            "the serving relay refuses it now, not at its next start"
        );
        assert_eq!(
            state
                .revoke_in(dir.path(), &laptop.device_id)
                .await
                .unwrap(),
            Revocation::AlreadyRevoked
        );
        terminal.running.stop_and_drain().await;
        drop(terminal);

        assert_eq!(
            state.revoke_in(dir.path(), &phone.device_id).await.unwrap(),
            Revocation::Revoked,
            "offline once nothing serves"
        );
    }

    /// The pane says when a relay serving here cannot be revoked through
    /// from a terminal; that needs the state to carry it, and only while
    /// it serves.
    #[tokio::test]
    async fn the_relay_serving_here_reports_whether_a_terminal_can_revoke_through_it() {
        use claudepot_core::relay::revoke_socket::Unavailable;
        let dir = data_dir();
        let state = RelayServerState::new();
        let (sink, _) = recorder();
        assert_eq!(state.describe().await.live_revoke, None);
        state
            .start_with(|| serve_in(dir.path()), sink.clone())
            .await
            .unwrap();
        let expected = if cfg!(unix) {
            LiveRevoke::Listening
        } else {
            LiveRevoke::Unavailable(Unavailable::Unsupported)
        };
        assert_eq!(state.describe().await.live_revoke, Some(expected));
        state.stop_with(sink.clone()).await;
        assert_eq!(state.describe().await.live_revoke, None);
    }

    /// What `useRelayErrorNotifications` logs is each upstream's
    /// `credential_error`, by its stable code; a cooldown (a 429) is not
    /// an error in this payload. The frontend tests feed these codes.
    #[test]
    fn an_upstream_credential_error_reaches_the_payload_as_its_code() {
        use claudepot_core::relay::health::UpstreamStatus;
        use claudepot_core::relay::target::UpstreamId;
        use claudepot_core::relay::upstream::UpstreamError;
        let up = |id: &str, error: Option<UpstreamError>, cooldown: Option<Duration>| {
            (
                UpstreamId::parse(id).unwrap(),
                UpstreamStatus {
                    credential_ready: error.is_none(),
                    credential_error: error,
                    cooldown,
                    ..UpstreamStatus::default()
                },
            )
        };
        let live = LiveStatus {
            active_streams: 0,
            upstreams: vec![
                up("a", Some(UpstreamError::Transport), None),
                up("b", None, Some(Duration::from_secs(30))),
                up("c", Some(UpstreamError::OwnerConflict), None),
            ],
            trust_fault: None,
        };
        let p = RelayStatusEvent::from_live(&live);
        let errors: Vec<_> = p
            .upstreams
            .iter()
            .map(|u| (u.id.as_str(), u.error.as_deref()))
            .collect();
        assert_eq!(
            errors,
            vec![
                ("a", Some("upstream_unreachable")),
                ("b", None),
                ("c", Some("owner_conflict"))
            ]
        );
        assert_eq!(p.blocking.as_deref(), Some("owner_conflict"));
        assert_eq!(p.last_error.as_deref(), Some("upstream_unreachable"));
    }

    #[test]
    fn the_stopped_payload_reports_nothing_serving() {
        let s = RelayStatusEvent::stopped();
        assert!(!s.serving);
        assert_eq!(s.active_streams, 0);
        assert!(s.blocking.is_none());
    }

    #[test]
    fn the_payload_is_camel_case_for_the_subscriber() {
        let json = serde_json::to_value(RelayStatusEvent::stopped()).unwrap();
        for key in [
            "serving",
            "activeStreams",
            "blocking",
            "lastError",
            "upstreams",
        ] {
            assert!(json.get(key).is_some(), "useRelayStatus reads `{key}`");
        }
    }
}

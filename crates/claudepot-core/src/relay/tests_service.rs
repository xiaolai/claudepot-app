use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::*;
use crate::relay::lease::mock::{self, MockBackend};
use crate::testing::make_account;

fn up(s: &str) -> UpstreamId {
    UpstreamId::parse(s).unwrap()
}

/// A data dir whose relay listens on an ephemeral loopback port, so
/// parallel tests never collide on 8787.
fn dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    set_bind(d.path(), "127.0.0.1", Some(0), None).unwrap();
    d
}

fn backend() -> Arc<MockBackend> {
    MockBackend::new("sk-ant-oat01-test", 3600)
}

async fn published(d: &Path, account: &Account, b: Arc<MockBackend>) -> u64 {
    publish_with(d, account, None, b).await.unwrap().1
}

/// A keyed gateway route that `publish_route` would accept while stopped.
fn gateway_route() -> Route {
    use crate::routes::{AuthScheme, GatewayConfig, RouteProvider};
    Route {
        id: uuid::Uuid::from_u128(0x9a),
        name: "Gateway".into(),
        provider: RouteProvider::Gateway(GatewayConfig {
            base_url: "https://gateway.example.com".into(),
            api_key: "sk-gateway-test".into(),
            auth_scheme: AuthScheme::Bearer,
            enable_tool_search: false,
            use_keychain: false,
        }),
        model: "gw-model".into(),
        small_fast_model: None,
        additional_models: vec![],
        wrapper_name: "claude-gw".into(),
        deployment_organization_uuid: uuid::Uuid::nil(),
        active_on_desktop: false,
        installed_on_cli: true,
        is_private_cloud: false,
        capabilities_override: None,
    }
}

fn serve_mock(
    d: &Path,
    b: Arc<MockBackend>,
) -> impl std::future::Future<Output = Result<Served, ServiceError>> + '_ {
    serve_with(d, move |_| Some(b.clone() as Arc<dyn LeaseBackend>))
}

/// Stop the way a caller that goes on to act as "stopped" must: drain,
/// then drop. A bare drop only signals, and the serve lock stays held
/// until the listener's last connection lets go of the state.
async fn stop(mut served: Served) {
    served.running.stop_and_drain().await;
}

#[test]
fn eligibility_refuses_the_cc_active_account_and_one_without_credentials() {
    let mut a = make_account("a@example.com");
    assert_eq!(eligibility(&a), Ok(()));
    a.is_cli_active = true;
    assert_eq!(eligibility(&a), Err(Ineligible::CcActive));
    a.is_cli_active = false;
    a.has_cli_credentials = false;
    assert_eq!(eligibility(&a), Err(Ineligible::NoCredentials));
}

#[tokio::test]
async fn publishing_refuses_the_cc_active_account_even_when_the_flag_is_stale() {
    let d = dir();
    let a = make_account("a@example.com");
    // The store says not active; CC's own credential store says it is.
    let b = backend();
    b.cc_owns.store(true, Ordering::SeqCst);
    let err = publish_with(d.path(), &a, None, b).await.unwrap_err();
    assert!(
        matches!(
            err,
            ServiceError::Ineligible {
                reason: Ineligible::CcActive,
                ..
            }
        ),
        "{err:?}"
    );
    let s = status(d.path(), &[a], Utc::now()).unwrap();
    assert!(s.publications.is_empty(), "nothing was published");
}

#[tokio::test]
async fn republishing_the_same_identity_keeps_the_generation_and_a_new_one_bumps_it() {
    let d = dir();
    let a = make_account("a@example.com");
    let b = backend();
    let g1 = published(d.path(), &a, b.clone()).await;
    assert_eq!(published(d.path(), &a, b.clone()).await, g1);
    *b.identity.lock() = mock::identity("another-account");
    assert!(published(d.path(), &a, b).await > g1);
    let s = status(d.path(), std::slice::from_ref(&a), Utc::now()).unwrap();
    assert_eq!(s.publications.len(), 1);
    assert_eq!(
        s.publications[0].account_email.as_deref(),
        Some("a@example.com")
    );
}

#[tokio::test]
async fn one_relay_serves_at_a_time_and_offline_mutations_wait_for_it_to_stop() {
    let d = dir();
    let a = make_account("a@example.com");
    let b = backend();
    published(d.path(), &a, b.clone()).await;

    let served = serve_mock(d.path(), b.clone()).await.unwrap();
    assert!(served.warnings.is_empty(), "{:?}", served.warnings);
    assert!(
        served.ca_fingerprint.is_some(),
        "the service always serves TLS"
    );
    assert!(is_serving(d.path()).unwrap());

    assert!(matches!(
        serve_mock(d.path(), b.clone()).await,
        Err(ServiceError::Serving)
    ));
    assert!(matches!(
        set_bind(d.path(), "127.0.0.1", Some(0), None),
        Err(ServiceError::Serving)
    ));
    assert!(matches!(
        publish_with(d.path(), &a, None, b.clone()).await,
        Err(ServiceError::Serving)
    ));
    assert!(matches!(
        unpublish(d.path(), &up("sub")),
        Err(ServiceError::Serving)
    ));
    let route = gateway_route();
    for translate in [None, Some(Translation::Chat)] {
        assert!(matches!(
            publish_route(d.path(), &route, Some(up("gw")), translate).await,
            Err(ServiceError::Serving)
        ));
    }
    assert!(matches!(
        revoke(d.path(), "dev-x"),
        Err(ServiceError::Serving)
    ));
    assert!(matches!(
        rotate_ca(d.path(), std::time::SystemTime::now()),
        Err(ServiceError::Serving)
    ));
    assert!(status(d.path(), &[], Utc::now()).unwrap().serving);

    stop(served).await;
    assert!(!is_serving(d.path()).unwrap());
    unpublish(d.path(), &up("sub")).unwrap();
    // The refusal came from the lock, not from the route.
    publish_route(d.path(), &route, Some(up("gw")), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_ca_rotation_replaces_the_ca_and_ends_every_pairing() {
    let d = dir();
    let a = make_account("a@example.com");
    let b = backend();
    published(d.path(), &a, b.clone()).await;
    let served = serve_mock(d.path(), b.clone()).await.unwrap();
    let old_ca = served.ca_fingerprint.clone().unwrap();
    let devices = served.running.state.devices();
    let kept = devices.issue("laptop", vec![]).unwrap();
    let gone = devices.issue("old", vec![]).unwrap();
    revoke_live(&served.running.state, &gone.device_id).unwrap();
    stop(served).await;

    let r = rotate_ca(d.path(), std::time::SystemTime::now()).unwrap();
    assert_ne!(r.ca_fingerprint, old_ca);
    assert_eq!(
        r.devices_revoked, 1,
        "an already-revoked device is not counted"
    );

    let s = status(d.path(), &[], Utc::now()).unwrap();
    assert_eq!(s.ca_fingerprint.as_deref(), Some(r.ca_fingerprint.as_str()));
    assert!(s.devices.iter().all(|x| x.revoked_at.is_some()));

    // The next start serves the new CA, and the old token is refused there.
    let served = serve_mock(d.path(), b).await.unwrap();
    assert_eq!(
        served.ca_fingerprint.as_deref(),
        Some(r.ca_fingerprint.as_str())
    );
    assert!(served
        .running
        .state
        .devices()
        .authenticate(kept.plaintext.expose())
        .is_none());
}

#[tokio::test]
async fn a_revocation_goes_through_the_serving_relay_and_survives_its_stop() {
    let d = dir();
    let a = make_account("a@example.com");
    let b = backend();
    published(d.path(), &a, b.clone()).await;
    let served = serve_mock(d.path(), b).await.unwrap();
    let issued = served
        .running
        .state
        .devices()
        .issue("laptop", vec![])
        .unwrap();

    revoke_live(&served.running.state, &issued.device_id).unwrap();
    assert!(matches!(
        revoke_live(&served.running.state, "no-such-device"),
        Err(ServiceError::NoSuchDevice(_))
    ));
    stop(served).await;

    let s = status(d.path(), &[], Utc::now()).unwrap();
    let dev = s.devices.iter().find(|x| x.id == issued.device_id).unwrap();
    assert!(dev.revoked_at.is_some(), "the revocation reached the file");
}

/// The serve lock lives as long as the registry it protects. Whatever still
/// holds the serving relay's state — a revoke arriving over the control
/// socket, a GUI call — writes `relay-devices.json` from that memory, so an
/// offline edit admitted beside it would be overwritten by its next write.
#[tokio::test]
async fn the_serve_lock_is_held_while_anything_still_holds_the_serving_relay() {
    let d = dir();
    let served = serve_with(d.path(), |_| None).await.unwrap();
    let issued = served
        .running
        .state
        .devices()
        .issue("laptop", vec![])
        .unwrap();
    let state = served.running.state.clone();
    stop(served).await;

    assert!(matches!(
        revoke(d.path(), &issued.device_id),
        Err(ServiceError::Serving)
    ));
    assert_eq!(
        revoke_live(&state, &issued.device_id).unwrap(),
        Revocation::Revoked
    );

    drop(state);
    assert!(!is_serving(d.path()).unwrap());
    assert_eq!(
        revoke(d.path(), &issued.device_id).unwrap(),
        Revocation::AlreadyRevoked,
        "the live write reached the file"
    );
}

/// `relay revoke` in another shell: the serving relay makes the
/// revocation, so its in-memory list cannot erase it; with no relay
/// serving, the file is written here.
#[cfg(unix)]
#[tokio::test]
async fn a_revoke_from_any_process_goes_to_whoever_owns_the_device_list() {
    let d = dir();
    let served = serve_with(d.path(), |_| None).await.unwrap();
    let devices = served.running.state.devices();
    let live = devices.issue("laptop", vec![]).unwrap();
    let later = devices.issue("phone", vec![]).unwrap();

    let through = |revocation| Revoked {
        via: Via::ServingRelay,
        revocation,
    };
    assert_eq!(
        revoke_anywhere(d.path(), &live.device_id).await.unwrap(),
        through(Revocation::Revoked)
    );
    assert!(devices.authenticate(live.plaintext.expose()).is_none());
    assert_eq!(
        revoke_anywhere(d.path(), &live.device_id).await.unwrap(),
        through(Revocation::AlreadyRevoked)
    );
    assert!(matches!(
        revoke_anywhere(d.path(), "no-such-device").await,
        Err(ServiceError::NoSuchDevice(id)) if id == "no-such-device"
    ));
    stop(served).await;

    let offline = |revocation| Revoked {
        via: Via::Offline,
        revocation,
    };
    assert_eq!(
        revoke_anywhere(d.path(), &later.device_id).await.unwrap(),
        offline(Revocation::Revoked)
    );
    assert_eq!(
        revoke_anywhere(d.path(), &live.device_id).await.unwrap(),
        offline(Revocation::AlreadyRevoked),
        "the serving relay's revocation reached the file"
    );
}

/// A serving relay that cannot be asked revokes nothing, and the file is
/// not written behind its back either.
#[tokio::test]
async fn a_serving_relay_without_a_revoke_socket_is_refused_and_nothing_is_revoked() {
    let d = dir();
    let served = serve_with(d.path(), |_| None).await.unwrap();
    let issued = served
        .running
        .state
        .devices()
        .issue("laptop", vec![])
        .unwrap();
    #[cfg(unix)]
    std::fs::remove_file(revoke_socket::socket_path(d.path())).unwrap();

    let e = revoke_anywhere(d.path(), &issued.device_id)
        .await
        .unwrap_err();
    #[cfg(unix)]
    assert!(
        matches!(
            e,
            ServiceError::LiveRevoke(LiveRevokeError::NoSocket { .. })
        ),
        "{e:?}"
    );
    #[cfg(not(unix))]
    assert!(
        matches!(e, ServiceError::LiveRevoke(LiveRevokeError::Unsupported)),
        "{e:?}"
    );
    assert!(served
        .running
        .state
        .devices()
        .authenticate(issued.plaintext.expose())
        .is_some());
    stop(served).await;
    let s = status(d.path(), &[], Utc::now()).unwrap();
    let dev = s.devices.iter().find(|x| x.id == issued.device_id).unwrap();
    assert!(dev.revoked_at.is_none(), "nothing reached the file");
}

#[tokio::test]
async fn an_account_identity_change_at_restart_bumps_the_generation() {
    let d = dir();
    let a = make_account("a@example.com");
    let b = backend();
    let g1 = published(d.path(), &a, b.clone()).await;

    *b.identity.lock() = mock::identity("another-account");
    let served = serve_mock(d.path(), b).await.unwrap();
    assert_eq!(served.republished, vec![up("sub")]);
    drop(served);

    let s = status(d.path(), &[], Utc::now()).unwrap();
    assert!(s.publications[0].generation > g1);
}

#[tokio::test]
async fn an_owner_conflict_at_start_warns_and_still_serves() {
    let d = dir();
    let a = make_account("a@example.com");
    let b = backend();
    published(d.path(), &a, b.clone()).await;
    b.cc_owns.store(true, Ordering::SeqCst);

    let served = serve_mock(d.path(), b).await.unwrap();
    assert_eq!(served.warnings.len(), 1);
    assert_eq!(served.warnings[0].upstream, up("sub"));
    assert_eq!(served.warnings[0].code, "owner_conflict");
    let live = live_status(&served.running.state).await;
    assert_eq!(live.blocking(), Some(UpstreamError::OwnerConflict));
    assert_eq!(live.active_streams, 0);
}

#[tokio::test]
async fn a_publication_whose_account_is_gone_is_reported_not_served() {
    let d = dir();
    let a = make_account("a@example.com");
    published(d.path(), &a, backend()).await;
    let served = serve_with(d.path(), |_| None).await.unwrap();
    assert_eq!(served.warnings[0].code, "account_missing");
    assert!(live_status(&served.running.state)
        .await
        .upstreams
        .is_empty());
}

#[test]
fn a_recovered_device_file_refuses_to_revoke() {
    let d = dir();
    std::fs::write(d.path().join(store::DEVICES_FILENAME), "{not json").unwrap();
    assert!(matches!(
        revoke(d.path(), "dev-x"),
        Err(ServiceError::DevicesRecovered)
    ));
}

#[test]
fn a_status_read_while_a_relay_serves_never_sets_a_store_aside() {
    // The serving relay holds both stores in memory; a status read that
    // set the file aside would report no devices while the live registry
    // still authorizes them, and the next start would lose every record.
    for name in [store::DEVICES_FILENAME, store::CONFIG_FILENAME] {
        let d = dir();
        let file = d.path().join(name);
        std::fs::write(&file, "{not json").unwrap();
        let held = acquire_serve_lock(d.path()).unwrap();

        let err = status(d.path(), &[], Utc::now()).unwrap_err();
        assert!(
            matches!(err, ServiceError::Store(StoreError::Unreadable { .. })),
            "{name}: {err}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"{not json", "{name}");
        assert!(
            crate::json_store::corrupt_siblings(&file).is_empty(),
            "{name}"
        );

        // Stopped, the read recovers — under the lock — and says so.
        drop(held);
        let s = status(d.path(), &[], Utc::now()).unwrap();
        assert!(s.config_recovered || s.devices_recovered, "{name}");
        assert!(!s.serving);
        assert!(
            !crate::json_store::corrupt_siblings(&file).is_empty(),
            "{name}"
        );
    }
}

#[test]
fn a_refused_bind_address_changes_nothing() {
    let d = dir();
    assert!(matches!(
        set_bind(d.path(), "8.8.8.8", None, None),
        Err(ServiceError::Listen(_))
    ));
    assert!(matches!(
        set_bind(d.path(), "not-an-ip", None, None),
        Err(ServiceError::BadBind(_))
    ));
    let s = status(d.path(), &[], Utc::now()).unwrap();
    assert!(s.bind.is_loopback());
    assert_eq!(s.port, 0);
    assert_eq!(s.exposure, Some(Exposure::Loopback));
}

/// Can this host listen on `addr` at all? `::1` is absent in some CI
/// containers, and macOS routes only `127.0.0.1` of `127/8` by default.
fn can_listen_on(addr: &str) -> bool {
    std::net::TcpListener::bind((addr.parse::<IpAddr>().unwrap(), 0)).is_ok()
}

#[tokio::test]
async fn a_loopback_bind_other_than_127_0_0_1_is_kept_and_served_on() {
    for addr in ["::1", "127.0.0.2"] {
        let d = dir();
        set_bind(d.path(), addr, Some(0), None).unwrap();
        let wanted: IpAddr = addr.parse().unwrap();
        let s = status(d.path(), &[], Utc::now()).unwrap();
        assert_eq!(s.bind, wanted, "the stored address is the one asked for");
        assert_eq!(s.exposure, Some(Exposure::Loopback));

        if !can_listen_on(addr) {
            eprintln!("skipping the serve half for {addr}: this host cannot listen there");
            continue;
        }
        let served = serve_with(d.path(), |_| None).await.unwrap();
        assert_eq!(served.running.addr.ip(), wanted, "served where configured");
    }
    // The default address keeps its implicit form.
    let d = dir();
    set_bind(d.path(), "127.0.0.1", Some(0), None).unwrap();
    let raw = std::fs::read_to_string(d.path().join(store::CONFIG_FILENAME)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(v["bind"].is_null(), "{raw}");
}

#[test]
fn an_unreadable_usage_ledger_is_reported_unavailable_not_as_zero_usage() {
    let d = dir();
    assert!(!status(d.path(), &[], Utc::now()).unwrap().usage_unavailable);
    // A directory where the ledger file should be: present, and unreadable
    // as a file on every platform.
    std::fs::create_dir(RelayPaths::in_dir(d.path()).usage).unwrap();
    let s = status(d.path(), &[], Utc::now()).unwrap();
    assert!(s.usage_unavailable);
}

#[tokio::test]
async fn a_configuration_reset_at_serve_is_reported_among_the_resets() {
    let d = dir();
    std::fs::write(d.path().join(store::CONFIG_FILENAME), "{not json").unwrap();
    // Up to the bind: a recovered config listens on the default port,
    // which a test must not take. The reset is decided before it.
    let plan = plan_serve(d.path(), |_| None, |_, _, _, _| None)
        .await
        .unwrap();
    assert_eq!(plan.prepared.resets, vec![Reset::Config]);
}

/// The key lives in the route itself; no keychain is consulted.
struct NoKeychain;
impl crate::relay::route_secret::KeychainReader for NoKeychain {
    fn read(
        &self,
        _route: crate::routes::RouteId,
    ) -> Result<Option<crate::relay::secret::Secret>, RouteSecretError> {
        Ok(None)
    }
}

#[tokio::test]
async fn bare_requests_default_to_the_subscription_even_when_a_route_was_published_first() {
    let d = dir();
    let route = RouteStore::open_at(d.path().join(ROUTES_FILENAME))
        .unwrap()
        .add(gateway_route())
        .unwrap();
    publish_route(d.path(), &route, Some(up("gw")), None)
        .await
        .unwrap();
    let a = make_account("a@example.com");
    let b = backend();
    published(d.path(), &a, b.clone()).await;

    let served = serve_with_routes(
        d.path(),
        move |_| Some(b.clone() as Arc<dyn LeaseBackend>),
        |target, r, _, _| {
            Some(RouteHandler::PassThrough(PassThroughHandler::with_client(
                target,
                r.clone(),
                reqwest::Client::new(),
                crate::relay::forward::Timeouts::default(),
                Arc::new(NoKeychain),
            )))
        },
    )
    .await
    .unwrap();
    let state = &served.running.state;
    assert_eq!(state.handlers().len(), 2, "both are served");
    assert_eq!(state.default_upstream(), Some(&up("sub")));
}

fn line(device: &str, ts: &str, outcome: &str, cost: Option<f64>) -> LedgerLine {
    let cost = cost
        .map(|usd| serde_json::json!({"usd": usd, "approximate": true, "family_estimate": false}));
    serde_json::from_value(serde_json::json!({
        "schema": 1, "ts": ts, "device": device, "upstream": "sub", "target": "t",
        "generation": 1, "operation": "/v1/messages", "requested_model": null,
        "model": null, "outcome": outcome, "status": 200, "ttfb_ms": null,
        "upstream_request_id": null, "error": null, "bytes": 0, "usage": null,
        "cost": cost,
    }))
    .unwrap()
}

#[test]
fn usage_rolls_up_per_device_for_one_local_day_and_last_seen_spans_all_days() {
    let today = Utc::now();
    let yesterday = today - chrono::Duration::days(1);
    let lines = vec![
        line("d1", &yesterday.to_rfc3339(), "complete", Some(5.0)),
        line("d1", &today.to_rfc3339(), "complete", Some(0.25)),
        line("d1", &today.to_rfc3339(), "failed", None),
        line("d2", &yesterday.to_rfc3339(), "complete", Some(1.0)),
    ];
    let mut r = Rollup::new();
    for l in &lines {
        roll(&mut r, l, today.with_timezone(&Local).date_naive());
    }
    let (seen, u) = &r["d1"];
    assert_eq!(seen.timestamp(), today.timestamp());
    assert_eq!(u.requests, 2);
    assert_eq!(u.failed, 1);
    assert_eq!(u.unpriced, 1);
    assert!((u.cost_usd - 0.25).abs() < 1e-9);
    let (seen2, u2) = &r["d2"];
    assert_eq!(seen2.timestamp(), yesterday.timestamp());
    assert_eq!(u2, &DayUsage::default());
}

#[test]
fn a_start_refused_for_its_ca_key_carries_the_banner_code_and_nothing_else_does() {
    let locked = ServiceError::Start(StartError::Trust(tls::TrustError::CaKeyUnavailable {
        path: "relay-ca-key.pem".into(),
    }));
    assert_eq!(locked.trust_code(), Some("relay_ca_key_unavailable"));
    assert_eq!(ServiceError::Serving.trust_code(), None);
    assert_eq!(ServiceError::NoPairing.trust_code(), None);
}

// A child process spawned while the lock is held inherits a reference to
// its open file description until it execs; `flock` lasts until the last
// reference closes. `try_clone` is that inherited reference. Without an
// explicit unlock, a Start right after a Stop found "a relay is serving"
// whenever another thread happened to be spawning a process.
#[test]
fn dropping_the_serve_lock_releases_it_while_another_reference_lives() {
    let d = tempfile::tempdir().unwrap();
    let lock = acquire_serve_lock(d.path()).unwrap();
    let inherited = lock.try_clone_file().unwrap();
    drop(lock);
    assert!(!is_serving(d.path()).unwrap());
    drop(inherited);
}

#[tokio::test]
async fn a_failed_start_probe_never_pins_a_different_account_under_the_old_publication() {
    let b = backend();
    let published = subscription_digest(&mock::identity("acct"));
    b.failing_identities.store(1, Ordering::SeqCst);
    let (lease, identity) = subscription_lease(b.clone(), &published).await;
    assert_eq!(identity, Err(UpstreamError::CredentialUnavailable));

    *b.identity.lock() = mock::identity("another-account");
    assert_eq!(
        lease.current().await.unwrap_err(),
        UpstreamError::IdentityChanged
    );
    assert_eq!(lease.readiness().identity, None);
}

#[tokio::test]
async fn a_failed_start_probe_recovers_to_the_published_account() {
    let b = backend();
    let published = subscription_digest(&mock::identity("acct"));
    b.failing_identities.store(1, Ordering::SeqCst);
    let (lease, _) = subscription_lease(b.clone(), &published).await;
    lease.current().await.unwrap();
    assert_eq!(lease.readiness().identity, Some(mock::identity("acct")));
}

use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;

use super::serve::{arm_with, Owner};
use super::wire::{self, Code, Line, Reply};
use super::*;
use crate::relay::cancel::CancelToken;
use crate::relay::devices::{DeviceRegistry, IssuedToken, Revocation};
use crate::relay::server::{Limits, RelayState};
use crate::relay::store::{self, DEVICES_FILENAME};

struct Fixture {
    _dir: tempfile::TempDir,
    data: PathBuf,
    state: Arc<RelayState>,
    issued: IssuedToken,
    /// The device's cancel token: it fires only when the revocation
    /// is on disk.
    cancel: CancelToken,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    let (devices, _) = DeviceRegistry::open(data.join(DEVICES_FILENAME)).unwrap();
    let issued = devices.issue("phone", vec![]).unwrap();
    let (_, cancel) = devices.authenticate(issued.plaintext.expose()).unwrap();
    let state = Arc::new(RelayState::new(devices, Limits::default()));
    Fixture {
        _dir: dir,
        data,
        state,
        issued,
        cancel,
    }
}

impl Fixture {
    fn id(&self) -> &str {
        &self.issued.device_id
    }

    fn arm(&self, owner: Owner) -> (LiveRevoke, Option<JoinHandle<()>>, CancelToken) {
        let shutdown = CancelToken::new();
        let (live, task) = arm_with(&self.data, self.state.clone(), shutdown.clone(), owner);
        (live, task, shutdown)
    }

    /// Untouched in memory, on disk, and in its in-flight streams.
    fn assert_untouched(&self) {
        let token = self.issued.plaintext.expose();
        assert!(
            self.state.devices().authenticate(token).is_some(),
            "revoked in memory"
        );
        assert!(!self.cancel.is_cancelled(), "its streams were ended");
        assert!(!self.revoked_on_disk(), "revoked on disk");
    }

    fn revoked_on_disk(&self) -> bool {
        let file = store::load_devices(&self.data.join(DEVICES_FILENAME)).unwrap();
        let device = file.value.devices.into_iter().find(|d| d.id == self.id());
        device.unwrap().revoked_at.is_some()
    }
}

fn me() -> Owner {
    Owner {
        dir: euid(),
        peer: euid(),
    }
}

fn is_socket(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket())
}

/// Every wait here is bounded, so a broken relay fails its test instead
/// of hanging the suite.
async fn within<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .unwrap_or_else(|_| panic!("timed out: {what}"))
}

async fn stop(shutdown: CancelToken, task: Option<JoinHandle<()>>) {
    shutdown.cancel();
    within("the listener stopping", task.expect("no listener task"))
        .await
        .expect("the listener task panicked");
}

/// Read one reply line, parsed.
async fn reply_on(s: &mut UnixStream) -> Option<Reply> {
    match within("a reply", wire::read_line(s, 4096)).await.unwrap() {
        Line::Whole(line) => wire::parse_reply(&line),
        other => panic!("no reply line: {other:?}"),
    }
}

/// Send raw bytes, optionally half-close, and read the one reply line.
async fn exchange(data: &Path, bytes: &[u8], close_write: bool) -> Option<Reply> {
    let mut s = UnixStream::connect(socket_path(data)).await.unwrap();
    s.write_all(bytes).await.unwrap();
    if close_write {
        s.shutdown().await.unwrap();
    }
    reply_on(&mut s).await
}

#[tokio::test]
async fn a_revoke_through_the_socket_ends_the_device_and_a_repeat_says_so() {
    let f = fixture();
    let shutdown = CancelToken::new();
    let (live, task) = arm(&f.data, f.state.clone(), shutdown.clone());
    assert_eq!(live, LiveRevoke::Listening);
    let mode = std::fs::metadata(f.data.join(DIR_NAME))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);

    assert_eq!(
        revoke(&f.data, f.id()).await.unwrap(),
        Some(Revocation::Revoked)
    );
    assert!(f.cancel.is_cancelled(), "its streams were not ended");
    let token = f.issued.plaintext.expose();
    assert!(f.state.devices().authenticate(token).is_none());
    assert!(f.revoked_on_disk());

    let again = revoke(&f.data, f.id()).await.unwrap();
    assert_eq!(again, Some(Revocation::AlreadyRevoked));
    assert_eq!(revoke(&f.data, "no-such-device").await.unwrap(), None);

    stop(shutdown, task).await;
    assert!(
        std::fs::symlink_metadata(socket_path(&f.data)).is_err(),
        "the socket outlived the relay"
    );
}

#[tokio::test]
async fn a_connection_from_another_uid_is_closed_unread() {
    let f = fixture();
    let (live, task, shutdown) = f.arm(Owner {
        dir: euid(),
        peer: euid().wrapping_add(1),
    });
    assert_eq!(live, LiveRevoke::Listening);
    let err = revoke(&f.data, f.id()).await.unwrap_err();
    assert!(
        matches!(
            err,
            LiveRevokeError::NoAnswer | LiveRevokeError::Unreachable { .. }
        ),
        "{err:?}"
    );
    f.assert_untouched();
    stop(shutdown, task).await;
}

#[tokio::test]
async fn the_client_sends_nothing_to_a_socket_another_uid_listens_on() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(DIR_NAME)).unwrap();
    let listener = UnixListener::bind(socket_path(dir.path())).unwrap();
    let fake = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut got = Vec::new();
        s.read_to_end(&mut got).await.unwrap();
        got
    });
    let err = request::request(dir.path(), "dev", euid().wrapping_add(1))
        .await
        .unwrap_err();
    assert!(matches!(err, LiveRevokeError::NotOwner { .. }), "{err:?}");
    let got = within("the fake relay", fake).await.unwrap();
    assert!(got.is_empty(), "the client sent bytes");
}

#[tokio::test]
async fn malformed_requests_are_refused_and_revoke_nothing() {
    let f = fixture();
    let (_, task, shutdown) = f.arm(me());
    let id = f.id();
    let padded = format!(
        r#"{{"v":1,"op":"revoke","device_id":"{id}","pad":"{}"}}"#,
        "x".repeat(1100)
    );
    let cases: Vec<(String, bool, Code)> = vec![
        ("not json\n".into(), false, Code::BadRequest),
        (
            format!("{{\"op\":\"revoke\",\"device_id\":\"{id}\"}}\n"),
            false,
            Code::BadRequest,
        ),
        (
            format!("{{\"v\":\"1\",\"op\":\"revoke\",\"device_id\":\"{id}\"}}\n"),
            false,
            Code::BadRequest,
        ),
        (
            format!("{{\"v\":2,\"op\":\"revoke\",\"device_id\":\"{id}\"}}\n"),
            false,
            Code::UnsupportedVersion,
        ),
        (
            format!("{{\"v\":2,\"op\":\"explode\",\"device_id\":\"{id}\"}}\n"),
            false,
            Code::UnsupportedVersion,
        ),
        (
            format!("{{\"v\":1,\"op\":\"revoke\",\"device_id\":\"{id}\",\"also\":1}}\n"),
            false,
            Code::BadRequest,
        ),
        (
            format!("{{\"v\":1,\"op\":\"restart\",\"device_id\":\"{id}\"}}\n"),
            false,
            Code::BadRequest,
        ),
        (format!("{padded}\n"), false, Code::BadRequest),
        // A whole request but no newline, then end of input.
        (
            format!("{{\"v\":1,\"op\":\"revoke\",\"device_id\":\"{id}\"}}"),
            true,
            Code::BadRequest,
        ),
    ];
    for (line, close_write, code) in cases {
        let reply = exchange(&f.data, line.as_bytes(), close_write).await;
        let shown = &line[..line.len().min(80)];
        assert_eq!(reply, Some(Reply::Failed(code)), "{shown}");
    }
    f.assert_untouched();
    stop(shutdown, task).await;
}

#[tokio::test]
async fn a_stale_socket_is_replaced() {
    let f = fixture();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(f.data.join(DIR_NAME))
        .unwrap();
    drop(std::os::unix::net::UnixListener::bind(socket_path(&f.data)).unwrap());
    assert!(
        is_socket(&socket_path(&f.data)),
        "precondition: a stale socket"
    );

    let (live, task, shutdown) = f.arm(me());
    assert_eq!(live, LiveRevoke::Listening);
    let revoked = request::request(&f.data, f.id(), euid()).await.unwrap();
    assert_eq!(revoked, Some(Revocation::Revoked));
    stop(shutdown, task).await;
}

#[tokio::test]
async fn the_presence_hint_sees_only_a_socket() {
    let f = fixture();
    assert!(!socket_present(&f.data), "nothing there yet");
    let (_, task, shutdown) = f.arm(me());
    assert!(socket_present(&f.data), "listening");
    stop(shutdown, task).await;
    assert!(!socket_present(&f.data), "removed at stop");
    std::fs::write(socket_path(&f.data), "not a socket").unwrap();
    assert!(!socket_present(&f.data), "a plain file is not a socket");
}

#[tokio::test]
async fn a_file_where_the_socket_goes_is_left_alone() {
    let f = fixture();
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(f.data.join(DIR_NAME))
        .unwrap();
    std::fs::write(socket_path(&f.data), "keep").unwrap();
    let (live, task, _) = f.arm(me());
    assert_eq!(live, LiveRevoke::Unavailable(Unavailable::PathOccupied));
    assert!(task.is_none());
    assert_eq!(
        std::fs::read_to_string(socket_path(&f.data)).unwrap(),
        "keep"
    );
}

#[tokio::test]
async fn a_directory_open_to_others_is_refused_and_not_repaired() {
    let f = fixture();
    let dir = f.data.join(DIR_NAME);
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (live, task, _) = f.arm(me());
    assert_eq!(live, LiveRevoke::Unavailable(Unavailable::UnsafeDirectory));
    assert!(task.is_none());
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755, "the directory was chmodded");
    assert!(!is_socket(&socket_path(&f.data)));
}

#[tokio::test]
async fn a_symlinked_directory_is_refused() {
    let f = fixture();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), f.data.join(DIR_NAME)).unwrap();
    let (live, task, _) = f.arm(me());
    assert_eq!(live, LiveRevoke::Unavailable(Unavailable::UnsafeDirectory));
    assert!(task.is_none());
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_directory_owned_by_another_uid_is_refused() {
    let f = fixture();
    let (live, task, _) = f.arm(Owner {
        dir: euid().wrapping_add(1),
        peer: euid(),
    });
    assert_eq!(live, LiveRevoke::Unavailable(Unavailable::UnsafeDirectory));
    assert!(task.is_none());
    assert!(!is_socket(&socket_path(&f.data)));
}

/// Whichever side of the accept the stop lands on, a request that has
/// not arrived whole is answered `stopping` and revokes nothing.
#[tokio::test]
async fn stopping_mid_request_answers_stopping() {
    let f = fixture();
    let (_, task, shutdown) = f.arm(me());
    let mut s = UnixStream::connect(socket_path(&f.data)).await.unwrap();
    s.write_all(br#"{"v":1,"#).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    shutdown.cancel();
    assert_eq!(reply_on(&mut s).await, Some(Reply::Failed(Code::Stopping)));
    stop(shutdown, task).await;
    f.assert_untouched();
}

/// A whole request still queued behind a silent connection when the relay
/// stops is answered `stopping`, never carried out, and the socket is gone
/// once the listener has stopped.
#[tokio::test]
async fn a_queued_request_is_answered_stopping_and_not_carried_out() {
    let f = fixture();
    let (_, task, shutdown) = f.arm(me());
    let _silent = UnixStream::connect(socket_path(&f.data)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut queued = UnixStream::connect(socket_path(&f.data)).await.unwrap();
    queued
        .write_all(wire::request_line(f.id()).as_bytes())
        .await
        .unwrap();
    shutdown.cancel();
    assert_eq!(
        reply_on(&mut queued).await,
        Some(Reply::Failed(Code::Stopping))
    );
    stop(shutdown, task).await;
    f.assert_untouched();
    assert!(std::fs::symlink_metadata(socket_path(&f.data)).is_err());
}

#[tokio::test]
async fn a_data_dir_too_deep_for_a_socket_serves_without_one() {
    let f = fixture();
    let deep = f.data.join("d".repeat(120));
    std::fs::create_dir(&deep).unwrap();
    let shutdown = CancelToken::new();
    let (live, task) = arm_with(&deep, f.state.clone(), shutdown, me());
    assert_eq!(live, LiveRevoke::Unavailable(Unavailable::PathTooLong));
    assert!(task.is_none());
    let err = request::request(&deep, f.id(), euid()).await.unwrap_err();
    assert!(matches!(err, LiveRevokeError::NoSocket { .. }), "{err:?}");
}

/// Run the client against a fake relay that checks the request it got and
/// answers `reply` verbatim, or closes without answering.
async fn client_against(
    reply: Option<&'static str>,
) -> Result<Option<Revocation>, LiveRevokeError> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(DIR_NAME)).unwrap();
    let listener = UnixListener::bind(socket_path(dir.path())).unwrap();
    let fake = tokio::spawn(async move {
        let (mut s, _) = listener.accept().await.unwrap();
        let Line::Whole(line) = wire::read_line(&mut s, 4096).await.unwrap() else {
            panic!("no request line");
        };
        assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1);
        let got: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(got, json!({ "v": 1, "op": "revoke", "device_id": "dev" }));
        if let Some(reply) = reply {
            s.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    let out = request::request(dir.path(), "dev", euid()).await;
    within("the fake relay", fake).await.unwrap();
    out
}

#[tokio::test]
async fn the_client_maps_every_reply() {
    use LiveRevokeError as E;
    let revoked = client_against(Some("{\"v\":1,\"result\":\"revoked\"}\n")).await;
    assert_eq!(revoked.unwrap(), Some(Revocation::Revoked));
    let already = client_against(Some("{\"v\":1,\"result\":\"already_revoked\"}\n")).await;
    assert_eq!(already.unwrap(), Some(Revocation::AlreadyRevoked));
    let none = client_against(Some(
        "{\"v\":1,\"result\":\"error\",\"code\":\"no_such_device\"}\n",
    ))
    .await;
    assert_eq!(none.unwrap(), None);

    let error = |code: &'static str| -> &'static str {
        Box::leak(format!("{{\"v\":1,\"result\":\"error\",\"code\":\"{code}\"}}\n").into())
    };
    let r = client_against(Some(error("store_failed"))).await;
    assert!(matches!(r, Err(E::StoreFailed)), "{r:?}");
    let r = client_against(Some(error("stopping"))).await;
    assert!(matches!(r, Err(E::Stopping)), "{r:?}");
    for code in ["bad_request", "unsupported_version"] {
        let r = client_against(Some(error(code))).await;
        assert!(
            matches!(r, Err(E::Refused { code: c }) if c == code),
            "{r:?}"
        );
    }
    let r = client_against(Some(error("brand_new"))).await;
    assert!(matches!(r, Err(E::Refused { code: "unknown" })), "{r:?}");

    for unreadable in [
        Some("{\"v\":1,\"result\":\"maybe\"}\n"),
        Some("{\"v\":2,\"result\":\"revoked\"}\n"),
        Some("garbage\n"),
        Some("{\"v\":1,\"result\":\"revoked\"}"),
        None,
    ] {
        let r = client_against(unreadable).await;
        assert!(matches!(r, Err(E::NoAnswer)), "{unreadable:?} → {r:?}");
    }
}

#[tokio::test]
async fn no_socket_and_a_dead_socket_are_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    let r = request::request(dir.path(), "dev", euid()).await;
    assert!(matches!(r, Err(LiveRevokeError::NoSocket { .. })), "{r:?}");

    std::fs::create_dir(dir.path().join(DIR_NAME)).unwrap();
    drop(std::os::unix::net::UnixListener::bind(socket_path(dir.path())).unwrap());
    let r = request::request(dir.path(), "dev", euid()).await;
    assert!(
        matches!(r, Err(LiveRevokeError::Unreachable { .. })),
        "{r:?}"
    );
}

#[tokio::test]
async fn a_revocation_that_cannot_be_saved_revokes_nothing() {
    let f = fixture();
    let (_, task, shutdown) = f.arm(me());
    // A directory where the file was: the save fails.
    let file = f.data.join(DEVICES_FILENAME);
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();

    let err = revoke(&f.data, f.id()).await.unwrap_err();
    assert!(matches!(err, LiveRevokeError::StoreFailed), "{err:?}");
    let token = f.issued.plaintext.expose();
    assert!(
        f.state.devices().authenticate(token).is_some(),
        "revoked in memory"
    );
    assert!(!f.cancel.is_cancelled(), "its streams were ended");
    stop(shutdown, task).await;
}

#[tokio::test]
async fn stopping_leaves_a_socket_it_did_not_create() {
    let f = fixture();
    let (_, task, shutdown) = f.arm(me());
    let path = socket_path(&f.data);
    std::fs::remove_file(&path).unwrap();
    let _replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
    stop(shutdown, task).await;
    assert!(
        is_socket(&path),
        "the relay removed a socket it did not create"
    );
}

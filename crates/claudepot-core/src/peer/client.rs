//! The Unix-domain-socket client that hands a prompt to a running
//! Claude Code session.
//!
//! Scope is deliberately one verb. CC's inbox accepts `rename` and the
//! idle-notification control frames too, but those serve peer sessions
//! coordinating with each other, not a human steering one. Adding them
//! later is a frame and a test; adding them now would be speculative
//! surface on a protocol that can move under us.

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

use super::wire::{encode_line, AuthFrame, Priority, UserFrame, MAX_LINE_BYTES, PEER_PROTOCOL};
use super::PeerError;
use crate::session_live::types::PidRecord;

/// Connecting to a live socket on the same machine is either immediate
/// or wrong. A session whose event loop is wedged will accept the
/// connection and never read; the write timeout below covers that.
#[cfg(unix)]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(unix)]
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// An addressable running session.
///
/// Construct from a `PidRecord` rather than by hand: the pid, the
/// session id, and the socket path have to come from the same reading
/// of the same registry file, or they describe different sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerTarget {
    pub pid: u32,
    pub session_id: String,
    pub socket_path: PathBuf,
}

impl PeerTarget {
    /// Refuses a record that cannot be addressed, and says which of the
    /// two reasons applies — they call for different user-facing copy.
    /// "The inbox is off" is a settings problem the user can fix;
    /// "the protocol moved" is our problem and must not be presented as
    /// theirs.
    pub fn from_record(record: &PidRecord) -> Result<Self, PeerError> {
        if let Some(found) = record.peer_protocol {
            if found != PEER_PROTOCOL {
                return Err(PeerError::ProtocolMismatch {
                    pid: record.pid,
                    found,
                    supported: PEER_PROTOCOL,
                });
            }
        }
        let socket_path = record
            .messaging_socket_path
            .as_deref()
            .filter(|p| !p.is_empty())
            .ok_or(PeerError::NoInbox { pid: record.pid })?;

        Ok(Self {
            pid: record.pid,
            session_id: record.session_id.clone(),
            socket_path: PathBuf::from(socket_path),
        })
    }
}

/// What a successful send actually establishes.
///
/// The name is `Handoff`, not `Delivery`, and a live run against CC
/// 2.1.239 showed exactly why: the message arrived, rendered in the
/// session's UI, and was **held for the user's approval** rather than
/// delivered. See the module docs for that gate.
///
/// `uuid` is the id CC dispatches the prompt with *if* it delivers it.
/// Both halves are measured: on delivery it is the `type: "user"`
/// record's own uuid, so it correlates exactly; while held it is absent,
/// because a held message is never dispatched and the held record is a
/// `type: "system"` notice carrying only a text preview. Correlate on
/// the prompt body when you do not yet know which happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub uuid: String,
    pub pid: u32,
    pub session_id: String,
}

/// Hand `text` to the session described by `target`.
///
/// Order matters and is not negotiable: validate, read the key, prove
/// the pid is not recycled, *then* connect. Every check that can be
/// made without touching another process's socket is made first.
pub async fn send_prompt(
    target: &PeerTarget,
    sessions_dir: &Path,
    text: &str,
    priority: Priority,
) -> Result<Handoff, PeerError> {
    if text.is_empty() {
        // CC logs "Ignoring user message with missing or non-string
        // content" and carries on, so an empty send would look like a
        // success and do nothing.
        return Err(PeerError::EmptyPrompt);
    }

    let key = super::key::read_key(sessions_dir, target.pid, &target.socket_path)?;
    verify_owner(target.pid, key.proc_start.as_deref()).await?;

    let uuid = uuid::Uuid::new_v4().to_string();
    let auth = encode_line(&AuthFrame::new(&key.peer_token))?;
    let frame = UserFrame::new(&target.session_id, text, priority, uuid.clone());
    let body = encode_line(&frame)?;

    if body.len() > MAX_LINE_BYTES {
        return Err(PeerError::PromptTooLong {
            bytes: body.len(),
            limit: MAX_LINE_BYTES,
        });
    }

    write_frames(&target.socket_path, target.pid, &[auth, body]).await?;

    Ok(Handoff {
        uuid,
        pid: target.pid,
        session_id: target.session_id.clone(),
    })
}

#[cfg(unix)]
async fn verify_owner(pid: u32, proc_start: Option<&str>) -> Result<(), PeerError> {
    super::key::verify_proc_start(pid, proc_start).await
}

#[cfg(not(unix))]
async fn verify_owner(_pid: u32, _proc_start: Option<&str>) -> Result<(), PeerError> {
    Ok(())
}

#[cfg(unix)]
async fn write_frames(socket: &Path, pid: u32, lines: &[Vec<u8>]) -> Result<(), PeerError> {
    use tokio::io::AsyncWriteExt;
    use tokio::net::UnixStream;

    let connect = tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(socket));
    let mut stream = match connect.await {
        Err(_) => {
            return Err(PeerError::Timeout {
                pid,
                phase: "connect",
            })
        }
        Ok(Err(source)) => {
            return Err(PeerError::Unreachable {
                pid,
                path: socket.display().to_string(),
                source,
            })
        }
        Ok(Ok(stream)) => stream,
    };

    let send = async {
        for line in lines {
            stream.write_all(line).await?;
        }
        stream.flush().await?;
        // CC reads to `end`, so a half-close is how it learns the batch
        // is complete rather than waiting on its line buffer.
        stream.shutdown().await
    };

    match tokio::time::timeout(WRITE_TIMEOUT, send).await {
        Err(_) => Err(PeerError::Timeout {
            pid,
            phase: "write",
        }),
        Ok(Err(source)) => Err(PeerError::Unreachable {
            pid,
            path: socket.display().to_string(),
            source,
        }),
        Ok(Ok(())) => Ok(()),
    }
}

/// Windows CC binds a named pipe (`\\.\pipe\...`) instead of a socket,
/// and derives the key name from a lowercased pipe name rather than the
/// path. That is a different implementation, not a smaller one, so it
/// is refused explicitly rather than approximated.
#[cfg(not(unix))]
async fn write_frames(_socket: &Path, pid: u32, _lines: &[Vec<u8>]) -> Result<(), PeerError> {
    Err(PeerError::UnsupportedPlatform { pid })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pid: u32, socket: Option<&str>, proto: Option<u32>) -> PidRecord {
        PidRecord {
            pid,
            session_id: "sess-1".into(),
            cwd: "/tmp".into(),
            started_at_ms: 0,
            updated_at_ms: None,
            version: None,
            kind: Some("interactive".into()),
            entrypoint: None,
            name: None,
            status: None,
            waiting_for: None,
            messaging_socket_path: socket.map(str::to_string),
            peer_protocol: proto,
        }
    }

    #[test]
    fn target_from_a_normal_record() {
        let t = PeerTarget::from_record(&record(11137, Some("/tmp/cc-socks/11137.sock"), Some(1)))
            .unwrap();
        assert_eq!(t.pid, 11137);
        assert_eq!(t.session_id, "sess-1");
        assert_eq!(t.socket_path, PathBuf::from("/tmp/cc-socks/11137.sock"));
    }

    #[test]
    fn record_without_a_socket_has_no_inbox() {
        let err = PeerTarget::from_record(&record(1, None, Some(1))).unwrap_err();
        assert!(matches!(err, PeerError::NoInbox { .. }));
    }

    #[test]
    fn empty_socket_path_is_no_inbox_not_a_relative_path() {
        let err = PeerTarget::from_record(&record(1, Some(""), Some(1))).unwrap_err();
        assert!(matches!(err, PeerError::NoInbox { .. }));
    }

    #[test]
    fn future_protocol_is_refused_rather_than_guessed() {
        let err =
            PeerTarget::from_record(&record(1, Some("/tmp/cc-socks/1.sock"), Some(2))).unwrap_err();
        assert!(matches!(err, PeerError::ProtocolMismatch { found: 2, .. }));
    }

    #[test]
    fn absent_protocol_field_is_tolerated() {
        // Older CC builds registered sessions without the field. The
        // socket path is the real capability signal.
        assert!(PeerTarget::from_record(&record(1, Some("/tmp/cc-socks/1.sock"), None)).is_ok());
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::path::Path;
        use tokio::io::AsyncReadExt;
        use tokio::net::UnixListener;

        /// Stand up a fake inbox and capture everything one client
        /// writes. This is the only way to test the send path without
        /// a live CC session.
        async fn fake_inbox(path: &Path) -> tokio::task::JoinHandle<String> {
            let listener = UnixListener::bind(path).unwrap();
            tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = String::new();
                stream.read_to_string(&mut buf).await.unwrap();
                buf
            })
        }

        fn seed_key(dir: &Path, pid: u32, socket: &Path, token: &str) {
            let name = super::super::super::key::key_file_name(pid, socket).unwrap();
            std::fs::write(dir.join(name), format!(r#"{{"peerToken":"{token}"}}"#)).unwrap();
        }

        #[tokio::test]
        async fn sends_auth_then_prompt_in_order() {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("s.sock");
            let pid = std::process::id();
            let token = "feedfacefeedfacefeedfacefeedface";
            seed_key(dir.path(), pid, &socket, token);
            let server = fake_inbox(&socket).await;

            let target = PeerTarget {
                pid,
                session_id: "sess-42".into(),
                socket_path: socket.clone(),
            };
            let handoff = send_prompt(&target, dir.path(), "ship it", Priority::Next)
                .await
                .unwrap();

            let got = server.await.unwrap();
            let lines: Vec<&str> = got.lines().collect();
            assert_eq!(lines.len(), 2, "expected exactly auth + user, got {got:?}");

            let auth: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
            assert_eq!(auth["type"], "auth");
            assert_eq!(auth["token"], token);

            let msg: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
            assert_eq!(msg["type"], "user");
            assert_eq!(msg["message"]["role"], "user");
            assert_eq!(msg["message"]["content"], "ship it");
            assert_eq!(msg["priority"], "next");
            // The guard that makes pid reuse non-fatal.
            assert_eq!(msg["session_id"], "sess-42");
            assert_eq!(msg["uuid"], handoff.uuid);
        }

        #[tokio::test]
        async fn empty_prompt_is_refused_before_connecting() {
            let dir = tempfile::tempdir().unwrap();
            // No socket exists: reaching the network would error
            // differently, which is what proves we stopped earlier.
            let target = PeerTarget {
                pid: std::process::id(),
                session_id: "s".into(),
                socket_path: dir.path().join("absent.sock"),
            };
            let err = send_prompt(&target, dir.path(), "", Priority::Next)
                .await
                .unwrap_err();
            assert!(matches!(err, PeerError::EmptyPrompt));
        }

        #[tokio::test]
        async fn a_dead_socket_reports_unreachable() {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("gone.sock");
            let pid = std::process::id();
            seed_key(dir.path(), pid, &socket, "feedfacefeedfacefeedfacefeedface");
            let target = PeerTarget {
                pid,
                session_id: "s".into(),
                socket_path: socket,
            };
            let err = send_prompt(&target, dir.path(), "hi", Priority::Next)
                .await
                .unwrap_err();
            assert!(matches!(err, PeerError::Unreachable { .. }), "got {err:?}");
        }

        #[tokio::test]
        async fn a_prompt_over_the_line_limit_is_refused_locally() {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("s.sock");
            let pid = std::process::id();
            seed_key(dir.path(), pid, &socket, "feedfacefeedfacefeedfacefeedface");
            let _server = fake_inbox(&socket).await;

            let target = PeerTarget {
                pid,
                session_id: "s".into(),
                socket_path: socket,
            };
            let huge = "x".repeat(MAX_LINE_BYTES + 1);
            let err = send_prompt(&target, dir.path(), &huge, Priority::Next)
                .await
                .unwrap_err();
            assert!(
                matches!(err, PeerError::PromptTooLong { .. }),
                "got {err:?}"
            );
        }

        #[tokio::test]
        async fn a_recycled_pid_is_refused_before_connecting() {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("s.sock");
            let pid = std::process::id();
            let name = super::super::super::key::key_file_name(pid, &socket).unwrap();
            std::fs::write(
                dir.path().join(name),
                r#"{"peerToken":"feedfacefeedfacefeedfacefeedface","procStart":"Thu Jan  1 00:00:00 1970"}"#,
            )
            .unwrap();
            let _server = fake_inbox(&socket).await;

            let target = PeerTarget {
                pid,
                session_id: "s".into(),
                socket_path: socket,
            };
            let err = send_prompt(&target, dir.path(), "hi", Priority::Next)
                .await
                .unwrap_err();
            assert!(matches!(err, PeerError::PidRecycled { .. }), "got {err:?}");
        }
    }
}

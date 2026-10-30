//! `relay revoke` while a relay serves: a Unix socket in the data
//! directory that the serving process answers.
//!
//! While a relay serves it owns the device list (`relay-serve.lock`), so
//! the offline [`service::revoke`](super::service::revoke) is refused: a
//! revocation written behind the server's back would neither refuse the
//! device's next request nor end its open streams, and the server's next
//! write would erase it. The serving process listens here instead and
//! revokes through its live device registry, which does both.
//!
//! ## The peer uid is the boundary
//!
//! Revoking is the socket's only verb, and only the user the relay runs
//! as may use it. Both ends check the kernel's peer credentials: the relay
//! closes a connection from another uid without reading it, and the
//! client sends nothing to a socket another uid listens on, so it cannot
//! be steered into revoking through someone else's relay. The
//! `relay-revoke/` directory is created 0700 and checked on every start —
//! refused, never repaired — as defence in depth. A data directory shared
//! with another user (`CLAUDEPOT_DATA_DIR`) lets that user stop live
//! revoke from working, never perform one.
//!
//! ## Protocol
//!
//! One JSON line each way, one request per connection:
//!
//! ```text
//! → {"v":1,"op":"revoke","device_id":"<id>"}
//! ← {"v":1,"result":"revoked"}            it was live; on disk, streams ended
//! ← {"v":1,"result":"already_revoked"}    nothing was written
//! ← {"v":1,"result":"error","code":"<code>"}
//! ```
//!
//! `code` is `no_such_device`, `store_failed`, `bad_request`,
//! `unsupported_version` or `stopping`, and none of them revoked anything.
//! A client that sent its line and got no reply cannot know whether the
//! revocation happened ([`LiveRevokeError::NoAnswer`]); revoking again is
//! safe, because a revoke is idempotent.
//!
//! ## Stopping
//!
//! A request the relay has read in full before it begins to stop is
//! carried out and answered. Any other is answered `stopping` and revokes
//! nothing — including one whose line had arrived but was still queued
//! behind another connection. The socket file is gone before
//! [`Running::stop_and_drain`](super::runtime::Running::stop_and_drain)
//! returns.
//!
//! ## Limits
//!
//! Connections are answered one at a time, so a client of the same user
//! that connects and sends nothing holds the next one back for up to two
//! seconds. Windows has no revoke socket: stop the relay, then revoke.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::task::JoinHandle;

use super::cancel::CancelToken;
use super::devices::Revocation;
use super::server::RelayState;

#[cfg(unix)]
mod request;
#[cfg(unix)]
mod serve;
#[cfg(unix)]
mod wire;

#[cfg(all(test, unix))]
mod tests;

/// The directory under the data directory that holds the socket.
pub const DIR_NAME: &str = "relay-revoke";
const SOCKET_NAME: &str = "socket";

/// Where the relay serving from `data_dir` listens.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_NAME).join(SOCKET_NAME)
}

/// Whether a relay serving from `data_dir` in another process has left a
/// socket to revoke through. A hint for a UI deciding whether to offer
/// revoke, not a promise: [`revoke`] is what finds out, and reports a
/// socket that does not answer.
pub fn socket_present(data_dir: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        std::fs::symlink_metadata(socket_path(data_dir)).is_ok_and(|m| m.file_type().is_socket())
    }
    #[cfg(not(unix))]
    {
        let _ = data_dir;
        false
    }
}

/// Whether another process can revoke through this relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveRevoke {
    Listening,
    Unavailable(Unavailable),
}

/// Why a serving relay has no revoke socket. It serves regardless;
/// revoking from another process then means stopping it first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// Not a Unix platform.
    Unsupported,
    /// The data directory's path is too long for a socket address.
    PathTooLong,
    /// Something other than a socket is where the socket goes.
    PathOccupied,
    /// `relay-revoke/` is not a directory of this user's with mode 0700.
    UnsafeDirectory,
    /// The directory or the socket could not be created, and why.
    Bind(std::io::ErrorKind),
}

impl Unavailable {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::PathTooLong => "path_too_long",
            Self::PathOccupied => "path_occupied",
            Self::UnsafeDirectory => "unsafe_directory",
            Self::Bind(_) => "bind_failed",
        }
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("this platform has no revoke socket"),
            Self::PathTooLong => f.write_str("the data directory's path is too long for a socket"),
            Self::PathOccupied => f.write_str("something other than a socket is in its place"),
            Self::UnsafeDirectory => f.write_str("its directory is not private to this user"),
            Self::Bind(kind) => write!(f, "its socket could not be created ({kind})"),
        }
    }
}

/// Why a revoke through the serving relay did not report an outcome.
/// Every variant but [`NoAnswer`](Self::NoAnswer) means nothing was
/// revoked.
#[derive(Debug, thiserror::Error)]
pub enum LiveRevokeError {
    #[error(
        "a relay is serving, and revoking through it is not supported on this platform; \
         nothing was revoked — stop the relay, then revoke again"
    )]
    Unsupported,
    #[error(
        "a relay is serving, but it has no revoke socket at {path}; nothing was revoked \
         — stop the relay, then revoke again"
    )]
    NoSocket { path: String },
    #[error(
        "a relay is serving, but its revoke socket at {path} did not answer; nothing was \
         revoked — stop the relay, then revoke again"
    )]
    Unreachable {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the revoke socket at {path} is served by another user, or its owner could not be \
         checked; nothing was sent to it"
    )]
    NotOwner { path: String },
    #[error(
        "the serving relay did not confirm the revocation, so the device may or may not be \
         revoked — `claudepot relay devices` shows its state, and revoking again is safe"
    )]
    NoAnswer,
    #[error(
        "the serving relay could not save the revocation, so nothing was revoked — stop \
         the relay, then revoke again to see why"
    )]
    StoreFailed,
    #[error("the relay is stopping; nothing was revoked — revoke again once it has stopped")]
    Stopping,
    #[error("the serving relay refused the request ({code}); stop it, then revoke again")]
    Refused { code: &'static str },
}

/// Listen for revoke requests until `shutdown` fires. Called by
/// [`runtime::start`](super::runtime::start) once nothing else can fail,
/// so a refused start leaves no task holding the relay's state. The relay
/// serves either way; [`LiveRevoke::Unavailable`] says why another
/// process cannot revoke through it. Crate-private: it removes a stale
/// socket on the strength of the serve lock `state` is assumed to hold.
pub(crate) fn arm(
    data_dir: &Path,
    state: Arc<RelayState>,
    shutdown: CancelToken,
) -> (LiveRevoke, Option<JoinHandle<()>>) {
    #[cfg(unix)]
    {
        let me = euid();
        serve::arm_with(
            data_dir,
            state,
            shutdown,
            serve::Owner { dir: me, peer: me },
        )
    }
    #[cfg(not(unix))]
    {
        let _ = (data_dir, state, shutdown);
        (LiveRevoke::Unavailable(Unavailable::Unsupported), None)
    }
}

/// Revoke `device_id` through the relay serving from `data_dir`, provided
/// it runs as this user. `Ok(None)`: it has no such device.
pub async fn revoke(
    data_dir: &Path,
    device_id: &str,
) -> Result<Option<Revocation>, LiveRevokeError> {
    #[cfg(unix)]
    {
        request::request(data_dir, device_id, euid()).await
    }
    #[cfg(not(unix))]
    {
        let _ = (data_dir, device_id);
        Err(LiveRevokeError::Unsupported)
    }
}

#[cfg(unix)]
fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

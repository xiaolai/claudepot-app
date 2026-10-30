//! The relay's end of the revoke socket: bound in a directory private to
//! the relay's user, answering one connection at a time, removed on the
//! way down.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::task::JoinHandle;

use super::wire::{self, Code, Line};
use super::{socket_path, LiveRevoke, Unavailable, DIR_NAME};
use crate::relay::cancel::CancelToken;
use crate::relay::devices::Revocation;
use crate::relay::server::RelayState;

/// How long a connection has to send its whole line.
const READ_DEADLINE: Duration = Duration::from_secs(2);
/// How long a reply has to go out.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// The pause after a failed accept, so a lasting error cannot spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// Far longer than any request; a longer line is refused.
const MAX_REQUEST: u64 = 1024;

/// The uids the socket trusts. Outside tests both are the relay's own;
/// tests set them apart to stand in for another user.
#[derive(Debug, Clone, Copy)]
pub(super) struct Owner {
    /// Who must own `relay-revoke/`.
    pub(super) dir: u32,
    /// Who a connecting client must run as.
    pub(super) peer: u32,
}

pub(super) fn arm_with(
    data_dir: &Path,
    state: Arc<RelayState>,
    shutdown: CancelToken,
    owner: Owner,
) -> (LiveRevoke, Option<JoinHandle<()>>) {
    match bind(data_dir, owner.dir) {
        Ok(socket) => {
            let task = tokio::spawn(accept_loop(socket, state, shutdown, owner.peer));
            (LiveRevoke::Listening, Some(task))
        }
        Err(why) => {
            tracing::warn!(
                path = %socket_path(data_dir).display(),
                reason = %why,
                "relay revoke socket not listening; revoking a device means stopping the relay"
            );
            (LiveRevoke::Unavailable(why), None)
        }
    }
}

/// The listener and the file it is bound to. Fields drop in declaration
/// order, so the file is checked while the listener still holds its
/// inode, and no replacement can carry the same number.
struct Socket {
    file: SocketFile,
    listener: UnixListener,
}

/// The socket file this relay created. Dropping it removes the file,
/// unless something has been put in its place since.
struct SocketFile {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl Drop for SocketFile {
    fn drop(&mut self) {
        let ours = std::fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.file_type().is_socket() && m.dev() == self.dev && m.ino() == self.ino);
        if !ours {
            return;
        }
        if let Err(e) = std::fs::remove_file(&self.path) {
            if e.kind() != io::ErrorKind::NotFound {
                tracing::warn!(path = %self.path.display(), error = %e, "relay revoke socket not removed");
            }
        }
    }
}

fn bind(data_dir: &Path, owner: u32) -> Result<Socket, Unavailable> {
    let dir = data_dir.join(DIR_NAME);
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => {
            return Err(Unavailable::Bind(e.kind()));
        }
        _ => {}
    }
    // Checked, never repaired: a chmod would hide that someone else could
    // already have written here.
    let private = std::fs::symlink_metadata(&dir)
        .is_ok_and(|m| m.is_dir() && m.uid() == owner && m.mode() & 0o077 == 0);
    if !private {
        return Err(Unavailable::UnsafeDirectory);
    }
    let path = socket_path(data_dir);
    match std::fs::symlink_metadata(&path) {
        // Left by a relay that did not stop cleanly: the serve lock this
        // relay holds says no other is serving from here.
        Ok(m) if m.file_type().is_socket() => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(Unavailable::Bind(e.kind()));
            }
            _ => {}
        },
        Ok(_) => return Err(Unavailable::PathOccupied),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(Unavailable::Bind(e.kind())),
    }
    let listener = UnixListener::bind(&path).map_err(|e| match e.kind() {
        io::ErrorKind::InvalidInput => Unavailable::PathTooLong,
        kind => Unavailable::Bind(kind),
    })?;
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) => {
            // Bound but unidentifiable: remove it now, since without its
            // inode the shutdown path could never tell it was ours.
            drop(listener);
            let _ = std::fs::remove_file(&path);
            return Err(Unavailable::Bind(e.kind()));
        }
    };
    Ok(Socket {
        file: SocketFile {
            path,
            dev: meta.dev(),
            ino: meta.ino(),
        },
        listener,
    })
}

async fn accept_loop(socket: Socket, state: Arc<RelayState>, shutdown: CancelToken, peer: u32) {
    loop {
        let accepted = tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            accepted = socket.listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, _)) => answer(stream, &state, &shutdown, peer).await,
            Err(e) => {
                tracing::warn!(error = %e, "relay revoke socket: accept failed");
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => break,
                    () = tokio::time::sleep(ACCEPT_BACKOFF) => {}
                }
            }
        }
    }
    let Socket { file, listener } = socket;
    // The file goes first, so nothing new can connect; then whoever is
    // already queued is told the relay is stopping.
    drop(file);
    drain_backlog(listener, peer).await;
}

/// What a connection's request came to.
enum Request {
    Line(Vec<u8>),
    Refuse(Code),
    /// Closed, silent or unreadable: there is no one to answer.
    Gone,
}

async fn answer(mut stream: UnixStream, state: &RelayState, shutdown: &CancelToken, peer: u32) {
    if !same_user(&stream, peer) {
        return;
    }
    let reading = tokio::time::timeout(READ_DEADLINE, wire::read_line(&mut stream, MAX_REQUEST));
    let request = tokio::select! {
        biased;
        () = shutdown.cancelled() => Request::Refuse(Code::Stopping),
        read = reading => match read {
            Ok(Ok(Line::Whole(line))) => Request::Line(line),
            Ok(Ok(Line::Partial)) => Request::Refuse(Code::BadRequest),
            Ok(Ok(Line::Empty) | Err(_)) | Err(_) => Request::Gone,
        },
    };
    let outcome = match request {
        Request::Line(line) => wire::parse_request(&line).and_then(|id| revoke(state, &id)),
        Request::Refuse(code) => Err(code),
        Request::Gone => return,
    };
    reply(&mut stream, outcome).await;
}

/// Whether the connecting process runs as `peer`. Anything else is
/// closed unread.
fn same_user(stream: &UnixStream, peer: u32) -> bool {
    match stream.peer_cred() {
        Ok(cred) if cred.uid() == peer => true,
        Ok(cred) => {
            tracing::warn!(
                uid = cred.uid(),
                "relay revoke socket: refused a connection from another user"
            );
            false
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "relay revoke socket: refused a connection whose user could not be checked"
            );
            false
        }
    }
}

fn revoke(state: &RelayState, device_id: &str) -> Result<Revocation, Code> {
    match state.devices().revoke(device_id) {
        Ok(Some(revocation)) => {
            tracing::info!(
                device_id,
                ?revocation,
                "relay device revoked from another process"
            );
            Ok(revocation)
        }
        Ok(None) => Err(Code::NoSuchDevice),
        Err(e) => {
            tracing::warn!(error = %e, "relay revoke socket: revocation not saved, so not made");
            Err(Code::StoreFailed)
        }
    }
}

/// A reply that cannot be delivered changes nothing here: the client
/// reports that it heard nothing, and a revocation already made stands.
async fn reply(stream: &mut UnixStream, outcome: Result<Revocation, Code>) {
    let line = wire::reply_line(outcome);
    match tokio::time::timeout(WRITE_TIMEOUT, stream.write_all(line.as_bytes())).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::debug!(error = %e, "relay revoke socket: reply not delivered"),
        Err(_) => tracing::debug!("relay revoke socket: reply timed out"),
    }
}

/// Answer `stopping` to every connection still queued. The std listener
/// accepts straight from the kernel's queue, where tokio's would wait for
/// a readiness event that may not have arrived.
async fn drain_backlog(listener: UnixListener, peer: u32) {
    // Already non-blocking: `into_std` leaves it so.
    let listener = match listener.into_std() {
        Ok(listener) => listener,
        Err(e) => {
            tracing::debug!(error = %e, "relay revoke socket: queue not drained");
            return;
        }
    };
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
            Err(e) => {
                tracing::debug!(error = %e, "relay revoke socket: queue not drained");
                return;
            }
        };
        match adopt(stream) {
            Ok(mut stream) if same_user(&stream, peer) => {
                reply(&mut stream, Err(Code::Stopping)).await;
            }
            Ok(_) => {}
            Err(e) => tracing::debug!(error = %e, "relay revoke socket: queued connection closed"),
        }
    }
}

/// `from_std` needs a non-blocking stream, and an accepted one is not on
/// Linux.
fn adopt(stream: StdUnixStream) -> io::Result<UnixStream> {
    stream.set_nonblocking(true)?;
    UnixStream::from_std(stream)
}

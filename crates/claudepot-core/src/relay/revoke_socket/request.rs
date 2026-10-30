//! The client's end of the revoke socket.

use std::io;
use std::path::Path;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::time::{timeout, timeout_at, Instant};

use super::wire::{self, Code, Line, Reply};
use super::{socket_path, LiveRevokeError};
use crate::relay::devices::Revocation;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// For the request and its reply together. The relay answers one
/// connection at a time and gives a silent one two seconds, so this
/// leaves room to wait behind one and for the revocation's own write.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
/// Far longer than any reply.
const MAX_REPLY: u64 = 1024;

/// Revoke through the relay listening under `data_dir`, provided it runs
/// as `owner`; nothing is sent to a socket anyone else listens on.
pub(super) async fn request(
    data_dir: &Path,
    device_id: &str,
    owner: u32,
) -> Result<Option<Revocation>, LiveRevokeError> {
    let path = socket_path(data_dir);
    let shown = path.display().to_string();
    let mut stream = match timeout(CONNECT_TIMEOUT, UnixStream::connect(&path)).await {
        Ok(Ok(stream)) => stream,
        // `InvalidInput`: a path too long to be a socket address, so no
        // relay can be listening there either.
        Ok(Err(e))
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
            ) =>
        {
            return Err(LiveRevokeError::NoSocket { path: shown });
        }
        Ok(Err(source)) => {
            return Err(LiveRevokeError::Unreachable {
                path: shown,
                source,
            })
        }
        Err(_) => {
            return Err(LiveRevokeError::Unreachable {
                path: shown,
                source: io::ErrorKind::TimedOut.into(),
            });
        }
    };
    if !matches!(stream.peer_cred(), Ok(cred) if cred.uid() == owner) {
        return Err(LiveRevokeError::NotOwner { path: shown });
    }

    let deadline = Instant::now() + EXCHANGE_TIMEOUT;
    let line = wire::request_line(device_id);
    // A failed write never sent the newline, which ends the line and is
    // the last byte written, so the relay cannot have acted on it.
    let write_error = match timeout_at(deadline, stream.write_all(line.as_bytes())).await {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(_) => Some(io::ErrorKind::TimedOut.into()),
    };
    // Read even after a failed write: a relay that is stopping may have
    // answered and closed before the request went out.
    let reply = match timeout_at(deadline, wire::read_line(&mut stream, MAX_REPLY)).await {
        Ok(Ok(Line::Whole(line))) => wire::parse_reply(&line),
        _ => None,
    };
    match reply {
        Some(Reply::Done(revocation)) => Ok(Some(revocation)),
        Some(Reply::Failed(Code::NoSuchDevice)) => Ok(None),
        Some(Reply::Failed(Code::StoreFailed)) => Err(LiveRevokeError::StoreFailed),
        Some(Reply::Failed(Code::Stopping)) => Err(LiveRevokeError::Stopping),
        Some(Reply::Failed(code @ (Code::BadRequest | Code::UnsupportedVersion))) => {
            Err(LiveRevokeError::Refused {
                code: code.as_str(),
            })
        }
        Some(Reply::FailedUnknown) => Err(LiveRevokeError::Refused { code: "unknown" }),
        None => Err(match write_error {
            None => LiveRevokeError::NoAnswer,
            Some(source) => LiveRevokeError::Unreachable {
                path: shown,
                source,
            },
        }),
    }
}

//! The revoke socket's wire format and framing: one JSON line each way.

use std::io;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};

use crate::relay::devices::Revocation;

const VERSION: u64 = 1;

/// Why the relay did not revoke. None of these changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Code {
    NoSuchDevice,
    StoreFailed,
    BadRequest,
    UnsupportedVersion,
    Stopping,
}

impl Code {
    const ALL: [Self; 5] = [
        Self::NoSuchDevice,
        Self::StoreFailed,
        Self::BadRequest,
        Self::UnsupportedVersion,
        Self::Stopping,
    ];

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::NoSuchDevice => "no_such_device",
            Self::StoreFailed => "store_failed",
            Self::BadRequest => "bad_request",
            Self::UnsupportedVersion => "unsupported_version",
            Self::Stopping => "stopping",
        }
    }

    fn from_wire(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == code)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(rename = "v")]
    _version: u64,
    op: Op,
    device_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Op {
    Revoke,
}

/// The device a request line asks to revoke. The version is read first,
/// so a newer client is told `unsupported_version`, not `bad_request`,
/// whatever else its line carries.
pub(super) fn parse_request(line: &[u8]) -> Result<String, Code> {
    let value: Value = serde_json::from_slice(line).map_err(|_| Code::BadRequest)?;
    match value.get("v").and_then(Value::as_u64) {
        Some(VERSION) => {}
        Some(_) => return Err(Code::UnsupportedVersion),
        None => return Err(Code::BadRequest),
    }
    let request: Request = serde_json::from_value(value).map_err(|_| Code::BadRequest)?;
    match request.op {
        Op::Revoke => Ok(request.device_id),
    }
}

pub(super) fn request_line(device_id: &str) -> String {
    terminated(json!({ "v": VERSION, "op": "revoke", "device_id": device_id }))
}

pub(super) fn reply_line(outcome: Result<Revocation, Code>) -> String {
    terminated(match outcome {
        Ok(Revocation::Revoked) => json!({ "v": VERSION, "result": "revoked" }),
        Ok(Revocation::AlreadyRevoked) => json!({ "v": VERSION, "result": "already_revoked" }),
        Err(code) => json!({ "v": VERSION, "result": "error", "code": code.as_str() }),
    })
}

fn terminated(value: Value) -> String {
    let mut line = value.to_string();
    line.push('\n');
    line
}

/// A reply this client understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reply {
    Done(Revocation),
    Failed(Code),
    /// An error with a code this client does not know: still an error,
    /// so still nothing revoked.
    FailedUnknown,
}

#[derive(Deserialize)]
struct RawReply {
    v: u64,
    result: String,
    #[serde(default)]
    code: Option<String>,
}

/// `None` for anything that is not a version-1 reply with a known result.
pub(super) fn parse_reply(line: &[u8]) -> Option<Reply> {
    let raw: RawReply = serde_json::from_slice(line).ok()?;
    if raw.v != VERSION {
        return None;
    }
    match raw.result.as_str() {
        "revoked" => Some(Reply::Done(Revocation::Revoked)),
        "already_revoked" => Some(Reply::Done(Revocation::AlreadyRevoked)),
        "error" => Some(
            raw.code
                .as_deref()
                .and_then(Code::from_wire)
                .map_or(Reply::FailedUnknown, Reply::Failed),
        ),
        _ => None,
    }
}

/// What one bounded line read found.
#[derive(Debug)]
pub(super) enum Line {
    /// The peer closed without sending a byte.
    Empty,
    /// Bytes, but no newline before the cap or the end.
    Partial,
    /// A line, newline included.
    Whole(Vec<u8>),
}

/// Read up to and including the first newline, never more than `cap`
/// bytes. Stopping at the newline matters: a peer that closes with our
/// bytes unread makes Linux report `ECONNRESET`, but only once the bytes
/// queued before it have been read.
pub(super) async fn read_line<R: AsyncRead + Unpin>(reader: R, cap: u64) -> io::Result<Line> {
    let mut buf = Vec::new();
    BufReader::new(reader.take(cap))
        .read_until(b'\n', &mut buf)
        .await?;
    Ok(match buf.last() {
        None => Line::Empty,
        Some(b'\n') => Line::Whole(buf),
        Some(_) => Line::Partial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_line_round_trips_with_one_trailing_newline() {
        let line = request_line("dev-1");
        assert!(line.ends_with('\n') && !line.trim_end_matches('\n').contains('\n'));
        assert_eq!(parse_request(line.as_bytes()), Ok("dev-1".to_owned()));
    }

    #[test]
    fn a_device_id_with_a_newline_stays_on_one_line() {
        let line = request_line("a\nb");
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(parse_request(line.as_bytes()), Ok("a\nb".to_owned()));
    }

    #[test]
    fn key_order_is_not_significant() {
        let line = br#"{"device_id":"d","op":"revoke","v":1}"#;
        assert_eq!(parse_request(line), Ok("d".to_owned()));
    }

    #[test]
    fn malformed_requests_are_refused_by_code() {
        let cases: [(&[u8], Code); 9] = [
            (b"not json", Code::BadRequest),
            (br#"{"op":"revoke","device_id":"d"}"#, Code::BadRequest),
            (
                br#"{"v":"1","op":"revoke","device_id":"d"}"#,
                Code::BadRequest,
            ),
            (
                br#"{"v":-1,"op":"revoke","device_id":"d"}"#,
                Code::BadRequest,
            ),
            (
                br#"{"v":2,"op":"revoke","device_id":"d"}"#,
                Code::UnsupportedVersion,
            ),
            (br#"{"v":2,"op":"explode"}"#, Code::UnsupportedVersion),
            (
                br#"{"v":1,"op":"revoke","device_id":"d","x":1}"#,
                Code::BadRequest,
            ),
            (
                br#"{"v":1,"op":"restart","device_id":"d"}"#,
                Code::BadRequest,
            ),
            (br#"{"v":1,"op":"revoke"}"#, Code::BadRequest),
        ];
        for (line, code) in cases {
            assert_eq!(
                parse_request(line),
                Err(code),
                "{}",
                String::from_utf8_lossy(line)
            );
        }
    }

    #[test]
    fn every_reply_parses_back_to_what_was_sent() {
        for outcome in [Ok(Revocation::Revoked), Ok(Revocation::AlreadyRevoked)] {
            let line = reply_line(outcome);
            assert_eq!(
                parse_reply(line.as_bytes()),
                Some(Reply::Done(outcome.unwrap()))
            );
        }
        for code in Code::ALL {
            let line = reply_line(Err(code));
            assert_eq!(parse_reply(line.as_bytes()), Some(Reply::Failed(code)));
        }
    }

    #[test]
    fn a_reply_this_client_cannot_read_is_none_and_an_unknown_error_is_still_an_error() {
        assert_eq!(parse_reply(br#"{"v":2,"result":"revoked"}"#), None);
        assert_eq!(parse_reply(br#"{"v":1,"result":"maybe"}"#), None);
        assert_eq!(parse_reply(b"garbage"), None);
        assert_eq!(
            parse_reply(br#"{"v":1,"result":"error","code":"new_code"}"#),
            Some(Reply::FailedUnknown)
        );
        assert_eq!(
            parse_reply(br#"{"v":1,"result":"error"}"#),
            Some(Reply::FailedUnknown)
        );
    }

    #[tokio::test]
    async fn read_line_stops_at_the_newline_and_at_the_cap() {
        let whole = read_line(&b"{}\nmore"[..], 64).await.unwrap();
        assert!(matches!(whole, Line::Whole(ref l) if l == b"{}\n"));
        assert!(matches!(
            read_line(&b"abcdef"[..], 4).await.unwrap(),
            Line::Partial
        ));
        assert!(matches!(
            read_line(&b"abc"[..], 64).await.unwrap(),
            Line::Partial
        ));
        assert!(matches!(
            read_line(&b""[..], 64).await.unwrap(),
            Line::Empty
        ));
    }
}

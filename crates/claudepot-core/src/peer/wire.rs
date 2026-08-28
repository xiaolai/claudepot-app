//! Newline-delimited JSON frames for Claude Code's cross-session peer
//! inbox.
//!
//! Verified against the **2.1.239** binary; see the `peer messaging`
//! row in `crates/xtask/cc-upstream-watch.md`. Everything here mirrors
//! CC's own reader, so each constant below is a claim about a specific
//! build rather than a value we chose.
//!
//! ## What the receiver accepts
//!
//! `type: "auth"` (mandatory first line), `type: "user"` (inject a
//! prompt), and `type: "control"` with actions `rename`,
//! `notify_when_idle`, `peer_idle_notice`, `peer_message_status`.
//!
//! There is deliberately **no** exit, interrupt, or restart action. A
//! peer can steer a running session; it cannot end one. Any surface
//! offering "restart this session" must do it some other way, and the
//! reason belongs in the UI copy rather than in a silent no-op.
//!
//! ## Injected prompts are not slash commands
//!
//! CC dispatches an injected prompt with `skipSlashCommands: true`, so
//! `/compact` arrives as literal text in the transcript. A surface that
//! lets the user type a prompt must not imply commands work.

use serde::Serialize;

/// The only `peerProtocol` this client speaks. CC publishes its value
/// per session in `~/.claude/sessions/<pid>.json`; a session announcing
/// anything else is refused rather than addressed on a guess, because
/// the frames below are the entire contract and a bumped version means
/// at least one of them moved.
pub const PEER_PROTOCOL: u32 = 1;

/// CC destroys the connection when one accumulated line exceeds this
/// (`Line exceeded ${N} chars; dropping connection`).
///
/// We enforce it on **UTF-8 bytes** while CC measures UTF-16 code
/// units. For ASCII the two agree; for anything wider, bytes ≥ code
/// units, so this rejects a little early and never lets through a line
/// CC would drop. Erring toward the loud side is the point — the
/// alternative is a connection that dies with no frame ever handled.
pub const MAX_LINE_BYTES: usize = 1_048_576;

/// Where CC places an injected prompt in the session's queue.
///
/// `Next` is CC's own default for peer messages and the one to use
/// unless the caller has a reason: `Now` preempts the running turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Preempt the current turn.
    Now,
    /// Queue immediately after the current turn. CC's default.
    #[default]
    Next,
    /// Queue behind anything already waiting.
    Later,
}

/// First line on every connection. CC closes the socket on any frame
/// that arrives before a valid one.
/// `Debug` is hand-written for the same reason as `KeyFile`'s: this
/// carries the peer token, and a derive is what turns a future
/// `?frame` into a credential in the log.
#[derive(Serialize)]
pub(crate) struct AuthFrame<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub token: &'a str,
}

impl std::fmt::Debug for AuthFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthFrame")
            .field("kind", &self.kind)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl<'a> AuthFrame<'a> {
    pub fn new(token: &'a str) -> Self {
        Self {
            kind: "auth",
            token,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct UserBody<'a> {
    pub role: &'static str,
    pub content: &'a str,
}

/// A prompt injected into a running session.
///
/// `session_id` is **always** populated, though CC treats it as
/// optional. The pid is the only handle a caller naturally has, and
/// pids are recycled: without this field a message addressed to a
/// session that exited lands in whatever session the OS handed the
/// number to next. With it, CC drops the message on mismatch. A silent
/// misdelivery is the worst outcome this module could produce, so the
/// field that prevents it is not optional here.
#[derive(Debug, Serialize)]
pub(crate) struct UserFrame<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub message: UserBody<'a>,
    pub session_id: &'a str,
    pub uuid: String,
    pub priority: Priority,
}

impl<'a> UserFrame<'a> {
    pub fn new(session_id: &'a str, content: &'a str, priority: Priority, uuid: String) -> Self {
        Self {
            kind: "user",
            message: UserBody {
                role: "user",
                content,
            },
            session_id,
            uuid,
            priority,
        }
    }
}

/// Serialize one frame as the line CC expects: compact JSON plus `\n`.
///
/// We deliberately do not set `from`. That field is a reply address —
/// the sender's own socket path — and Claudepot binds no inbox, so
/// there is nothing to address. The cost is that CC sends us no
/// `peer_message_status` receipt, which is why `send_prompt` reports
/// "handed to the socket", never "delivered".
///
/// # Why this stays true even though the receipt got more informative
///
/// CC 2.1.248 made two outcomes explicit that used to vanish: sending
/// to a session with `crossSessionInbound: "refuse"` now reports
/// `refused` instead of a silent success, and an inbox that drops a
/// message (rate limit, full queue) says so. The full status set on the
/// wire is `held | denied | expired | delivered | refused | dropped`,
/// correlated by `orig_msg_id` — and `refused` travels as
/// `{status: "expired", status_detail: "refused"}`, normalised by the
/// receiver.
///
/// Adopting any of it needs more than setting `from`. The receipt is
/// **not** written back on the sending connection; CC opens a *new*
/// connection to the reply address, and only after checking that the
/// address is `uds:`-shaped and resolves **inside CC's own peer socket
/// namespace** under an allowed owner uid. CC's own log names the
/// rejection: *"hold-receipt skipped: reply address unshaped or outside
/// our socket namespace"*. So receiving one means Claudepot binds a
/// listening inbox in a directory Claude Code owns and presents itself
/// to every peer as a CC session — a new accept-frames attack surface,
/// adopted to improve a status string. That trade is refused here, not
/// overlooked.
///
/// Note the fall-through is **silent**: no `from` means CC returns
/// early, with no error and no log. The absence of receipts is
/// therefore not evidence of anything, and must not be read as one.
///
/// Verified against Claude Code 2.1.250 (2026-08-28); the
/// `peer messaging inbox` row in `crates/xtask/cc-upstream-watch.md`
/// re-checks it.
pub(crate) fn encode_line<T: Serialize>(frame: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut buf = serde_json::to_vec(frame)?;
    buf.push(b'\n');
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_frame_matches_cc_shape() {
        let line = encode_line(&AuthFrame::new("deadbeef")).unwrap();
        assert_eq!(
            String::from_utf8(line).unwrap(),
            "{\"type\":\"auth\",\"token\":\"deadbeef\"}\n"
        );
    }

    #[test]
    fn user_frame_carries_session_id_and_role() {
        let frame = UserFrame::new("sess-1", "hello", Priority::Next, "u-1".into());
        let line = String::from_utf8(encode_line(&frame).unwrap()).unwrap();
        assert_eq!(
            line,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\
             \"session_id\":\"sess-1\",\"uuid\":\"u-1\",\"priority\":\"next\"}\n"
        );
    }

    #[test]
    fn priority_serializes_as_cc_wire_words() {
        for (p, want) in [
            (Priority::Now, "\"now\""),
            (Priority::Next, "\"next\""),
            (Priority::Later, "\"later\""),
        ] {
            assert_eq!(serde_json::to_string(&p).unwrap(), want);
        }
    }

    #[test]
    fn default_priority_is_next() {
        assert_eq!(Priority::default(), Priority::Next);
    }

    #[test]
    fn every_line_ends_with_exactly_one_newline() {
        let line = encode_line(&AuthFrame::new("t")).unwrap();
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1);
    }
}

//! What actually happened to a prompt after `send_prompt` returned.
//!
//! `Handoff` proves only that CC took the bytes. The session may then
//! deliver the message, hold it for its user, or refuse it outright —
//! and CC tells us none of that, because Claudepot binds no inbox to be
//! replied to. The transcript is the one honest channel, so this module
//! watches it.
//!
//! Only text appended **after** the send is considered. Scanning the
//! whole file would let an older held message from a previous run
//! classify this one, which is the kind of bug that makes a status
//! display worse than no status display.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::PeerError;

/// CC's wording when a peer message is parked for the user. Matching a
/// human-readable string is fragile by nature, which is exactly why the
/// watchlist row for this module exists.
const HELD_MARKER: &str = "Held peer message";

/// What became of an injected prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It became a real user turn. Claude has seen it.
    Delivered,
    /// It is parked awaiting the recipient user's approval. Claude has
    /// **not** seen it, and never will unless a human clicks.
    Held,
    /// Nothing conclusive appeared before the deadline. Not a failure:
    /// a busy session may not flush a turn for a while.
    Undetermined,
}

/// A byte offset taken before sending, so classification only ever
/// reads what this send produced.
#[derive(Debug, Clone)]
pub struct Watch {
    pub transcript: Option<PathBuf>,
    pub offset: u64,
}

/// Locate `<config>/projects/*/<session_id>.jsonl`.
///
/// A shallow scan rather than a reimplementation of CC's path
/// sanitizer: the session id is unique across projects, so this is both
/// simpler and immune to sanitizer drift. Returns `None` for a session
/// that has not yet taken a turn — that is the normal state of a
/// freshly-started session, not an error.
pub fn transcript_path(config_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let wanted = format!("{session_id}.jsonl");
    std::fs::read_dir(config_dir.join("projects"))
        .ok()?
        .flatten()
        .map(|e| e.path().join(&wanted))
        .find(|p| p.is_file())
}

/// Snapshot the transcript's length before a send.
pub fn begin_watch(config_dir: &Path, session_id: &str) -> Watch {
    let transcript = transcript_path(config_dir, session_id);
    let offset = transcript
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map_or(0, |m| m.len());
    Watch { transcript, offset }
}

/// What a send needs to know about itself to recognise its own notice
/// in the transcript.
#[derive(Debug, Clone, Copy)]
pub struct SendIdentity<'a> {
    /// The uuid on the frame. CC echoes it on the delivered user turn.
    pub uuid: &'a str,
    /// Our own process id. CC reads the peer credential off the socket
    /// and prints it as `[verified pid N]`, so this is the one field on
    /// a held notice that is checkable rather than guessed.
    pub pid: u32,
    /// The text that was sent. CC may include a `preview: «...»` of it,
    /// which is the fallback when the pid was not verified.
    pub text: &'a str,
}

/// Does this held notice belong to *our* send?
///
/// CC's held record (2.1.241) is
/// `Held peer message — from {address}[ [verified pid N]][ (peer claims
/// name: X)][; preview: «...»] — not delivered to Claude (N held). ...`
/// — it carries no uuid, so the delivered path's correlation is not
/// available here. What it does carry is the verified peer pid and a
/// preview of the text, and both are ours to check.
fn held_is_ours(line: &str, id: &SendIdentity<'_>) -> bool {
    if line.contains(&format!("[verified pid {}]", id.pid)) {
        return true;
    }
    // `preview: «...»` — the guillemets are literal in CC's template.
    let Some(rest) = line.split("preview: «").nth(1) else {
        return false;
    };
    let Some(preview) = rest.split('»').next() else {
        return false;
    };
    // CC truncates, and the transcript is JSON-escaped, so compare on a
    // normalised prefix rather than for equality. An empty preview must
    // never match — every text starts with "".
    let preview = preview.trim();
    !preview.is_empty() && id.text.starts_with(preview)
}

/// Classify the text a send appended. Pure, so the precedence between
/// "delivered" and "held" is testable without a live session.
///
/// Delivered wins when both appear: a message that was held and then
/// approved is delivered, and that is the state the user cares about.
///
/// **A held notice must be attributable to this send.** It used to be
/// enough for `HELD_MARKER` to appear anywhere in the appended text, so
/// any concurrent peer message parked in the same window — from another
/// device, or the GUI alongside the CLI — reported *this* send as Held,
/// as a fact, on a surface whose whole job is to say what happened.
/// Delivery was correlated by uuid all along; held was not.
///
/// An unattributable held notice yields `None`, not `Held`: the honest
/// answer is that nothing conclusive about *this* send has appeared
/// yet, which is what `Undetermined` already means.
pub fn classify(appended: &str, id: &SendIdentity<'_>) -> Option<Outcome> {
    let mut held = false;
    for line in appended.lines() {
        // Substring matching on `"type":"user"` missed any valid JSONL
        // whose spacing or key order differed. Parse it.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("user")
                && v.get("uuid").and_then(|u| u.as_str()) == Some(id.uuid)
            {
                return Some(Outcome::Delivered);
            }
        }
        if line.contains(HELD_MARKER) && held_is_ours(line, id) {
            held = true;
        }
    }
    held.then_some(Outcome::Held)
}

/// Poll until the prompt is classifiable or `timeout` elapses.
pub async fn await_outcome(
    config_dir: &Path,
    watch: &Watch,
    session_id: &str,
    id: SendIdentity<'_>,
    timeout: Duration,
) -> Result<Outcome, PeerError> {
    let start = Instant::now();
    loop {
        // Re-resolve each tick: a session with no transcript at send
        // time grows one on its first turn, and the path only exists
        // from that moment.
        let path = watch
            .transcript
            .clone()
            .or_else(|| transcript_path(config_dir, session_id));

        if let Some(path) = path {
            // A CC transcript reaches tens of MB, and `read_from`
            // re-reads from byte zero whenever the file shrank under
            // us. `.claude/rules/rust-conventions.md` puts anything
            // that large on `tokio::fs` or a blocking pool; done inline
            // this stalls the runtime every 250 ms for as long as the
            // caller waits.
            let offset = watch.offset;
            let p = path.clone();
            let body = tokio::task::spawn_blocking(move || read_from(&p, offset))
                .await
                .map_err(|e| PeerError::RegistryUnreadable {
                    path: path.display().to_string(),
                    source: std::io::Error::other(e.to_string()),
                })??;
            if let Some(outcome) = classify(&body, &id) {
                return Ok(outcome);
            }
        }
        // `Instant::now() + timeout` panics on overflow, and `timeout`
        // comes from `--wait <secs>` on the CLI, so a large value is
        // caller-supplied input rather than a theoretical one. Comparing
        // elapsed against the duration cannot overflow.
        if start.elapsed() >= timeout {
            return Ok(Outcome::Undetermined);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn read_from(path: &Path, offset: u64) -> Result<String, PeerError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|source| PeerError::RegistryUnreadable {
        path: path.display().to_string(),
        source,
    })?;
    // A transcript that shrank was rewritten under us (`session slim`,
    // retention). Re-read whole rather than seeking past the end.
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = if len < offset { 0 } else { offset };
    f.seek(SeekFrom::Start(start))
        .map_err(|source| PeerError::RegistryUnreadable {
            path: path.display().to_string(),
            source,
        })?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)
        .map_err(|source| PeerError::RegistryUnreadable {
            path: path.display().to_string(),
            source,
        })?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "a99d94f3-05a9-4e2d-a5c4-5d4e6d4b39c3";
    const PID: u32 = 4242;
    const TEXT: &str = "run the tests and report back";

    fn id() -> SendIdentity<'static> {
        SendIdentity {
            uuid: UUID,
            pid: PID,
            text: TEXT,
        }
    }

    fn user_line(uuid: &str) -> String {
        format!(r#"{{"type":"user","uuid":"{uuid}","message":{{"role":"user"}}}}"#)
    }

    /// CC 2.1.241's real shape, verified against the binary:
    /// `Held peer message — from {addr}[ [verified pid N]][ (peer claims
    /// name: X)][; preview: «...»] — not delivered to Claude (N held).`
    fn held_line(pid: Option<u32>, preview: Option<&str>) -> String {
        let v = pid
            .map(|p| format!(" [verified pid {p}]"))
            .unwrap_or_default();
        let p = preview
            .map(|t| format!("; preview: «{t}»"))
            .unwrap_or_default();
        format!(
            r#"{{"type":"system","content":"{HELD_MARKER} — from cc-peer{v}{p} — not delivered to Claude (1 held)."}}"#
        )
    }

    #[test]
    fn a_user_record_with_our_uuid_is_delivered() {
        assert_eq!(classify(&user_line(UUID), &id()), Some(Outcome::Delivered));
    }

    #[test]
    fn a_delivered_record_is_recognised_whatever_the_json_spacing() {
        // The old check was two substring probes for `"type":"user"`
        // and `"type": "user"`, so any other valid encoding — a
        // different key order, a tab, a newline in the object — read as
        // "not delivered".
        for line in [
            format!(r#"{{ "uuid" : "{UUID}" , "type" : "user" }}"#),
            format!("{{\n  \"type\": \"user\",\n  \"uuid\": \"{UUID}\"\n}}").replace('\n', ""),
            format!(r#"{{"message":{{"role":"user"}},"uuid":"{UUID}","type":"user"}}"#),
        ] {
            assert_eq!(classify(&line, &id()), Some(Outcome::Delivered), "{line}");
        }
    }

    #[test]
    fn a_user_record_for_a_different_send_is_not_ours() {
        let other = "11111111-2222-3333-4444-555555555555";
        assert_eq!(classify(&user_line(other), &id()), None);
    }

    #[test]
    fn a_held_notice_with_our_verified_pid_is_held() {
        assert_eq!(
            classify(&held_line(Some(PID), None), &id()),
            Some(Outcome::Held)
        );
    }

    #[test]
    fn a_held_notice_whose_preview_is_our_text_is_held() {
        assert_eq!(
            classify(&held_line(None, Some("run the tests")), &id()),
            Some(Outcome::Held)
        );
    }

    #[test]
    fn a_concurrent_held_notice_is_not_reported_as_ours() {
        // The bug: any `Held peer message` appended after the watch
        // offset classified THIS send as held. A second device sending
        // to the same session in the same window was enough — and the
        // CLI printed it as fact.
        assert_eq!(classify(&held_line(Some(9999), None), &id()), None);
        assert_eq!(
            classify(&held_line(None, Some("something else entirely")), &id()),
            None
        );
    }

    #[test]
    fn an_unattributable_held_notice_is_not_claimed() {
        // Neither a verified pid nor a preview. We cannot tell, so we
        // do not say. `Undetermined` is the honest outcome.
        assert_eq!(classify(&held_line(None, None), &id()), None);
    }

    #[test]
    fn an_empty_preview_matches_nothing() {
        // Every string starts with "", so a naive prefix test would
        // make an empty preview match every send.
        assert_eq!(classify(&held_line(None, Some("")), &id()), None);
    }

    #[test]
    fn delivery_still_wins_over_a_held_notice_for_the_same_send() {
        let both = format!("{}\n{}", held_line(Some(PID), None), user_line(UUID));
        assert_eq!(classify(&both, &id()), Some(Outcome::Delivered));
    }

    #[test]
    fn nothing_recognisable_is_undetermined() {
        assert_eq!(classify(r#"{"type":"queue-operation"}"#, &id()), None);
    }

    #[test]
    fn empty_input_is_undetermined() {
        assert_eq!(classify("", &id()), None);
    }

    #[test]
    fn another_sessions_uuid_is_not_ours() {
        let other = user_line("00000000-0000-0000-0000-000000000000");
        assert_eq!(classify(&other, &id()), None);
    }

    #[test]
    fn a_uuid_on_a_non_user_record_is_not_delivery() {
        // CC emits queue-operation and attachment records around a
        // delivery; only the user turn means Claude saw it.
        let line = format!(r#"{{"type":"attachment","uuid":"{UUID}"}}"#);
        assert_eq!(classify(&line, &id()), None);
    }

    #[test]
    fn begin_watch_on_a_session_with_no_transcript_starts_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("projects")).unwrap();
        let w = begin_watch(dir.path(), "no-such-session");
        assert!(w.transcript.is_none());
        assert_eq!(w.offset, 0);
    }

    #[test]
    fn begin_watch_records_the_existing_length() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("-tmp-x");
        std::fs::create_dir_all(&proj).unwrap();
        let t = proj.join("sess-1.jsonl");
        std::fs::write(&t, "old content\n").unwrap();
        let w = begin_watch(dir.path(), "sess-1");
        assert_eq!(w.transcript.as_deref(), Some(t.as_path()));
        assert_eq!(w.offset, 12);
    }

    #[tokio::test]
    async fn history_before_the_send_cannot_classify_this_one() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("-tmp-x");
        std::fs::create_dir_all(&proj).unwrap();
        let t = proj.join("sess-1.jsonl");
        // A held notice from a PREVIOUS run already in the file.
        std::fs::write(&t, format!("{}\n", held_line(Some(PID), None))).unwrap();

        let watch = begin_watch(dir.path(), "sess-1");
        let got = await_outcome(
            dir.path(),
            &watch,
            "sess-1",
            id(),
            Duration::from_millis(300),
        )
        .await
        .unwrap();
        assert_eq!(
            got,
            Outcome::Undetermined,
            "an older held notice must not classify a later send"
        );
    }

    #[tokio::test]
    async fn text_appended_after_the_watch_is_classified() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("-tmp-x");
        std::fs::create_dir_all(&proj).unwrap();
        let t = proj.join("sess-1.jsonl");
        std::fs::write(&t, "old\n").unwrap();
        let watch = begin_watch(dir.path(), "sess-1");

        let t2 = t.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&t2).unwrap();
            writeln!(f, "{}", user_line(UUID)).unwrap();
        });

        let got = await_outcome(dir.path(), &watch, "sess-1", id(), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(got, Outcome::Delivered);
    }

    #[tokio::test]
    async fn a_transcript_created_after_the_send_is_still_found() {
        // The fresh-session case: no file at send time.
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("-tmp-x");
        std::fs::create_dir_all(&proj).unwrap();
        let watch = begin_watch(dir.path(), "sess-2");
        assert!(watch.transcript.is_none());

        let t = proj.join("sess-2.jsonl");
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            std::fs::write(&t, format!("{}\n", user_line(UUID))).unwrap();
        });

        let got = await_outcome(dir.path(), &watch, "sess-2", id(), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(got, Outcome::Delivered);
    }
}

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

/// Classify the text a send appended. Pure, so the precedence between
/// "delivered" and "held" is testable without a live session.
///
/// Delivered wins when both appear: a message that was held and then
/// approved is delivered, and that is the state the user cares about.
pub fn classify(appended: &str, uuid: &str) -> Option<Outcome> {
    let delivered = appended
        .lines()
        .filter(|l| l.contains(uuid))
        .any(|l| l.contains(r#""type":"user""#) || l.contains(r#""type": "user""#));
    if delivered {
        return Some(Outcome::Delivered);
    }
    if appended.contains(HELD_MARKER) {
        return Some(Outcome::Held);
    }
    None
}

/// Poll until the prompt is classifiable or `timeout` elapses.
pub async fn await_outcome(
    config_dir: &Path,
    watch: &Watch,
    session_id: &str,
    uuid: &str,
    timeout: Duration,
) -> Result<Outcome, PeerError> {
    let deadline = Instant::now() + timeout;
    loop {
        // Re-resolve each tick: a session with no transcript at send
        // time grows one on its first turn, and the path only exists
        // from that moment.
        let path = watch
            .transcript
            .clone()
            .or_else(|| transcript_path(config_dir, session_id));

        if let Some(path) = path {
            let body = read_from(&path, watch.offset)?;
            if let Some(outcome) = classify(&body, uuid) {
                return Ok(outcome);
            }
        }
        if Instant::now() >= deadline {
            return Ok(Outcome::Undetermined);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn read_from(path: &Path, offset: u64) -> Result<String, PeerError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).map_err(|source| PeerError::KeyUnreadable {
        pid: 0,
        path: path.display().to_string(),
        source,
    })?;
    // A transcript that shrank was rewritten under us (`session slim`,
    // retention). Re-read whole rather than seeking past the end.
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = if len < offset { 0 } else { offset };
    f.seek(SeekFrom::Start(start))
        .map_err(|source| PeerError::KeyUnreadable {
            pid: 0,
            path: path.display().to_string(),
            source,
        })?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)
        .map_err(|source| PeerError::KeyUnreadable {
            pid: 0,
            path: path.display().to_string(),
            source,
        })?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "a99d94f3-05a9-4e2d-a5c4-5d4e6d4b39c3";

    fn user_line(uuid: &str) -> String {
        format!(r#"{{"type":"user","uuid":"{uuid}","message":{{"role":"user"}}}}"#)
    }

    fn held_line() -> String {
        format!(r#"{{"type":"system","content":"{HELD_MARKER} — from an unidentified session"}}"#)
    }

    #[test]
    fn a_user_record_with_our_uuid_is_delivered() {
        assert_eq!(classify(&user_line(UUID), UUID), Some(Outcome::Delivered));
    }

    #[test]
    fn a_held_notice_is_held() {
        assert_eq!(classify(&held_line(), UUID), Some(Outcome::Held));
    }

    #[test]
    fn nothing_recognisable_is_undetermined() {
        assert_eq!(classify(r#"{"type":"queue-operation"}"#, UUID), None);
    }

    #[test]
    fn empty_input_is_undetermined() {
        assert_eq!(classify("", UUID), None);
    }

    #[test]
    fn delivered_wins_over_a_held_notice_for_the_same_send() {
        // Approving a held message produces both records. The user
        // cares that it landed, not that it waited.
        let both = format!("{}\n{}", held_line(), user_line(UUID));
        assert_eq!(classify(&both, UUID), Some(Outcome::Delivered));
    }

    #[test]
    fn another_sessions_uuid_is_not_ours() {
        let other = user_line("00000000-0000-0000-0000-000000000000");
        assert_eq!(classify(&other, UUID), None);
    }

    #[test]
    fn a_uuid_on_a_non_user_record_is_not_delivery() {
        // CC emits queue-operation and attachment records around a
        // delivery; only the user turn means Claude saw it.
        let line = format!(r#"{{"type":"attachment","uuid":"{UUID}"}}"#);
        assert_eq!(classify(&line, UUID), None);
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
        std::fs::write(&t, format!("{}\n", held_line())).unwrap();

        let watch = begin_watch(dir.path(), "sess-1");
        let got = await_outcome(
            dir.path(),
            &watch,
            "sess-1",
            UUID,
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

        let got = await_outcome(dir.path(), &watch, "sess-1", UUID, Duration::from_secs(5))
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

        let got = await_outcome(dir.path(), &watch, "sess-2", UUID, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(got, Outcome::Delivered);
    }
}

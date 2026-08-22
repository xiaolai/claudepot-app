//! Live end-to-end check for `claudepot_core::peer`.
//!
//! Every other test in this module stands up a fake inbox, which proves
//! the frames we emit and nothing about the protocol. This one talks to
//! a **real running Claude Code session** and confirms the prompt lands
//! in that session's transcript.
//!
//! `#[ignore]` by default, and it refuses to guess a target: it drives
//! off `CLAUDEPOT_PEER_TEST_PID`, because the failure mode of picking a
//! session automatically is injecting a stray prompt into work someone
//! is actually doing.
//!
//! ```bash
//! tmux new-session -d -s peertest -c ~/ccspace claude
//! CLAUDEPOT_PEER_TEST_PID=<pid> \
//!   cargo test -p claudepot-core --test peer_live_injection -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use claudepot_core::peer::{self, Priority};
use claudepot_core::session_live::types::PidRecord;

fn sessions_dir() -> PathBuf {
    claudepot_core::paths::claude_config_dir().join("sessions")
}

/// Find `<config>/projects/*/<session_id>.jsonl` without reimplementing
/// CC's path sanitizer — the session id is unique across projects, so a
/// shallow scan is both simpler and immune to sanitizer drift.
fn transcript_path(session_id: &str) -> Option<PathBuf> {
    let projects = claudepot_core::paths::claude_config_dir().join("projects");
    let wanted = format!("{session_id}.jsonl");
    for entry in std::fs::read_dir(projects).ok()?.flatten() {
        let candidate = entry.path().join(&wanted);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[tokio::test]
#[ignore = "needs a live Claude Code session; set CLAUDEPOT_PEER_TEST_PID"]
async fn a_prompt_reaches_a_live_session_transcript() {
    let pid: u32 = std::env::var("CLAUDEPOT_PEER_TEST_PID")
        .expect("set CLAUDEPOT_PEER_TEST_PID to the pid of a live session")
        .parse()
        .expect("CLAUDEPOT_PEER_TEST_PID must be a pid");

    let raw = std::fs::read_to_string(sessions_dir().join(format!("{pid}.json")))
        .expect("no registry file for that pid — is the session still running?");
    let record: PidRecord = serde_json::from_str(&raw).expect("registry file did not parse");

    println!(
        "target: pid={} session={} cwd={}",
        record.pid, record.session_id, record.cwd
    );
    println!("socket: {:?}", record.messaging_socket_path);
    println!("protocol: {:?}", record.peer_protocol);

    let target = peer::PeerTarget::from_record(&record).expect("session is not addressable");
    let session_id = target.session_id.clone();

    // A session that has taken no turn yet has no transcript file at
    // all, so the path is resolved *after* sending rather than before.
    // Requiring it up front is what made the first run of this test
    // fail against a freshly-spawned session.
    println!("transcript before send: {:?}", transcript_path(&session_id));

    let marker = format!("claudepot-peer-probe-{}", uuid::Uuid::new_v4());
    let prompt = format!(
        "This is an automated connectivity probe from Claudepot ({marker}). \
         Reply with exactly: PEER-OK"
    );

    let handoff = peer::send_prompt(&target, &sessions_dir(), &prompt, Priority::Next)
        .await
        .expect("send_prompt failed");
    println!("handed off: uuid={}", handoff.uuid);

    // The handoff proves we wrote to the socket. Only the transcript
    // proves CC accepted the frame, matched the session_id, and queued
    // it as a real user turn.
    let started = Instant::now();
    let deadline = started + Duration::from_secs(90);
    let mut landed: Option<(PathBuf, String)> = None;
    loop {
        if let Some(path) = transcript_path(&session_id) {
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            if body.contains(&marker) {
                println!(
                    "marker found after {:?} in {}",
                    started.elapsed(),
                    path.display()
                );
                landed = Some((path, body));
                break;
            }
        }
        if Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let (path, body) = landed.unwrap_or_else(|| {
        panic!(
            "prompt never reached a transcript for session {session_id}: CC \
             refused the connection, rejected the auth line, or dropped the \
             frame before recording it"
        )
    });

    // Arrival is the transport assertion and it has now passed. What CC
    // *did* with the message depends on that session's
    // `crossSessionInbound` setting, which this test does not control —
    // so classify the outcome rather than failing on the environment.
    let record = body
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|r| r.to_string().contains(&marker))
        .expect("marker matched the file but no single record contains it");

    let kind = record.get("type").and_then(|t| t.as_str()).unwrap_or("?");
    let held = record
        .get("content")
        .and_then(|c| c.as_str())
        .is_some_and(|c| c.contains("Held peer message"));

    if held {
        println!(
            "\nOUTCOME: HELD — arrived as type={kind:?} and is waiting for the \
             user to approve it.\n  Claude has not seen it. Set \
             crossSessionInbound=\"accept\" on the target session to deliver \
             without a prompt."
        );
        // The uuid is only dispatched on delivery, so it must NOT be here.
        assert!(
            !body.contains(&handoff.uuid),
            "a held message should not yet carry the dispatch uuid"
        );
    } else {
        println!("\nOUTCOME: DELIVERED — arrived as type={kind:?} and became a turn.");
        assert!(
            body.contains(&handoff.uuid),
            "{} delivered the prompt but not our uuid {} — Handoff.uuid is \
             not a usable correlation handle on the delivered path",
            path.display(),
            handoff.uuid
        );
        println!("uuid {} correlated in transcript", handoff.uuid);
    }
}

//! Answering a permission prompt from the phone.
//!
//! Every other remote surface in this crate is read-only or injects
//! text. This one is different in kind: it lets whoever holds the admin
//! password **grant a permission**, which is the thing
//! `remote::panel::ask` deliberately refuses to do. Read that module's
//! header first — it explains why a peer message cannot answer a
//! prompt, and none of that reasoning is wrong. This does not route
//! around it; it uses a different door.
//!
//! **The mechanism is Claude Code's `PermissionRequest` hook, not the
//! peer socket.** CC fires the hook exactly when it is about to draw a
//! permission prompt, hands it `session_id` / `tool_name` /
//! `tool_input`, and honours the decision the hook prints. So the
//! answer arrives *before* the prompt is drawn, from a process CC
//! started itself — no keystroke injection, no peer message, no
//! laundering. Verified against the 2.1.241 binary; see the
//! `PermissionRequest hook` row in `crates/xtask/cc-upstream-watch.md`.
//!
//! Four properties are load-bearing:
//!
//! - **Silence is the fall-through.** CC's decision union is `allow` or
//!   `deny` and has no "ask" arm, so a hook that prints nothing leaves
//!   the normal prompt to be drawn at the machine. That makes every
//!   failure — remote switched off, nobody holding the phone, a corrupt
//!   file, a panic — degrade to *exactly today's behaviour* rather than
//!   to a denied tool call. It is the reason this feature can be built
//!   at all: the failure mode is "walk to the machine".
//!
//! - **The wait must end before CC's does.** CC clamps a hook timeout to
//!   `UQ_ = 300_000` ms and kills the process at it; a killed
//!   `PreToolUse` hook blocks the tool call outright. So [`WAIT`] is
//!   held strictly under the [`HOOK_TIMEOUT_SECS`] we install, and the
//!   hook exits by itself. Being killed is the one path that does not
//!   fall through, so we never take it.
//!
//! - **The hook is inert unless the remote surface is on.** Checked at
//!   *runtime*, not only at install time — a hook left in
//!   `settings.json` by a crash, a hand-edit, or an uninstall would
//!   otherwise pause every permission prompt on the machine for two
//!   minutes each. See [`store::gate`].
//!
//! - **One writer per file.** A request and its decision are two files,
//!   not two fields of one. The hook writes only the request; the
//!   server writes only the decision. Atomic rename is crash-safety,
//!   not concurrency-safety — `remote::panel::read_state` documents
//!   losing a write to exactly that confusion — and these two writers
//!   are in *different processes*, where a process-local mutex buys
//!   nothing. Splitting the file removes the race rather than trying to
//!   win it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::session_live::redact::redact_secrets;

pub mod install;

/// How long the hook holds the tool call waiting for a tap.
///
/// Two minutes is the window someone actually holding their phone will
/// use; past that they are not there, and the prompt waiting at the
/// keyboard is the better outcome.
pub const WAIT: Duration = Duration::from_secs(110);

/// What we write into CC's settings, and it is deliberately larger than
/// [`WAIT`]: CC killing the hook is the one outcome that does not fall
/// through cleanly, so the hook must always finish first.
pub const HOOK_TIMEOUT_SECS: u64 = 120;

/// How long a request stays answerable if the hook that made it died.
/// Longer than [`WAIT`] so a live request is never swept from under a
/// hook that is still waiting on it.
const STALE_AFTER: Duration = Duration::from_secs(300);

/// How recently the server must have said it is alive for the hook to
/// bother waiting on it.
///
/// `server.enabled` is a stored *preference*, not liveness — it stays
/// true after the server is killed. Gating on it alone would leave
/// every permission prompt on the machine waiting the full [`WAIT`] for
/// a phone that has nothing to answer through. So the server keeps a
/// heartbeat and the hook believes the heartbeat, not the preference.
const SERVING_FRESH: Duration = Duration::from_secs(20);

/// How often the server refreshes it. Comfortably inside
/// [`SERVING_FRESH`] so an ordinary scheduling hiccup is not read as
/// death.
pub const HEARTBEAT: Duration = Duration::from_secs(5);

/// Cap on the rendered argument. Matches `panel::ask::PendingTool` — a
/// card shows a subject, not a payload.
const ARGUMENT_CHARS: usize = 200;

/// Wall clock in milliseconds. Zero if the clock is before the epoch,
/// which `is_serving` reads as "not serving" — the safe direction.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// What CC hands the hook on stdin.
///
/// Only the fields this feature reads are modelled; CC sends more, and
/// `serde` ignores the rest so a new field upstream is not a parse
/// failure.
#[derive(Debug, Clone, Deserialize)]
pub struct HookInput {
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: serde_json::Value,
}

/// A permission prompt, waiting for a tap.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub id: String,
    pub session_id: String,
    pub cwd: String,
    pub tool_name: String,
    /// The subject — a path, a command — never the whole input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
    pub created_at_ms: u64,
}

/// Allow or deny. There is no third arm, because CC has none: the
/// absence of a decision *is* "ask", and it is expressed by writing no
/// decision at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
}

/// A tap, recorded by the server for the hook to collect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApprovalDecision {
    pub decision: Decision,
    /// Which paired device answered. Self-reported in the same sense
    /// `board::writer_id` is — it is the authenticated device's own id,
    /// so it is honest about *which credential* answered, and a UI
    /// should not present it as a person.
    pub device_id: String,
    pub at_ms: u64,
}

impl ApprovalRequest {
    /// Build a request from what CC handed the hook.
    ///
    /// The argument is redacted and truncated on the way *in*, so a
    /// secret in a command line never reaches the file, let alone the
    /// phone. `redact_secrets` is knowingly incomplete — see
    /// `session_live::redact` — which is why the panel says masked and
    /// never scrubbed.
    pub fn new(input: &HookInput, id: String, now_ms: u64) -> Self {
        let argument = argument_of(&input.tool_input)
            .map(|a| truncate(&redact_secrets(&a), ARGUMENT_CHARS))
            .filter(|a| !a.is_empty());
        Self {
            id,
            session_id: input.session_id.clone(),
            cwd: input.cwd.clone(),
            tool_name: input.tool_name.clone(),
            argument,
            created_at_ms: now_ms,
        }
    }

    fn is_stale(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.created_at_ms) > STALE_AFTER.as_millis() as u64
    }
}

/// The subject of a tool call, by the same rule the transcript tick
/// uses — reused rather than re-derived so the phone and the desktop
/// name the same thing the same way.
fn argument_of(input: &serde_json::Value) -> Option<String> {
    const SUBJECT_KEYS: &[&str] = &[
        "command",
        "file_path",
        "path",
        "notebook_path",
        "pattern",
        "query",
        "url",
        "prompt",
        "description",
    ];
    let map = input.as_object()?;
    for key in SUBJECT_KEYS {
        if let Some(s) = map.get(*key).and_then(|v| v.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                // A heredoc must not turn one card into forty rows.
                return Some(s.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// What the hook prints on stdout for a decision CC will honour.
///
/// Shape verified against the 2.1.241 binary's own schema:
/// `{hookEventName: "PermissionRequest", decision: {behavior: "allow"}
/// | {behavior: "deny", message?}}`.
pub fn decision_output(decision: Decision) -> serde_json::Value {
    let inner = match decision {
        Decision::Allow => serde_json::json!({ "behavior": "allow" }),
        Decision::Deny => serde_json::json!({
            "behavior": "deny",
            "message": "Denied from Claudepot remote.",
        }),
    };
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": inner,
        }
    })
}

pub mod store {
    //! The two-file queue. Everything here is I/O; the decisions are
    //! above.

    use super::*;

    /// Requests live beside the other remote state.
    pub fn dir() -> PathBuf {
        crate::paths::claudepot_data_dir().join("approvals")
    }

    fn request_path(dir: &Path, id: &str) -> PathBuf {
        dir.join(format!("{id}.request.json"))
    }

    fn decision_path(dir: &Path, id: &str) -> PathBuf {
        dir.join(format!("{id}.decision.json"))
    }

    fn serving_path(dir: &Path) -> PathBuf {
        dir.join(".serving")
    }

    /// Server side: "I am here." Refreshed every [`HEARTBEAT`].
    pub fn mark_serving(dir: &Path, now_ms: u64) -> std::io::Result<()> {
        crate::fs_utils::atomic_write(&serving_path(dir), now_ms.to_string().as_bytes())
    }

    /// Server side: "I am going." Best effort — the heartbeat going
    /// stale is what makes this safe to miss, which it will be on
    /// `kill -9`.
    pub fn stop_serving(dir: &Path) {
        let _ = std::fs::remove_file(serving_path(dir));
    }

    /// Is a server actually alive to show this to anyone?
    ///
    /// A clock that jumped backwards reads as not serving, which is the
    /// safe direction: the hook falls through and the prompt waits at
    /// the keyboard.
    pub fn is_serving(dir: &Path, now_ms: u64) -> bool {
        let Ok(raw) = std::fs::read_to_string(serving_path(dir)) else {
            return false;
        };
        let Ok(beat) = raw.trim().parse::<u64>() else {
            return false;
        };
        now_ms >= beat && now_ms - beat <= SERVING_FRESH.as_millis() as u64
    }

    /// Should this hook invocation do anything at all?
    ///
    /// The runtime half of the gate, and the half that holds when the
    /// settings-file half has failed. Checked on **every** invocation,
    /// and every failure to answer reads as "no" — an unreadable
    /// config, a dead server, a clock in the future. The cost of a
    /// false "no" is the prompt waiting at the keyboard exactly as it
    /// does today; the cost of a false "yes" is a two-minute pause on
    /// work nobody is watching.
    pub fn gate() -> bool {
        let enabled = crate::remote::config::load()
            .map(|loaded| loaded.value.server.enabled)
            .unwrap_or(false);
        enabled && is_serving(&dir(), crate::remote::approval::now_ms())
    }

    /// Hook side: publish a request for the phone to see.
    pub fn put_request(dir: &Path, req: &ApprovalRequest) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(req)?;
        crate::fs_utils::atomic_write(&request_path(dir, &req.id), &bytes)
    }

    /// Server side: everything still waiting, oldest first.
    ///
    /// Unreadable and stale entries are skipped rather than raised: this
    /// feeds a list on a phone, and one corrupt file must not blank the
    /// others.
    pub fn pending(dir: &Path, now_ms: u64) -> Vec<ApprovalRequest> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<ApprovalRequest> = entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.ends_with(".request.json"))
            })
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice::<ApprovalRequest>(&b).ok())
            .filter(|r| !r.is_stale(now_ms))
            .collect();
        out.sort_by_key(|r| r.created_at_ms);
        out
    }

    /// Server side: record a tap.
    ///
    /// Refuses an id with no live request, so a decision can never be
    /// planted for a prompt that was never asked.
    ///
    /// **First decision wins, and that is enforced by the filesystem.**
    /// This used to `atomic_write`, which overwrites — so two phones
    /// answering the same prompt, or one person tapping Deny after
    /// Allow, resolved to whichever write landed second, and both calls
    /// returned `true`. For a route whose whole job is granting a tool
    /// capability, "whoever was slower decides" is the wrong rule and an
    /// unobservable one.
    ///
    /// `create_new` is the exclusive-create the race needs. Atomic
    /// rename gives crash-safety, not mutual exclusion — the same
    /// distinction the module docs draw about one writer per file — and
    /// a process-local mutex buys nothing here because the hook and the
    /// server are different processes.
    ///
    /// `Ok(false)` therefore means "not yours to answer": either no live
    /// request, or already answered.
    pub fn put_decision(
        dir: &Path,
        id: &str,
        decision: &ApprovalDecision,
        now_ms: u64,
    ) -> std::io::Result<bool> {
        if !pending(dir, now_ms).iter().any(|r| r.id == id) {
            return Ok(false);
        }
        let bytes = serde_json::to_vec_pretty(decision)?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(decision_path(dir, id))
        {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(&bytes)?;
                f.sync_all()?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Hook side: has it been answered?
    pub fn take_decision(dir: &Path, id: &str) -> Option<ApprovalDecision> {
        let bytes = std::fs::read(decision_path(dir, id)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Hook side: done with it, either way.
    pub fn clear(dir: &Path, id: &str) {
        let _ = std::fs::remove_file(request_path(dir, id));
        let _ = std::fs::remove_file(decision_path(dir, id));
    }

    /// Drop the leavings of hooks that died before clearing up.
    pub fn sweep(dir: &Path, now_ms: u64) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let stale = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<ApprovalRequest>(&b).ok())
                .map(|r| r.is_stale(now_ms));
            // A decision file carries no timestamp we trust here; it is
            // removed with its request by `clear`, and orphaned only if
            // the hook died — in which case its request sweeps too.
            if stale == Some(true) {
                let _ = std::fs::remove_file(&path);
                if let Some(id) = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.strip_suffix(".request.json"))
                {
                    let _ = std::fs::remove_file(decision_path(dir, id));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::store;
    use super::*;

    fn input(tool: &str, json: serde_json::Value) -> HookInput {
        HookInput {
            session_id: "s1".into(),
            cwd: "/tmp/proj".into(),
            tool_name: tool.into(),
            tool_input: json,
        }
    }

    fn req(id: &str, now: u64) -> ApprovalRequest {
        ApprovalRequest::new(
            &input("Bash", serde_json::json!({"command": "ls"})),
            id.into(),
            now,
        )
    }

    #[test]
    fn cc_sends_more_fields_than_we_model_and_that_is_not_an_error() {
        // CC ships ~27 releases a month. A new field upstream must not
        // turn every permission prompt on the machine into a parse
        // failure.
        let raw = serde_json::json!({
            "session_id": "s1",
            "transcript_path": "/x.jsonl",
            "cwd": "/p",
            "tool_name": "Bash",
            "tool_input": {"command": "ls"},
            "permission_suggestions": [],
            "a_field_from_next_month": 1,
        });
        let got: HookInput = serde_json::from_value(raw).unwrap();
        assert_eq!(got.tool_name, "Bash");
    }

    #[test]
    fn the_subject_is_the_argument_not_the_payload() {
        let r = ApprovalRequest::new(
            &input(
                "Edit",
                serde_json::json!({"file_path": "/a/b.rs", "old_string": "x"}),
            ),
            "i".into(),
            0,
        );
        // `Edit` carries both; the path is the subject.
        assert_eq!(r.argument.as_deref(), Some("/a/b.rs"));
    }

    #[test]
    fn a_heredoc_does_not_become_forty_rows() {
        let r = ApprovalRequest::new(
            &input(
                "Bash",
                serde_json::json!({"command": "cat <<E\nline1\nline2\nE"}),
            ),
            "i".into(),
            0,
        );
        assert!(!r.argument.unwrap().contains('\n'));
    }

    #[test]
    fn a_secret_in_a_command_never_reaches_the_file() {
        let r = ApprovalRequest::new(
            &input(
                "Bash",
                serde_json::json!({"command": "deploy --key sk-ant-oat01-LEAKED"}),
            ),
            "i".into(),
            0,
        );
        assert!(!r.argument.unwrap().contains("LEAKED"));
    }

    #[test]
    fn an_unrecognised_tool_still_makes_a_card() {
        // Degrades to no argument rather than to no request: "Claude
        // wants to use SomeNewTool" is still worth a tap.
        let r = ApprovalRequest::new(
            &input("SomeNewTool", serde_json::json!({"x": 1})),
            "i".into(),
            0,
        );
        assert_eq!(r.argument, None);
        assert_eq!(r.tool_name, "SomeNewTool");
    }

    #[test]
    fn the_decision_matches_ccs_schema() {
        let allow = decision_output(Decision::Allow);
        assert_eq!(
            allow["hookSpecificOutput"]["hookEventName"],
            serde_json::json!("PermissionRequest")
        );
        assert_eq!(
            allow["hookSpecificOutput"]["decision"]["behavior"],
            serde_json::json!("allow")
        );
        let deny = decision_output(Decision::Deny);
        assert_eq!(
            deny["hookSpecificOutput"]["decision"]["behavior"],
            serde_json::json!("deny")
        );
    }

    #[test]
    fn the_wait_ends_before_cc_kills_the_hook() {
        // The one property whose failure does NOT fall through: a
        // killed hook blocks the tool call.
        assert!(WAIT.as_secs() < HOOK_TIMEOUT_SECS);
        // And CC clamps a hook timeout to 300s, so ours must be under it.
        assert!(HOOK_TIMEOUT_SECS < 300);
    }

    #[test]
    fn a_request_and_its_decision_round_trip() {
        let d = tempfile::tempdir().unwrap();
        let r = req("abc", 1_000);
        store::put_request(d.path(), &r).unwrap();

        assert_eq!(store::pending(d.path(), 1_000), vec![r.clone()]);
        assert!(store::take_decision(d.path(), "abc").is_none());

        let dec = ApprovalDecision {
            decision: Decision::Allow,
            device_id: "dev1".into(),
            at_ms: 1_100,
        };
        assert!(store::put_decision(d.path(), "abc", &dec, 1_000).unwrap());
        assert_eq!(store::take_decision(d.path(), "abc"), Some(dec));

        store::clear(d.path(), "abc");
        assert!(store::pending(d.path(), 1_000).is_empty());
        assert!(store::take_decision(d.path(), "abc").is_none());
    }

    #[test]
    fn a_decision_cannot_be_planted_for_a_prompt_nobody_asked() {
        let d = tempfile::tempdir().unwrap();
        let dec = ApprovalDecision {
            decision: Decision::Allow,
            device_id: "dev1".into(),
            at_ms: 0,
        };
        assert!(!store::put_decision(d.path(), "ghost", &dec, 0).unwrap());
        assert!(store::take_decision(d.path(), "ghost").is_none());
    }

    #[test]
    fn a_stale_request_is_neither_listed_nor_answerable() {
        let d = tempfile::tempdir().unwrap();
        store::put_request(d.path(), &req("old", 0)).unwrap();
        let later = STALE_AFTER.as_millis() as u64 + 1;

        assert!(store::pending(d.path(), later).is_empty());
        let dec = ApprovalDecision {
            decision: Decision::Allow,
            device_id: "d".into(),
            at_ms: later,
        };
        assert!(!store::put_decision(d.path(), "old", &dec, later).unwrap());
    }

    #[test]
    fn one_corrupt_file_does_not_blank_the_list() {
        let d = tempfile::tempdir().unwrap();
        store::put_request(d.path(), &req("good", 5)).unwrap();
        std::fs::write(d.path().join("bad.request.json"), b"{not json").unwrap();

        let got = store::pending(d.path(), 5);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "good");
    }

    #[test]
    fn pending_is_oldest_first() {
        let d = tempfile::tempdir().unwrap();
        store::put_request(d.path(), &req("second", 200)).unwrap();
        store::put_request(d.path(), &req("first", 100)).unwrap();
        let ids: Vec<_> = store::pending(d.path(), 200)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec!["first", "second"]);
    }

    #[test]
    fn sweep_removes_a_dead_hooks_leavings_and_spares_the_living() {
        let d = tempfile::tempdir().unwrap();
        store::put_request(d.path(), &req("dead", 0)).unwrap();
        let now = STALE_AFTER.as_millis() as u64 + 1;
        store::put_request(d.path(), &req("live", now)).unwrap();

        store::sweep(d.path(), now);
        let ids: Vec<_> = store::pending(d.path(), now)
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, vec!["live"]);
    }

    #[test]
    fn a_missing_directory_is_an_empty_list_not_an_error() {
        // The ordinary state on a machine that has never used this.
        assert!(store::pending(Path::new("/nonexistent/claudepot-approvals"), 0).is_empty());
    }
}

#[cfg(test)]
mod serving_tests {
    use super::store;
    use super::*;

    #[test]
    fn a_live_heartbeat_reads_as_serving() {
        let d = tempfile::tempdir().unwrap();
        assert!(!store::is_serving(d.path(), 10_000), "nothing written yet");

        store::mark_serving(d.path(), 10_000).unwrap();
        assert!(store::is_serving(d.path(), 10_000));
        assert!(store::is_serving(
            d.path(),
            10_000 + HEARTBEAT.as_millis() as u64
        ));
    }

    #[test]
    fn a_killed_server_stops_holding_prompts() {
        // The case the heartbeat exists for: `server.enabled` is still
        // true on disk, but nothing is listening.
        let d = tempfile::tempdir().unwrap();
        store::mark_serving(d.path(), 0).unwrap();
        assert!(!store::is_serving(
            d.path(),
            SERVING_FRESH.as_millis() as u64 + 1
        ));
    }

    #[test]
    fn a_clean_shutdown_falls_through_at_once() {
        let d = tempfile::tempdir().unwrap();
        store::mark_serving(d.path(), 100).unwrap();
        store::stop_serving(d.path());
        assert!(!store::is_serving(d.path(), 100));
    }

    #[test]
    fn a_heartbeat_from_the_future_reads_as_dead() {
        // A clock that jumped must not pin the gate open.
        let d = tempfile::tempdir().unwrap();
        store::mark_serving(d.path(), 10_000).unwrap();
        assert!(!store::is_serving(d.path(), 5_000));
    }

    #[test]
    fn garbage_in_the_heartbeat_reads_as_dead() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".serving"), b"soon").unwrap();
        assert!(!store::is_serving(d.path(), 0));
    }

    #[test]
    fn the_heartbeat_is_not_mistaken_for_a_request() {
        // `.serving` lives in the same directory as the queue.
        let d = tempfile::tempdir().unwrap();
        store::mark_serving(d.path(), 0).unwrap();
        assert!(store::pending(d.path(), 0).is_empty());
        store::sweep(d.path(), u64::MAX);
        assert!(store::is_serving(d.path(), 0), "sweep must not eat it");
    }

    #[test]
    fn the_first_decision_wins_and_the_second_is_told_so() {
        // Two phones on the same prompt, or one person changing their
        // mind mid-tap. Overwriting made the LAST writer decide whether
        // a tool call runs, and told both of them they had decided.
        let d = tempfile::tempdir().unwrap();
        let now = 1_000u64;
        let req = ApprovalRequest {
            id: "req-1".into(),
            session_id: "sess-1".into(),
            cwd: "/tmp".into(),
            tool_name: "Bash".into(),
            argument: Some("rm -rf /tmp/x".into()),
            created_at_ms: now,
        };
        store::put_request(d.path(), &req).unwrap();

        let allow = ApprovalDecision {
            decision: Decision::Allow,
            device_id: "phone-a".into(),
            at_ms: now + 1,
        };
        let deny = ApprovalDecision {
            decision: Decision::Deny,
            device_id: "phone-b".into(),
            at_ms: now + 2,
        };

        assert!(store::put_decision(d.path(), "req-1", &allow, now).unwrap());
        assert!(
            !store::put_decision(d.path(), "req-1", &deny, now).unwrap(),
            "the second decision must be refused, not applied"
        );
        assert_eq!(
            store::take_decision(d.path(), "req-1"),
            Some(allow),
            "the first decision is the one that stands"
        );
    }
}

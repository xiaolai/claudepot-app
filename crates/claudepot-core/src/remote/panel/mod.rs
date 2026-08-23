//! What the remote panel is served.
//!
//! Lives beside [`super::server`] rather than in it so the HTTP layer
//! stays about HTTP: everything here is a pure-ish read over surfaces
//! that already exist — the PID registry, `session_index`, `project`,
//! `account` — joined into one shape a phone can render.
//!
//! ## The join
//!
//! Two sources, joined on CC's session id:
//!
//! - **Live** comes from `~/.claude/sessions/<pid>.json`, which CC
//!   writes for every top-level process. It carries `status` and
//!   `waitingFor` and nothing else worth rendering.
//! - **Everything else** comes from `sessions.db`, which already folds
//!   the transcript into a title, counts, timings, branch and models.
//!
//! ## What is deliberately not here
//!
//! - **`stuck` and `idle_ms`.** Both are overlays computed by
//!   `session_live::LiveRuntime`, which is a long-running poller the GUI
//!   owns. `claudepot remote serve` has no runtime, and standing one up
//!   per HTTP request would produce a value with no history behind it.
//!   `last_ts` is served instead and the client computes elapsed time,
//!   which is honest about being a file mtime rather than a liveness
//!   measurement.
//! - **A tool-call count.** `sessions.db` has no tool-call table and
//!   adding one is a schema migration. The field is absent rather than
//!   estimated: a card showing a number nobody computed is worse than a
//!   card without one.
//! - **A `failed` status.** The only available signal is
//!   `SessionRow::has_error`, true of any transcript containing one
//!   errored tool call — routine in a long session. It is served as its
//!   own boolean, not folded into the status the UI colours red.
//!
//! ## Secrets
//!
//! Every string here that originates in a transcript — titles, tails,
//! pending questions — goes through
//! [`crate::session_live::redact::redact_secrets`] on the way out. That
//! profile is explicitly incomplete (its own docs list what it covers),
//! which is why the client says so rather than implying the text is
//! scrubbed. Redacting anyway is worth it for the families it does
//! cover; claiming it is sufficient would not be.

pub mod ask;
pub mod read_state;
pub mod transcript;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::session::SessionRow;
use crate::session_live::redact::redact_secrets;
use crate::session_live::registry::{self, SysinfoCheck};
use crate::session_live::types::PidRecord;

/// How many finished sessions to send alongside the live ones.
///
/// `sessions.db` has everything; a phone does not want it. Twenty is
/// roughly two screenfuls of history — enough to find yesterday's work,
/// short enough that the response stays under a few tens of kilobytes on
/// a LAN. Paging is deliberately deferred: the screen that would need it
/// does not exist yet.
pub const DEFAULT_HISTORY: usize = 20;

/// A card's title is a prompt, and a prompt can be a paragraph.
const TITLE_CHARS: usize = 140;

#[derive(Debug, thiserror::Error)]
pub enum PanelError {
    #[error("cannot read the session registry: {0}")]
    Registry(String),
    #[error("cannot read the session index: {0}")]
    Index(String),
}

/// What CC says a live session is doing, plus the one state a finished
/// session can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelStatus {
    /// The model is producing output or a tool is running.
    Busy,
    /// Attached and waiting for the user to type.
    Idle,
    /// Blocked on an approval or a question.
    Waiting,
    /// No process. The transcript is all that is left.
    Ended,
}

impl PanelStatus {
    fn from_record(r: &PidRecord) -> Self {
        match r.status.as_deref() {
            Some("busy") => Self::Busy,
            Some("waiting") => Self::Waiting,
            // CC only writes `status` behind a feature gate. Absent
            // means "we cannot tell", and claiming Busy would put a
            // pulsing dot on a session that is sitting still.
            _ => Self::Idle,
        }
    }
}

/// One row of the panel's session list.
#[derive(Debug, Clone, Serialize)]
pub struct PanelSession {
    pub session_id: String,
    /// A process is registered for this session right now.
    pub live: bool,
    /// Present for display and diagnosis only. **Never an address** —
    /// pids are recycled, so every mutation resolves by session id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// `false` when the session is live but has no reachable inbox —
    /// an older Claude Code, or one whose socket bind failed. The client
    /// disables its composer rather than offering a send that cannot
    /// land.
    pub addressable: bool,
    pub status: PanelStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<String>,
    /// CC's own name for the session, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// First real user prompt — markdown marks stripped, collapsed to
    /// one line, truncated. The closest thing a transcript has to a
    /// subject line. See `session::title`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub project_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Every model this session used, as `SessionRow` stores them —
    /// **sorted and deduped**, not chronological — minus the ones that
    /// are not models.
    ///
    /// This was a single `model` taken from `models.last()`, which is
    /// the alphabetically last one — so a session that ran haiku and
    /// then opus reported opus, and one that ran opus and then haiku
    /// *also* reported opus. The card was stating a fact nobody had
    /// established. Sending the set and letting the client say "2
    /// models" when there are two is the honest shape; the chronological
    /// answer lives in `session_turns` and is not worth a per-row query
    /// on every poll.
    pub models: Vec<String>,
    pub messages: usize,
    /// The four components, not one sum.
    ///
    /// `TokenUsage::total()` includes cache reads, which dominate by two
    /// orders of magnitude on a long session — one measured here reported
    /// 1.8 *billion* tokens, a figure that tells a phone nothing. Which
    /// number belongs on a card is a presentation decision, so the parts
    /// travel and the client adds up the ones it means.
    pub tokens: crate::session::TokenUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_ts: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_ts: Option<DateTime<Utc>>,
    /// The transcript contains at least one failed tool call. A marker,
    /// not a status — see the module docs.
    pub has_error: bool,
    /// Last line of prose, for the live card. `None` when the tail is
    /// all tool traffic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_line: Option<String>,
    /// The pending question and its offered choices, when one is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<ask::PendingAsk>,
    /// What the session is blocked on when it is **not** a question —
    /// in practice a permission prompt.
    ///
    /// Carries what is being asked, not a way to answer it: there is a
    /// real difference between "approve Bash" and
    /// `Bash: rm -rf target/debug`, and it decides whether the walk to
    /// the machine is worth making. Mutually exclusive with `ask`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_tool: Option<ask::PendingTool>,
    /// Events this device has not seen since it last looked.
    ///
    /// **Absent when this device has never opened the session**, which is
    /// not the same as zero. A count against no baseline is just the
    /// event total — a fresh phone would show a four-digit badge on
    /// every row, which is noise rather than a signal. The first open
    /// sets the baseline; appends after that are what the badge counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unread: Option<u32>,
}

/// Everything `list` needs, injected so tests never touch a real home.
pub struct ListContext {
    /// `~/.claude`.
    pub config_dir: PathBuf,
    /// `$XDG_RUNTIME_DIR/…/sessions` — where CC writes `<pid>.json`.
    pub sessions_dir: PathBuf,
    /// How many finished sessions to include.
    pub history: usize,
    /// Which device is asking, for the unread counts.
    pub device_id: Option<uuid::Uuid>,
}

impl ListContext {
    /// The production context: CC's real config dir and session dir.
    pub fn live(device_id: Option<uuid::Uuid>) -> Self {
        Self {
            config_dir: crate::paths::claude_config_dir(),
            sessions_dir: registry::default_sessions_dir(),
            history: DEFAULT_HISTORY,
            device_id,
        }
    }
}

/// Live sessions, always; then the most recently touched finished ones.
pub fn list(ctx: &ListContext) -> Result<Vec<PanelSession>, PanelError> {
    let live = live_records(&ctx.sessions_dir)?;
    let rows = crate::session::list_all_sessions(&ctx.config_dir)
        .map_err(|e| PanelError::Index(e.to_string()))?;

    // `sessions.db` is keyed by file path, and one session id can have
    // several transcripts after a resume. The most complete copy wins,
    // measured by event count — the same rule `corpus` uses when it
    // dedupes across machines.
    let mut best: HashMap<&str, &SessionRow> = HashMap::new();
    for row in &rows {
        best.entry(row.session_id.as_str())
            .and_modify(|cur| {
                if row.event_count > cur.event_count {
                    *cur = row;
                }
            })
            .or_insert(row);
    }

    let read = read_state::load_for(ctx.device_id);

    let mut out: Vec<PanelSession> = Vec::with_capacity(live.len() + ctx.history);
    for record in &live {
        let row = best.get(record.session_id.as_str()).copied();
        out.push(compose(Some(record), row, &read));
    }

    // Most recently ACTIVE first, which is not the order they started
    // in. `live_records` sorts by process start — fine as a tiebreak,
    // useless as the answer to "what was I just doing", since the
    // session you opened first this morning is the one you have been
    // working in all day. Sorting the composed rows rather than the pid
    // records because `last_ts` comes from the transcript, which only
    // exists after `compose`.
    out.sort_by(|a, b| b.last_ts.cmp(&a.last_ts));

    let live_ids: std::collections::HashSet<&str> =
        live.iter().map(|r| r.session_id.as_str()).collect();
    // `list_all_sessions` returns newest-first, so `take` is the "most
    // recently touched" rule without a second sort.
    for row in rows
        .iter()
        .filter(|r| !live_ids.contains(r.session_id.as_str()) && !r.is_sidechain)
        .take(ctx.history)
    {
        out.push(compose(None, Some(row), &read));
    }
    Ok(out)
}

/// Live records, newest first. Every registered process, not only the
/// addressable ones: a session we cannot message is still a session the
/// user can watch.
fn live_records(sessions_dir: &Path) -> Result<Vec<PidRecord>, PanelError> {
    let check = SysinfoCheck::new();
    let outcome = registry::poll_dir(sessions_dir, &check)
        .map_err(|e| PanelError::Registry(e.to_string()))?;
    let mut live = outcome.live;
    live.sort_by(|a, b| b.started_at_ms.cmp(&a.started_at_ms));
    Ok(live)
}

fn compose(
    record: Option<&PidRecord>,
    row: Option<&SessionRow>,
    read: &read_state::DeviceReadState,
) -> PanelSession {
    let session_id = record
        .map(|r| r.session_id.clone())
        .or_else(|| row.map(|r| r.session_id.clone()))
        .unwrap_or_default();

    let addressable = record
        .map(|r| crate::peer::PeerTarget::from_record(r).is_ok())
        .unwrap_or(false);

    let project_path = row
        .map(|r| r.project_path.clone())
        .or_else(|| record.map(|r| r.cwd.clone()))
        .unwrap_or_default();

    let tail = row.and_then(|r| transcript::tail_summary(&r.file_path));
    let waiting_row = record
        .filter(|r| matches!(PanelStatus::from_record(r), PanelStatus::Waiting))
        .and(row);
    let ask = waiting_row.and_then(|r| ask::pending(&r.file_path));
    // Only when there is no question: a card shows one or the other, and
    // a permission prompt is what "waiting" means the rest of the time.
    let pending_tool = match ask {
        Some(_) => None,
        None => waiting_row.and_then(|r| ask::pending_tool(&r.file_path)),
    };

    let events = row.map(|r| r.event_count).unwrap_or(0);
    // `event_count` only grows within one transcript, so the difference
    // is the number of rows this device has not looked at. A rebuilt
    // index or a slimmed transcript can move it backwards, hence the
    // saturating subtraction rather than a signed count.
    let unread = read
        .seen(&session_id)
        .map(|seen| u32::try_from(events.saturating_sub(seen)).unwrap_or(u32::MAX));

    PanelSession {
        session_id,
        live: record.is_some(),
        pid: record.map(|r| r.pid),
        addressable,
        status: record
            .map(PanelStatus::from_record)
            .unwrap_or(PanelStatus::Ended),
        waiting_for: record
            .and_then(|r| r.waiting_for.clone())
            .map(|s| redact_secrets(&s)),
        name: record
            .and_then(|r| r.name.clone())
            .map(|s| redact_secrets(&s)),
        // Redact, then derive, then truncate — in that order. Redaction
        // first so nothing can survive it; `derive` strips markdown
        // marks rather than rendering them, because a card title has no
        // room for a heading or a fence and every prompt that starts
        // with `## User Input` was showing them.
        // The LAST prompt, falling back to the first. A card names
        // what the session is doing now; the stored
        // `first_user_prompt` names what it was doing hours ago, which
        // on a long thread is a question already settled. The fallback
        // matters — a session whose tail holds only tool traffic still
        // needs a name.
        title: row
            .and_then(|r| {
                transcript::last_user_prompt(&r.file_path).or_else(|| r.first_user_prompt.clone())
            })
            .and_then(|t| crate::session::title::derive(&redact_secrets(&t)))
            .map(|t| truncate(&t, TITLE_CHARS)),
        project_path,
        branch: row.and_then(|r| r.git_branch.clone()),
        models: row
            .map(|r| {
                r.models
                    .iter()
                    .filter(|m| !is_pseudo_model(m))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
        messages: row.map(|r| r.message_count).unwrap_or(0),
        tokens: row.map(|r| r.tokens.clone()).unwrap_or_default(),
        first_ts: row.and_then(|r| r.first_ts),
        last_ts: row.and_then(|r| r.last_ts),
        has_error: row.map(|r| r.has_error).unwrap_or(false),
        last_line: tail,
        ask,
        pending_tool,
        unread,
    }
}

/// Model ids Claude Code writes that name no model.
///
/// `<synthetic>` is the one that occurs, and it occurs constantly:
/// CC uses it for assistant turns it wrote itself rather than received
/// — "No response requested.", "API Error: 500", "Login expired". Found
/// on real data, where the first live session reported
/// `["<synthetic>", "claude-opus-5"]` and the card said "2 models".
fn is_pseudo_model(model: &str) -> bool {
    model.starts_with('<') && model.ends_with('>')
}

/// Truncate on a character boundary, adding an ellipsis only when
/// something was actually cut.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, events: usize) -> SessionRow {
        SessionRow {
            session_id: id.to_string(),
            slug: "-tmp".into(),
            file_path: PathBuf::from("/nonexistent/x.jsonl"),
            file_size_bytes: 0,
            last_modified: None,
            project_path: "/tmp/proj".into(),
            project_from_transcript: true,
            first_ts: None,
            last_ts: None,
            event_count: events,
            message_count: 3,
            user_message_count: 1,
            assistant_message_count: 2,
            first_user_prompt: Some("do the thing".into()),
            models: vec!["claude-opus-5".into()],
            tokens: Default::default(),
            git_branch: Some("main".into()),
            cc_version: None,
            display_slug: None,
            has_error: false,
            is_sidechain: false,
        }
    }

    #[test]
    fn truncate_only_marks_what_it_cuts() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("  padded  ", 10), "padded");
        assert_eq!(truncate("abcdefghij", 5), "abcde…");
    }

    #[test]
    fn truncate_respects_character_boundaries() {
        // Byte slicing here would panic; the test exists because the
        // first version of this function used `&s[..max]`.
        let s = "把这段代码改好";
        assert_eq!(truncate(s, 3), "把这段…");
    }

    #[test]
    fn a_session_with_no_process_is_ended_and_not_addressable() {
        let r = row("s1", 10);
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        assert_eq!(s.status, PanelStatus::Ended);
        assert!(!s.live);
        assert!(!s.addressable);
        assert_eq!(s.pid, None);
    }

    #[test]
    fn absent_cc_status_reads_as_idle_never_busy() {
        // CC writes `status` only behind a feature gate. Guessing Busy
        // would put a pulsing "live" dot on a session sitting still.
        let mut rec = pid_record("s1");
        rec.status = None;
        assert_eq!(PanelStatus::from_record(&rec), PanelStatus::Idle);
        rec.status = Some("unrecognised-future-value".into());
        assert_eq!(PanelStatus::from_record(&rec), PanelStatus::Idle);
        rec.status = Some("busy".into());
        assert_eq!(PanelStatus::from_record(&rec), PanelStatus::Busy);
        rec.status = Some("waiting".into());
        assert_eq!(PanelStatus::from_record(&rec), PanelStatus::Waiting);
    }

    #[test]
    fn unread_never_goes_negative_when_the_index_shrinks() {
        // `session slim` rewrites a transcript smaller in place and a
        // rebuild re-counts from zero. Either can put `seen` above
        // `event_count`; a signed count would render "-4 unread".
        let r = row("s1", 2);
        let mut read = read_state::DeviceReadState::empty();
        read.set("s1", 99);
        let s = compose(None, Some(&r), &read);
        assert_eq!(s.unread, Some(0));
    }

    #[test]
    fn a_session_this_device_never_opened_carries_no_badge() {
        // Not zero, and not the event total: a fresh phone showing a
        // four-digit badge on every row is noise, and a count against no
        // baseline is exactly that.
        let r = row("s1", 4000);
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        assert_eq!(s.unread, None);
        let json = serde_json::to_value(&s).unwrap();
        assert!(
            json.get("unread").is_none(),
            "an absent badge must not serialize"
        );
    }

    #[test]
    fn a_badge_counts_appends_since_the_last_look() {
        let r = row("s1", 42);
        let mut read = read_state::DeviceReadState::empty();
        read.set("s1", 40);
        assert_eq!(compose(None, Some(&r), &read).unread, Some(2));
    }

    #[test]
    fn token_components_travel_rather_than_one_sum() {
        // A sum including cache reads reported 1.8 billion on a real
        // session, which tells a phone nothing.
        let mut r = row("s1", 1);
        r.tokens = crate::session::TokenUsage {
            input: 10,
            output: 20,
            cache_creation: 30,
            cache_read: 4_000_000,
        };
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        assert_eq!(s.tokens.input, 10);
        assert_eq!(s.tokens.output, 20);
        assert_eq!(s.tokens.cache_read, 4_000_000);
    }

    #[test]
    fn a_synthetic_marker_is_not_a_model() {
        // Measured: the first live session on this machine reported
        // `["<synthetic>", "claude-opus-5"]`, and the card said "2
        // models" for a session that used one.
        let mut r = row("s1", 1);
        r.models = vec!["<synthetic>".into(), "claude-opus-5".into()];
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        assert_eq!(s.models, vec!["claude-opus-5".to_string()]);
    }

    #[test]
    fn a_session_with_only_synthetic_turns_reports_no_model() {
        // Better than reporting "<synthetic>" as one: an API error is
        // not a model choice, and a card that names it is stating a
        // fact that is not true.
        let mut r = row("s1", 1);
        r.models = vec!["<synthetic>".into()];
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        assert!(s.models.is_empty());
    }

    #[test]
    fn every_model_travels_because_the_order_is_not_chronological() {
        // `SessionRow::models` is sorted and deduped. Picking `.last()`
        // reported the alphabetically last model as "the" model, which
        // is the same answer whichever one the session actually ended
        // on.
        let mut r = row("s1", 1);
        r.models = vec!["claude-haiku-4-5".into(), "claude-opus-5".into()];
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        assert_eq!(s.models.len(), 2);
    }

    #[test]
    fn a_title_carries_no_markdown_marks() {
        // The card is one line; a heading, a fence and a bold run have
        // nowhere to go in it. Measured on this machine, where every
        // `/execute-plan` session's title began "## User Input ```text".
        let mut r = row("s1", 1);
        r.first_user_prompt =
            Some("## User Input\n\n```text\nplan.md 3\n```\n\n- **Plan files**".into());
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        let title = s.title.unwrap();
        for mark in ["##", "```", "**"] {
            assert!(!title.contains(mark), "{mark} survived in {title:?}");
        }
        assert!(title.starts_with("User Input"), "{title}");
    }

    #[test]
    fn a_title_that_is_only_scaffolding_is_absent_not_blank() {
        let mut r = row("s1", 1);
        r.first_user_prompt = Some("## \n\n```\n".into());
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        assert_eq!(s.title, None, "a blank row is worse than no title");
    }

    #[test]
    fn a_title_is_redacted_before_it_is_truncated() {
        let mut r = row("s1", 1);
        r.first_user_prompt = Some("use sk-ant-oat01-SECRETVALUE please".into());
        let s = compose(None, Some(&r), &read_state::DeviceReadState::empty());
        let title = s.title.unwrap();
        assert!(!title.contains("SECRETVALUE"), "{title}");
    }

    fn pid_record(id: &str) -> PidRecord {
        PidRecord {
            pid: 42,
            session_id: id.to_string(),
            cwd: "/tmp/proj".into(),
            started_at_ms: 0,
            updated_at_ms: None,
            version: None,
            kind: None,
            entrypoint: None,
            name: None,
            status: None,
            waiting_for: None,
            messaging_socket_path: None,
            peer_protocol: None,
        }
    }

    #[test]
    fn a_live_session_without_an_inbox_is_shown_but_not_addressable() {
        let rec = pid_record("s1");
        let s = compose(Some(&rec), None, &read_state::DeviceReadState::empty());
        assert!(s.live, "a session we cannot message is still one to watch");
        assert!(!s.addressable);
    }

    #[test]
    fn a_live_session_with_an_inbox_is_addressable() {
        let mut rec = pid_record("s1");
        rec.messaging_socket_path = Some("/tmp/cc-socks/42.sock".into());
        rec.peer_protocol = Some(crate::peer::PEER_PROTOCOL);
        let s = compose(Some(&rec), None, &read_state::DeviceReadState::empty());
        assert!(s.addressable);
    }

    #[test]
    fn a_future_peer_protocol_is_not_addressable() {
        // The protocol pin is a hard refusal everywhere else; the panel
        // must not offer a composer the send path will reject.
        let mut rec = pid_record("s1");
        rec.messaging_socket_path = Some("/tmp/cc-socks/42.sock".into());
        rec.peer_protocol = Some(crate::peer::PEER_PROTOCOL + 1);
        let s = compose(Some(&rec), None, &read_state::DeviceReadState::empty());
        assert!(!s.addressable);
    }
}

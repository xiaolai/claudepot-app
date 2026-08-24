//! Reading a transcript for the panel's thread view.
//!
//! **This is the secret-bearing surface.** A transcript is the model's
//! output plus every tool result verbatim — on a real machine that has
//! included financial records, private paths, and credentials. It goes
//! through [`crate::session_live::redact::redact_secrets`], whose own
//! documentation says the list of families it covers is incomplete, so
//! the client is told the text is *masked where recognised* and never
//! that it is scrubbed. A user who believes a screen is scrubbed will
//! screenshot it.
//!
//! ## Paging
//!
//! The cursor is a **count of raw events consumed**, not the index of
//! the last one delivered. Both readings are defensible and mixing them
//! is an off-by-one that only shows up on a live session: the first
//! version returned `total` as the cursor and matched `index > cursor`,
//! so on a 100-event transcript the client asked for `> 100` and the
//! event that arrived at index 100 was never delivered. Nothing was
//! visibly broken — the thread simply stopped one message short until
//! something else appended.
//!
//! A count makes the empty case work too: an empty transcript yields
//! cursor `0`, and `index >= 0` is everything. A last-index cursor has
//! no value that means "nothing yet".
//!
//! Three windows, because a phone opening a 900-event transcript wants
//! the end:
//!
//! - [`Window::Tail`] — the last `limit`. What opening a thread does.
//! - [`Window::Since`] — indices at or above a cursor. Following along.
//! - [`Window::Before`] — the `limit` indices ending just under an
//!   index. Scrolling up. `before` is an **index**, not a cursor: it is
//!   read straight off an event the client already holds.
//!
//! ## Tool calls
//!
//! A `tool_use` and its `tool_result` render as one tick — but only when
//! both are inside the window being served. The join is **window-aware**
//! for exactly one reason: on a live session the call goes out in one
//! page and the result lands in the next, so a join that ran over the
//! whole file would attach the result to a tick whose index is below the
//! cursor and then slice that tick away. The output of the command would
//! simply never arrive, and nothing would look wrong.
//!
//! So a result whose call sits below the window is emitted as its own
//! tick. Two ticks is worse-looking than one; losing a command's output
//! because of where a page boundary fell is worse.
//!
//! ## Cost
//!
//! Every read parses the whole file. That is what the GUI does too, and
//! it is bounded by transcript size rather than by history: a session
//! big enough for this to hurt is one CC itself is already slow on. The
//! *list* endpoint does not pay it — [`tail_summary`] reads a bounded
//! window from the end of the file instead, because that one runs on
//! every poll.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::session::SessionEvent;
use crate::session_live::redact::redact_secrets;

/// Bytes read from the end of a transcript for the one-line summary.
///
/// Generous enough to contain the last few events on any real
/// transcript, small enough that the cost does not scale with the file.
const TAIL_BYTES: u64 = 64 * 1024;

/// The summary line on a live card.
const TAIL_CHARS: usize = 240;

/// A tool tick's inline preview.
const PREVIEW_CHARS: usize = 120;

/// A tool result's expandable body.
const DETAIL_CHARS: usize = 2000;

/// Prose bodies. Long enough for a real answer, short enough that one
/// runaway event cannot make a page unloadable.
const BODY_CHARS: usize = 8000;

#[derive(Debug, thiserror::Error)]
pub enum TranscriptError {
    #[error("no transcript on disk for this session")]
    NotFound,
    #[error("the transcript could not be read: {0}")]
    Unreadable(String),
    #[error("refusing to read a path outside the Claude Code projects directory")]
    OutsideProjects,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    User,
    Assistant,
    Thinking,
    Tool,
    System,
    Summary,
}

#[derive(Debug, Clone, Serialize)]
pub struct PanelEvent {
    /// Ordinal in the parsed event list. The paging cursor.
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<DateTime<Utc>>,
    pub kind: EventKind,
    /// Prose for a message; the call's argument preview for a tick.
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// The tool's output, for a tick the user expands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptPage {
    pub events: Vec<PanelEvent>,
    /// Raw events consumed. Pass back as `since` to get what arrived
    /// after this page — see the module docs on why this is a count and
    /// not the last delivered index.
    pub next_cursor: usize,
    /// Events in the whole transcript, so a client knows whether there
    /// is anything above the window it holds.
    pub total: usize,
}

#[derive(Debug, Clone, Copy)]
pub enum Window {
    Tail {
        limit: usize,
    },
    /// Events with raw index **at or above** `since`, which is a cursor
    /// from a previous page.
    Since {
        since: usize,
        limit: usize,
    },
    /// The `limit` events with raw index strictly **below** `before`,
    /// which is an index the client already holds.
    Before {
        before: usize,
        limit: usize,
    },
}

/// The largest page a client may ask for. A phone that asks for
/// everything gets a page, not the whole session.
pub const MAX_LIMIT: usize = 200;

/// Read one page of a transcript.
///
/// `file_path` comes from `session_index`, which only ever holds paths
/// under `<config_dir>/projects/`; it is never a client-supplied string.
/// The containment check runs anyway and fails closed, because "the
/// caller only passes indexed paths" is a property of today's callers
/// and not of this function.
pub fn page(
    config_dir: &Path,
    file_path: &Path,
    window: Window,
) -> Result<TranscriptPage, TranscriptError> {
    guard_under_projects(config_dir, file_path)?;
    if !file_path.exists() {
        return Err(TranscriptError::NotFound);
    }
    let raw = crate::session::core::parse_events_public(file_path)
        .map_err(|e| TranscriptError::Unreadable(e.to_string()))?;

    let total = raw.len();
    // The join's lower bound is the window's. Everything below it has
    // already been delivered, so a result down there has to stand alone.
    let emit_from = match window {
        Window::Since { since, .. } => since,
        Window::Tail { .. } | Window::Before { .. } => 0,
    };
    let all = fold(&raw, emit_from);

    let (slice, next_cursor) = match window {
        Window::Tail { limit } => {
            let limit = limit.clamp(1, MAX_LIMIT);
            let start = all.len().saturating_sub(limit);
            (&all[start..], total)
        }
        Window::Since { since, limit } => {
            let limit = limit.clamp(1, MAX_LIMIT);
            let from = all.partition_point(|e| e.index < since);
            let end = (from + limit).min(all.len());
            // Once the window reaches the end, the cursor is the whole
            // file's length — so an event kind this fold drops (a
            // malformed line, say) cannot stall the client forever
            // asking for the same cursor.
            let cursor = if end == all.len() {
                total
            } else {
                all[end - 1].index + 1
            };
            (&all[from..end], cursor)
        }
        Window::Before { before, limit } => {
            let limit = limit.clamp(1, MAX_LIMIT);
            let end = all.partition_point(|e| e.index < before);
            let start = end.saturating_sub(limit);
            (&all[start..end], total)
        }
    };

    Ok(TranscriptPage {
        events: slice.to_vec(),
        next_cursor,
        total,
    })
}

/// The most recent line of prose, for the live card.
///
/// Reads a bounded window from the end rather than parsing the file:
/// this runs on every list poll, and the list poll must not scale with
/// transcript size.
/// What a card needs from the tail, read in one pass.
///
/// Both values come from the same escalating scan because the list
/// re-reads every live transcript on every poll, and two functions each
/// doing their own would double that for no gain.
#[derive(Debug, Default, Clone)]
pub(super) struct TailMarks {
    /// The last thing the **user** actually typed — the card's title.
    ///
    /// A card used to be titled from the row's stored `first_user_prompt`,
    /// which on a thread running for hours names a question settled long
    /// ago.
    ///
    /// Skips [`is_harness_text`]: Claude Code writes `<command-name>`,
    /// `<bash-stdout>`, `<system-reminder>` and friends into the USER
    /// role, and a title reading `<bash-stdout>` would be worse than the
    /// stale one it replaced. Tool results are user-role by schema and
    /// never typed by anyone, so they are skipped too.
    pub prompt: Option<String>,
    /// When Claude last **replied in prose** — the closest thing in a
    /// transcript to "a job finished".
    ///
    /// Deliberately not the last event of any kind. A session grinding
    /// through a hundred tool calls emits an event every second or two
    /// and would sit permanently at the top of the list while having
    /// told the user nothing since breakfast. Ordering on this asks
    /// "what has actually come back to me recently", which is the
    /// question the list exists to answer.
    pub replied_at: Option<DateTime<Utc>>,
}

pub(super) fn tail_marks(file_path: &Path) -> TailMarks {
    // Escalating windows, cheapest first. A tool-heavy session can put
    // megabytes of output between two prompts: measured on a real 14 MB
    // transcript, the last 64 KB held **zero** user turns while 512 KB
    // held three, the newest being the prompt actually wanted. A single
    // large window would pay that cost on every session on every poll;
    // this pays it only where the cheap read came up short.
    //
    // The cap is deliberate. Past it the honest answer is "nothing
    // recent in reach" and the caller falls back — better than reading
    // a whole transcript every five seconds to name a card.
    const WINDOWS: [u64; 2] = [TAIL_BYTES, 512 * 1024];
    let mut last_len = 0usize;
    let mut marks = TailMarks::default();
    for window in WINDOWS {
        let Some(events) = tail_events(file_path, window) else {
            continue;
        };
        // A larger window that read no further means the file is
        // smaller than the window — escalating again cannot help.
        let grew = events.len() > last_len;
        last_len = events.len();

        marks = TailMarks::default();
        for e in events.iter().rev() {
            match e {
                SessionEvent::UserText { text, .. }
                    if marks.prompt.is_none() && !is_harness_text(text) =>
                {
                    let t = text.trim();
                    if !t.is_empty() {
                        marks.prompt = Some(t.to_string());
                    }
                }
                SessionEvent::AssistantText { ts, text, .. }
                    if marks.replied_at.is_none() && !text.trim().is_empty() =>
                {
                    marks.replied_at = *ts;
                }
                _ => {}
            }
            if marks.prompt.is_some() && marks.replied_at.is_some() {
                break;
            }
        }
        if marks.prompt.is_some() || !grew {
            break;
        }
    }
    marks
}

pub fn tail_summary(file_path: &Path) -> Option<String> {
    let events = tail_events(file_path, TAIL_BYTES)?;
    events.iter().rev().find_map(|e| match e {
        SessionEvent::AssistantText { text, .. } if !text.trim().is_empty() => {
            Some(clip(&redact_secrets(text), TAIL_CHARS))
        }
        SessionEvent::TaskSummary { summary, .. } if !summary.trim().is_empty() => {
            Some(clip(&redact_secrets(summary), TAIL_CHARS))
        }
        _ => None,
    })
}

/// Complete lines from the last `bytes` of a file.
///
/// The first line of the window is dropped unless the window starts at
/// byte zero — it is almost certainly a fragment, and a fragment parses
/// as a malformed event rather than as nothing.
///
/// Shared with [`super::ask`], which reads a larger window for the same
/// reason. It was written twice before: two copies of "seek, read,
/// lossy-decode, split, drop the fragment" that differed only in the
/// byte limit, which is one bug fixed in one place away from divergence.
pub(super) fn tail_lines(path: &Path, bytes: u64) -> Option<Vec<String>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let from = len.saturating_sub(bytes);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::with_capacity(bytes as usize);
    f.take(bytes).read_to_end(&mut buf).ok()?;

    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
    if from > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    Some(lines)
}

/// Parse a bounded tail of a transcript into events. The other half of
/// what both tail readers wanted.
pub(super) fn tail_events(path: &Path, bytes: u64) -> Option<Vec<SessionEvent>> {
    let lines = tail_lines(path, bytes)?;
    let mut events = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        crate::session::core::parse_line_into(&mut events, line, i + 1);
    }
    Some(events)
}

/// Turn parsed events into what a phone renders, joining each tool call
/// to its result.
///
/// `emit_from` is the lowest raw index the caller will actually serve. A
/// result whose call is below it cannot be joined — the joined tick
/// would be sliced away and the result with it — so it becomes its own
/// tick instead. See the module docs.
fn fold(raw: &[SessionEvent], emit_from: usize) -> Vec<PanelEvent> {
    // tool_use_id -> (index into `out`)
    let mut tick_at: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut out: Vec<PanelEvent> = Vec::with_capacity(raw.len());

    for (index, ev) in raw.iter().enumerate() {
        if index < emit_from {
            // Below the window. Recorded nowhere, so nothing above can
            // join to it.
            continue;
        }
        match ev {
            SessionEvent::UserText { ts, text, .. } => {
                // Claude Code injects machinery into the user role —
                // command caveats, stdout captures, interrupt notices.
                // Rendering those as something the user typed
                // misattributes them, and rendering them as anything
                // else is a row that says nothing on a phone.
                if text.trim().is_empty() || is_harness_text(text) {
                    continue;
                }
                out.push(PanelEvent {
                    index,
                    ts: *ts,
                    kind: EventKind::User,
                    text: clip(&redact_secrets(text), BODY_CHARS),
                    tool_name: None,
                    detail: None,
                    is_error: false,
                });
            }
            SessionEvent::AssistantText { ts, text, .. } => {
                if text.trim().is_empty() {
                    continue;
                }
                out.push(PanelEvent {
                    index,
                    ts: *ts,
                    kind: EventKind::Assistant,
                    text: clip(&redact_secrets(text), BODY_CHARS),
                    tool_name: None,
                    detail: None,
                    is_error: false,
                });
            }
            SessionEvent::AssistantThinking { ts, text, .. } => {
                if text.trim().is_empty() {
                    continue;
                }
                out.push(PanelEvent {
                    index,
                    ts: *ts,
                    kind: EventKind::Thinking,
                    text: clip(&redact_secrets(text), BODY_CHARS),
                    tool_name: None,
                    detail: None,
                    is_error: false,
                });
            }
            SessionEvent::AssistantToolUse {
                ts,
                tool_name,
                tool_use_id,
                input_preview,
                input_full,
                ..
            } => {
                tick_at.insert(tool_use_id.as_str(), out.len());
                out.push(PanelEvent {
                    index,
                    ts: *ts,
                    kind: EventKind::Tool,
                    text: clip(
                        &redact_secrets(&tool_argument(input_full, input_preview)),
                        PREVIEW_CHARS,
                    ),
                    tool_name: Some(tool_name.clone()),
                    detail: None,
                    is_error: false,
                });
            }
            SessionEvent::UserToolResult {
                ts,
                tool_use_id,
                content,
                is_error,
                ..
            } => {
                let body = clip(&redact_secrets(content), DETAIL_CHARS);
                // Joinable only when the call will be served in this
                // window too. `tick_at` holds nothing below `emit_from`,
                // so a call already delivered falls through to the
                // stand-alone branch — which is the live case, not an
                // edge one.
                match tick_at.get(tool_use_id.as_str()) {
                    Some(&at) => {
                        out[at].detail = Some(body);
                        out[at].is_error = *is_error;
                    }
                    None => {
                        // Either the call is below the window, or it is
                        // not in this transcript at all — a resumed
                        // session, or a result CC wrote first.
                        out.push(PanelEvent {
                            index,
                            ts: *ts,
                            kind: EventKind::Tool,
                            text: String::new(),
                            tool_name: Some("result".into()),
                            detail: Some(body),
                            is_error: *is_error,
                        });
                    }
                }
            }
            SessionEvent::TaskSummary { ts, summary, .. } => out.push(PanelEvent {
                index,
                ts: *ts,
                kind: EventKind::Summary,
                text: clip(&redact_secrets(summary), BODY_CHARS),
                tool_name: None,
                detail: None,
                is_error: false,
            }),
            // `SessionEvent::System` is deliberately not rendered.
            // Its `detail` is CC's `level` field — "suggestion",
            // "error" — never the notice body, which the parser does
            // not carry. A row reading "System: suggestion" is a row
            // that says nothing, and a real transcript is full of
            // `stop_hook_summary` records that produce exactly that.
            //
            // The one system notice that *would* matter is a held peer
            // message. Surfacing it means teaching `parse_line_into` to
            // carry the body first; until then this is an omission, not
            // a filter. See the `transcript JSONL schema` row in
            // `crates/xtask/cc-upstream-watch.md`.
            //
            // Attachments, file snapshots, unknown kinds and malformed
            // lines carry nothing a phone can act on either. All of them
            // keep their index — the cursor counts raw events — and none
            // are rendered.
            _ => {}
        }
    }
    out
}

/// The one argument worth putting on a one-line tick.
///
/// `input_preview` is the raw serialized input trimmed to 240 chars, so
/// a `Write` renders as `{"content":"# Ambages — UX brief\n` … — the
/// least useful 240 characters available. Every tool worth showing puts
/// its subject in one of a handful of keys, so the first one present
/// wins and anything unrecognised falls back to the preview.
///
/// Order matters: `Edit` carries both `file_path` and `old_string`, and
/// the path is the subject. This is a **display** heuristic with a total
/// fallback — it never decides anything, so a tool CC adds tomorrow
/// degrades to the preview rather than breaking.
pub(super) fn tool_argument(input_full: &str, input_preview: &str) -> String {
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
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(input_full)
    else {
        return input_preview.to_string();
    };
    for key in SUBJECT_KEYS {
        if let Some(s) = map.get(*key).and_then(|v| v.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                // Newlines collapse: a heredoc in a Bash command must
                // not turn one tick into forty rows.
                return s.split_whitespace().collect::<Vec<_>>().join(" ");
            }
        }
    }
    input_preview.to_string()
}

/// Text Claude Code injects into the user role rather than the user
/// typing it. The markers are CC-internal and drift; see the
/// `transcript JSONL schema` row in `crates/xtask/cc-upstream-watch.md`.
fn is_harness_text(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "<local-command-caveat>",
        "<command-name>",
        "<command-message>",
        "<bash-stdout>",
        "<bash-stderr>",
        "[Request interrupted by user",
        // Found leaking into a card title on real data: CC posts a
        // subagent's completion into the USER role, so a card read
        // `<task-notification> <task-id>a5ba8459…`. A scan of 40 recent
        // transcripts turned up exactly two shapes that open with a tag
        // — this one and the interrupt above.
        "<task-notification>",
        "<system-reminder>",
    ];
    // **Anchored to a line start, not `contains`.** A bare substring
    // test drops any user message that so much as MENTIONS one of these
    // — "what does `<system-reminder>` actually do?" vanished from the
    // transcript and could never become a card title. CC always emits
    // these as a block that opens its own line, which is the property
    // worth matching.
    //
    // Line-anchored rather than text-anchored because CC appends a
    // `<system-reminder>` block to the end of an otherwise ordinary
    // turn as well as sending one alone; anchoring to the whole string
    // would let that leak back into titles.
    text.lines()
        .any(|line| MARKERS.iter().any(|m| line.trim_start().starts_with(m)))
}

/// The containment gate. Neither side is *canonicalized* — `file_path`
/// originates in a walk of `<config_dir>/projects/`, so it is already
/// rooted there, and canonicalizing on every page read would stat the
/// whole chain.
///
/// Both sides ARE run through `simplify_windows_path` first, per
/// `.claude/rules/paths.md`. `std::fs::canonicalize` on Windows returns
/// the verbatim `\\?\C:\…` form, so a stored path that went through it
/// and a `config_dir` that did not are the same directory written two
/// ways — and `Path::starts_with` compares components, so it says no.
/// That direction fails *closed*: a legitimate transcript is refused
/// rather than a foreign one admitted. Still wrong, and invisible on
/// macOS and Linux where the call is a no-op.
fn guard_under_projects(config_dir: &Path, file_path: &Path) -> Result<(), TranscriptError> {
    let simplify = |p: &Path| {
        std::path::PathBuf::from(crate::path_utils::simplify_windows_path(
            &p.to_string_lossy(),
        ))
    };
    let projects = simplify(&config_dir.join("projects"));
    let file_path = &simplify(file_path);
    if !file_path.starts_with(&projects) {
        return Err(TranscriptError::OutsideProjects);
    }
    if file_path.extension().map(|e| e != "jsonl").unwrap_or(true) {
        return Err(TranscriptError::OutsideProjects);
    }
    // A `..` anywhere means the string was assembled, not walked.
    if file_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(TranscriptError::OutsideProjects);
    }
    Ok(())
}

fn clip(s: &str, max: usize) -> String {
    super::truncate(s, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_transcript(dir: &Path, slug: &str, name: &str, lines: &[&str]) -> std::path::PathBuf {
        let d = dir.join("projects").join(slug);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join(format!("{name}.jsonl"));
        let mut f = std::fs::File::create(&p).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        p
    }

    fn user(text: &str) -> String {
        serde_json::json!({
            "type": "user",
            "timestamp": "2026-08-23T01:00:00Z",
            "message": { "role": "user", "content": text }
        })
        .to_string()
    }

    fn assistant(text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-08-23T01:00:01Z",
            "message": { "role": "assistant", "model": "claude-opus-5",
                         "content": [{ "type": "text", "text": text }] }
        })
        .to_string()
    }

    fn tool_use(id: &str, name: &str, arg: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-08-23T01:00:02Z",
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": id, "name": name, "input": { "command": arg } }
            ]}
        })
        .to_string()
    }

    fn tool_result(id: &str, body: &str, is_error: bool) -> String {
        serde_json::json!({
            "type": "user",
            "timestamp": "2026-08-23T01:00:03Z",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": id, "content": body, "is_error": is_error }
            ]}
        })
        .to_string()
    }

    fn assistant_at(ts: &str, text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "timestamp": ts,
            "message": { "role": "assistant", "model": "claude-opus-5",
                         "content": [{ "type": "text", "text": text }] }
        })
        .to_string()
    }

    #[test]
    fn the_title_is_the_last_thing_the_user_typed() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[
                &user("first question"),
                &assistant("ok"),
                &user("second question"),
            ],
        );
        assert_eq!(tail_marks(&p).prompt.as_deref(), Some("second question"));
    }

    #[test]
    fn text_claude_code_wrote_into_the_user_role_is_not_a_title() {
        // Every one of these is CC talking to itself. A card reading
        // `<bash-stdout>` would be worse than a stale title, and
        // `<task-notification>` was found doing exactly that on real
        // data — a live card read `<task-notification> <task-id>a5ba…`.
        let tmp = tempfile::tempdir().unwrap();
        for injected in [
            "<task-notification> <task-id>abc</task-id>",
            "<command-name>/compact</command-name>",
            "<bash-stdout>ok</bash-stdout>",
            "<system-reminder>note</system-reminder>",
            "[Request interrupted by user]",
        ] {
            let p = write_transcript(
                tmp.path(),
                "-tmp-p",
                "s-inj",
                &[
                    &user("what I actually typed"),
                    &assistant("ok"),
                    &user(injected),
                ],
            );
            assert_eq!(
                tail_marks(&p).prompt.as_deref(),
                Some("what I actually typed"),
                "{injected} must not become a title"
            );
        }
    }

    #[test]
    fn replied_at_is_the_last_prose_turn_not_the_last_event() {
        // The whole point of ordering on it: a session grinding through
        // tool calls emits an event every second or two and would sit
        // permanently at the top of the list while having told the user
        // nothing since its last actual answer.
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s2",
            &[
                &user("go"),
                &assistant_at("2026-08-23T01:00:00Z", "here is the answer"),
                &tool_use("t9", "Bash", "cargo test"),
                &tool_result("t9", "ok", false),
            ],
        );
        let marks = tail_marks(&p);
        assert_eq!(
            marks.replied_at.map(|t| t.to_rfc3339()),
            Some("2026-08-23T01:00:00+00:00".to_string()),
            "tool traffic after the reply must not move it"
        );
    }

    #[test]
    fn a_transcript_with_no_reply_yet_has_no_reply_mark() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(tmp.path(), "-tmp-p", "s3", &[&user("just asked")]);
        let marks = tail_marks(&p);
        assert_eq!(marks.prompt.as_deref(), Some("just asked"));
        assert!(marks.replied_at.is_none(), "nothing has come back yet");
    }

    #[test]
    fn a_tool_call_and_its_result_render_as_one_tick() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[
                &user("go"),
                &tool_use("t1", "Bash", "ls"),
                &tool_result("t1", "a\nb", false),
            ],
        );
        let page = page(tmp.path(), &p, Window::Tail { limit: 50 }).unwrap();
        let ticks: Vec<_> = page
            .events
            .iter()
            .filter(|e| e.kind == EventKind::Tool)
            .collect();
        assert_eq!(ticks.len(), 1, "the result must fold into the call");
        assert_eq!(ticks[0].tool_name.as_deref(), Some("Bash"));
        assert!(ticks[0].detail.as_deref().unwrap().contains('a'));
    }

    #[test]
    fn an_errored_result_marks_its_tick() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[
                &tool_use("t1", "Bash", "false"),
                &tool_result("t1", "boom", true),
            ],
        );
        let page = page(tmp.path(), &p, Window::Tail { limit: 50 }).unwrap();
        assert!(page.events.iter().any(|e| e.is_error));
    }

    #[test]
    fn a_result_arriving_after_its_call_was_delivered_still_shows() {
        // The live case, and the one the docs used to claim was handled
        // while the code dropped it: the call goes out in one page, the
        // result lands in the next. Joining across the whole file
        // attached the result to a tick below the cursor, and the slice
        // then removed both — the command's output simply never arrived.
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[&user("go"), &tool_use("t1", "Bash", "sleep 5")],
        );
        let first = page(tmp.path(), &p, Window::Tail { limit: 60 }).unwrap();
        assert!(first
            .events
            .iter()
            .any(|e| e.tool_name.as_deref() == Some("Bash")));

        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{}", tool_result("t1", "the output", false)).unwrap();
        drop(f);

        let next = page(
            tmp.path(),
            &p,
            Window::Since {
                since: first.next_cursor,
                limit: 60,
            },
        )
        .unwrap();
        assert_eq!(next.events.len(), 1, "the result was dropped");
        assert_eq!(next.events[0].detail.as_deref(), Some("the output"));
    }

    #[test]
    fn a_result_whose_call_is_not_here_still_shows() {
        // The live case: the call went out in an earlier page. Dropping
        // the result would lose a command's output because of where a
        // page boundary fell.
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[&tool_result("orphan", "output", false)],
        );
        let page = page(tmp.path(), &p, Window::Tail { limit: 50 }).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].detail.as_deref(), Some("output"));
    }

    #[test]
    fn tail_returns_the_end_and_before_walks_back() {
        let tmp = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (0..10).map(|i| assistant(&format!("line {i}"))).collect();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let p = write_transcript(tmp.path(), "-tmp-p", "s1", &refs);

        let tail = page(tmp.path(), &p, Window::Tail { limit: 3 }).unwrap();
        assert_eq!(tail.events.len(), 3);
        assert!(tail.events[2].text.contains("line 9"));
        assert_eq!(tail.total, 10);

        let earlier = page(
            tmp.path(),
            &p,
            Window::Before {
                before: tail.events[0].index,
                limit: 3,
            },
        )
        .unwrap();
        assert_eq!(earlier.events.len(), 3);
        assert!(earlier.events[2].text.contains("line 6"));
    }

    #[test]
    fn a_cursor_handed_back_verbatim_loses_nothing() {
        // The bug this locks down: `next_cursor` was the index one past
        // the last delivered event while the window matched
        // `index > cursor`, so handing the cursor back skipped exactly
        // one event — invisibly, because the thread just sat one message
        // behind until something else appended.
        let tmp = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (0..5).map(|i| assistant(&format!("line {i}"))).collect();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let p = write_transcript(tmp.path(), "-tmp-p", "s1", &refs);

        let mut seen: Vec<String> = Vec::new();
        let mut cursor = 0;
        loop {
            let got = page(
                tmp.path(),
                &p,
                Window::Since {
                    since: cursor,
                    limit: 2,
                },
            )
            .unwrap();
            if got.events.is_empty() {
                break;
            }
            seen.extend(got.events.iter().map(|e| e.text.clone()));
            cursor = got.next_cursor;
        }
        assert_eq!(
            seen.len(),
            5,
            "paging with the returned cursor dropped events: {seen:?}"
        );
        for i in 0..5 {
            assert!(
                seen.iter().any(|t| t.contains(&format!("line {i}"))),
                "lost line {i}"
            );
        }
    }

    #[test]
    fn the_tail_cursor_delivers_the_next_appended_event() {
        // The live case, and the one that was broken: open a thread,
        // hand the tail's cursor to the next poll, and the event that
        // lands immediately after must arrive.
        use std::io::Write as _;
        let tmp = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (0..3).map(|i| assistant(&format!("line {i}"))).collect();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let p = write_transcript(tmp.path(), "-tmp-p", "s1", &refs);

        let tail = page(tmp.path(), &p, Window::Tail { limit: 60 }).unwrap();

        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{}", assistant("the new one")).unwrap();
        drop(f);

        let next = page(
            tmp.path(),
            &p,
            Window::Since {
                since: tail.next_cursor,
                limit: 60,
            },
        )
        .unwrap();
        assert_eq!(next.events.len(), 1, "the appended event was skipped");
        assert!(next.events[0].text.contains("the new one"));
    }

    #[test]
    fn a_cursor_past_the_end_returns_nothing_and_still_points_past_the_end() {
        let tmp = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (0..5).map(|i| assistant(&format!("line {i}"))).collect();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let p = write_transcript(tmp.path(), "-tmp-p", "s1", &refs);
        let done = page(
            tmp.path(),
            &p,
            Window::Since {
                since: 99,
                limit: 50,
            },
        )
        .unwrap();
        assert!(done.events.is_empty());
        assert_eq!(done.next_cursor, 5);
    }

    #[test]
    fn a_zero_cursor_on_an_empty_transcript_is_not_a_special_case() {
        // A last-index cursor has no value meaning "nothing yet"; a
        // count does, and this is the test that says so.
        use std::io::Write as _;
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(tmp.path(), "-tmp-p", "s1", &[]);
        let empty = page(
            tmp.path(),
            &p,
            Window::Since {
                since: 0,
                limit: 50,
            },
        )
        .unwrap();
        assert!(empty.events.is_empty());
        assert_eq!(empty.next_cursor, 0);

        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{}", assistant("first ever")).unwrap();
        drop(f);

        let first = page(
            tmp.path(),
            &p,
            Window::Since {
                since: 0,
                limit: 50,
            },
        )
        .unwrap();
        assert_eq!(
            first.events.len(),
            1,
            "the very first event must not be skipped"
        );
    }

    #[test]
    fn an_unrendered_trailing_event_does_not_stall_the_cursor() {
        // The bug this prevents: the last raw event is an attachment,
        // which `fold` drops. If `next_cursor` came from the last
        // *rendered* event, the client would ask for the same `after`
        // forever and never learn the file had grown.
        let tmp = tempfile::tempdir().unwrap();
        let attachment =
            serde_json::json!({ "type": "attachment", "timestamp": "2026-08-23T01:00:09Z" })
                .to_string();
        let p = write_transcript(tmp.path(), "-tmp-p", "s1", &[&assistant("hi"), &attachment]);
        let got = page(
            tmp.path(),
            &p,
            Window::Since {
                since: 0,
                limit: 50,
            },
        )
        .unwrap();
        assert_eq!(
            got.next_cursor, 2,
            "cursor must count raw events, not rendered ones"
        );
    }

    #[test]
    fn secrets_are_masked_in_prose_and_in_tool_output() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[
                &assistant("your key is sk-ant-oat01-LEAKEDVALUE ok"),
                &tool_use("t1", "Bash", "echo"),
                &tool_result("t1", "Authorization: Bearer LEAKEDBEARER", false),
            ],
        );
        let got = page(tmp.path(), &p, Window::Tail { limit: 50 }).unwrap();
        let blob = serde_json::to_string(&got).unwrap();
        assert!(!blob.contains("LEAKEDVALUE"), "{blob}");
        assert!(!blob.contains("LEAKEDBEARER"), "{blob}");
    }

    #[test]
    fn harness_injected_text_is_not_attributed_to_the_user() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[
                &user("<command-name>/clear</command-name>"),
                &user("real question"),
            ],
        );
        let got = page(tmp.path(), &p, Window::Tail { limit: 50 }).unwrap();
        assert_eq!(got.events.len(), 1, "harness plumbing is not a turn");
        assert_eq!(got.events[0].kind, EventKind::User);
        assert!(got.events[0].text.contains("real question"));
    }

    #[test]
    fn a_stop_hook_summary_is_not_a_row() {
        // Measured on a real transcript: `SessionEvent::System` carries
        // CC's `level` field, so these rendered as "System: suggestion"
        // — a row saying nothing, twice per assistant turn.
        let tmp = tempfile::tempdir().unwrap();
        let hook = serde_json::json!({
            "type": "system",
            "subtype": "stop_hook_summary",
            "level": "suggestion",
            "timestamp": "2026-08-23T01:00:00Z"
        })
        .to_string();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[&hook, &assistant("real answer")],
        );
        let got = page(tmp.path(), &p, Window::Tail { limit: 50 }).unwrap();
        assert_eq!(got.events.len(), 1);
        assert_eq!(got.events[0].kind, EventKind::Assistant);
        assert_eq!(
            got.total, 2,
            "the dropped row still counts toward the cursor"
        );
    }

    #[test]
    fn a_tick_shows_the_subject_not_the_first_240_bytes_of_json() {
        assert_eq!(
            tool_argument(
                r##"{"file_path":"dev-docs/ux-brief.md","content":"# Long doc"}"##,
                "PREVIEW"
            ),
            "dev-docs/ux-brief.md"
        );
        assert_eq!(
            tool_argument("{\"command\":\"git add -A\\n  git commit\"}", "PREVIEW"),
            "git add -A git commit",
            "a heredoc must not turn one tick into forty rows"
        );
        // `Edit` carries both; the path is the subject.
        assert_eq!(
            tool_argument(
                r#"{"old_string":"a","file_path":"/x/y.rs","new_string":"b"}"#,
                "PREVIEW"
            ),
            "/x/y.rs"
        );
    }

    #[test]
    fn an_unrecognised_tool_input_falls_back_to_the_preview() {
        // A display heuristic must never decide anything. A tool CC adds
        // tomorrow degrades rather than breaking.
        assert_eq!(
            tool_argument(r#"{"brand_new_key":"x"}"#, "PREVIEW"),
            "PREVIEW"
        );
        assert_eq!(tool_argument("not json", "PREVIEW"), "PREVIEW");
        assert_eq!(tool_argument("[1,2,3]", "PREVIEW"), "PREVIEW");
        assert_eq!(tool_argument(r#"{"command":"   "}"#, "PREVIEW"), "PREVIEW");
    }

    #[test]
    fn a_path_outside_the_projects_directory_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("elsewhere.jsonl");
        std::fs::write(&outside, "{}").unwrap();
        assert!(matches!(
            page(tmp.path(), &outside, Window::Tail { limit: 1 }),
            Err(TranscriptError::OutsideProjects)
        ));

        let traversal = tmp.path().join("projects").join("..").join("secrets.jsonl");
        assert!(matches!(
            page(tmp.path(), &traversal, Window::Tail { limit: 1 }),
            Err(TranscriptError::OutsideProjects)
        ));

        let wrong_ext = tmp.path().join("projects").join("x").join("id_rsa");
        assert!(matches!(
            page(tmp.path(), &wrong_ext, Window::Tail { limit: 1 }),
            Err(TranscriptError::OutsideProjects)
        ));
    }

    #[test]
    fn a_limit_beyond_the_cap_is_clamped_not_honoured() {
        let tmp = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (0..MAX_LIMIT + 50)
            .map(|i| assistant(&format!("l{i}")))
            .collect();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let p = write_transcript(tmp.path(), "-tmp-p", "s1", &refs);
        let got = page(
            tmp.path(),
            &p,
            Window::Since {
                since: 0,
                limit: 100_000,
            },
        )
        .unwrap();
        assert_eq!(got.events.len(), MAX_LIMIT);
    }

    #[test]
    fn the_tail_summary_reads_the_end_without_parsing_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lines: Vec<String> = (0..200)
            .map(|i| assistant(&format!("filler {i}")))
            .collect();
        lines.push(assistant("the last thing said"));
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let p = write_transcript(tmp.path(), "-tmp-p", "s1", &refs);
        assert_eq!(tail_summary(&p).as_deref(), Some("the last thing said"));
    }

    #[test]
    fn the_tail_summary_skips_a_truncated_first_line() {
        // A window that starts mid-line yields a fragment. Parsing it
        // would produce a Malformed event; keeping it could produce a
        // half-string on screen.
        let tmp = tempfile::tempdir().unwrap();
        let filler = "x".repeat(70_000);
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[&assistant(&filler), &assistant("visible tail")],
        );
        assert_eq!(tail_summary(&p).as_deref(), Some("visible tail"));
    }

    #[test]
    fn the_tail_summary_is_none_when_the_end_is_all_tool_traffic() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_transcript(
            tmp.path(),
            "-tmp-p",
            "s1",
            &[
                &tool_use("t1", "Bash", "ls"),
                &tool_result("t1", "out", false),
            ],
        );
        assert_eq!(tail_summary(&p), None);
    }

    /// `Path::starts_with` splits on `\` only on Windows, so the
    /// verbatim-prefix case can only be *exercised* there —
    /// `.claude/rules/paths.md` puts OS-specific path behaviour behind
    /// a cfg gate for exactly this reason, and CI runs the matrix.
    #[cfg(target_os = "windows")]
    #[test]
    fn the_containment_gate_accepts_a_verbatim_windows_path() {
        // `canonicalize` on Windows yields `\\?\C:\…`, so a stored path
        // that went through it and a `config_dir` that did not are the
        // same directory spelled two ways. Rust models the verbatim
        // prefix as its own `Component::Prefix`, so `starts_with` says
        // no and the guard refused a legitimate transcript.
        let config = Path::new(r"\\?\C:\Users\dev\.claude");
        let file = Path::new(r"C:\Users\dev\.claude\projects\-c-work\s.jsonl");
        assert!(guard_under_projects(config, file).is_ok());

        let config = Path::new(r"C:\Users\dev\.claude");
        let file = Path::new(r"\\?\C:\Users\dev\.claude\projects\-c-work\s.jsonl");
        assert!(guard_under_projects(config, file).is_ok());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn normalising_does_not_widen_the_windows_gate() {
        let config = Path::new(r"\\?\C:\Users\dev\.claude");
        for bad in [
            r"C:\Users\dev\.claude\notprojects\s.jsonl",
            r"\\?\UNC\server\share\projects\s.jsonl",
            r"C:\Users\dev\.claude\projects\..\..\secrets\s.jsonl",
            r"C:\Users\dev\.claude\projects\-c-work\s.txt",
        ] {
            assert!(
                guard_under_projects(config, Path::new(bad)).is_err(),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn the_containment_gate_still_refuses_what_it_always_did() {
        // Runs everywhere: these are the shapes the guard sees on this
        // host, and normalising must not have widened it.
        let cfg = Path::new("/home/dev/.claude");
        assert!(
            guard_under_projects(cfg, Path::new("/home/dev/.claude/projects/-work/s.jsonl"))
                .is_ok()
        );
        for bad in [
            "/etc/passwd",
            "/home/dev/.claude/notprojects/s.jsonl",
            "/home/dev/.claude/projects/../../secrets/s.jsonl",
            "/home/dev/.claude/projects/-work/s.txt",
        ] {
            assert!(
                guard_under_projects(cfg, Path::new(bad)).is_err(),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn a_user_who_mentions_a_harness_marker_is_not_erased() {
        // The bug: `contains` meant quoting a marker deleted your own
        // message from the transcript. Asking about the harness is an
        // ordinary thing to do in this repo.
        for real in [
            "what does `<system-reminder>` actually do?",
            "grep for <command-name> in the transcript please",
            "the docs mention <bash-stdout> — is that ours?",
            "I saw [Request interrupted by user] in the log, why?",
        ] {
            assert!(!is_harness_text(real), "{real} is a real user message");
        }
    }

    #[test]
    fn harness_blocks_are_still_filtered() {
        // Both shapes CC actually emits: a block on its own, and one
        // appended to the end of a real turn.
        for injected in [
            "<system-reminder>\nsomething\n</system-reminder>",
            "<command-name>/clear</command-name>",
            "  <local-command-caveat>Caveat: the messages below…",
            "<task-notification>\n<task-id>abc</task-id>",
            "[Request interrupted by user]",
            "please do the thing\n\n<system-reminder>\ncontext\n</system-reminder>",
        ] {
            assert!(is_harness_text(injected), "{injected:?} is harness text");
        }
    }
}

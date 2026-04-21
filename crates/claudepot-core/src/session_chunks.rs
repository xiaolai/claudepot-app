//! Group classified `SessionEvent`s into rendering chunks.
//!
//! A chunk is a stretch of events that belongs together on screen:
//!
//! | Chunk    | Starts on                                 | Absorbs                             |
//! |----------|-------------------------------------------|--------------------------------------|
//! | `User`   | A `User` (real input) event               | Nothing — one event per chunk.       |
//! | `System` | A `System` (slash-command stdout) event   | Nothing — one event per chunk.       |
//! | `Compact`| A `Compact` (summary) event               | Nothing — one event per chunk.       |
//! | `Ai`     | The first `Ai` event after a non-Ai chunk | All following `Ai` events until the  |
//! |          |                                           | next non-`Ai` classification.        |
//!
//! `HardNoise` events are dropped entirely. The first chunk can be an
//! `Ai` chunk even without a leading `User` — a session can start with
//! tool traffic (e.g. an adopted transcript that begins mid-flight).
//!
//! Ported from claude-devtools' `ChunkBuilder.ts`, minus the Electron
//! plumbing; we keep the four chunk variants, metrics aggregation, and
//! the tool-execution rollup inside AI chunks.

use crate::session::{SessionEvent, TokenUsage};
use crate::session_classify::{classify_event, MessageCategory};
use crate::session_tool_link::{link_tools, LinkedTool};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Rolled-up metrics for a chunk. Mirrors claude-devtools'
/// `SessionMetrics` in spirit but uses the `TokenUsage` breakdown we
/// already have in core.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ChunkMetrics {
    pub duration_ms: i64,
    pub tokens: TokenUsage,
    pub message_count: usize,
    pub tool_call_count: usize,
    pub thinking_count: usize,
}

/// Common fields on every chunk. Flattened into each variant via serde
/// so the frontend can parse a flat JSON object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkHeader {
    /// Stable ordinal, 0-based — the position of the chunk in the
    /// produced vector. Stable across re-reads of the same transcript.
    pub id: usize,
    pub start_ts: Option<DateTime<Utc>>,
    pub end_ts: Option<DateTime<Utc>>,
    pub metrics: ChunkMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "chunkType", rename_all = "camelCase")]
pub enum SessionChunk {
    /// A single genuine user input.
    #[serde(rename = "user")]
    User {
        #[serde(flatten)]
        header: ChunkHeader,
        event_index: usize,
    },
    /// Consecutive run of assistant text / thinking / tool traffic.
    #[serde(rename = "ai")]
    Ai {
        #[serde(flatten)]
        header: ChunkHeader,
        /// Indices into the original event vector. Preserves order.
        event_indices: Vec<usize>,
        /// Paired tool calls that live inside this chunk.
        tool_executions: Vec<LinkedTool>,
    },
    /// Slash-command stdout.
    #[serde(rename = "system")]
    System {
        #[serde(flatten)]
        header: ChunkHeader,
        event_index: usize,
    },
    /// Compaction boundary.
    #[serde(rename = "compact")]
    Compact {
        #[serde(flatten)]
        header: ChunkHeader,
        event_index: usize,
    },
}

impl SessionChunk {
    pub fn header(&self) -> &ChunkHeader {
        match self {
            SessionChunk::User { header, .. }
            | SessionChunk::Ai { header, .. }
            | SessionChunk::System { header, .. }
            | SessionChunk::Compact { header, .. } => header,
        }
    }
}

/// Build chunks from a full event vector.
///
/// Linker runs once over `events`; each AI chunk picks up the linked
/// tools whose `call_index` falls inside its event span. This keeps the
/// chunk output self-contained for rendering.
pub fn build_chunks(events: &[SessionEvent]) -> Vec<SessionChunk> {
    let linked = link_tools(events);
    let mut chunks: Vec<SessionChunk> = Vec::new();
    let mut current_ai: Option<AiInProgress> = None;
    let mut next_id: usize = 0;

    for (idx, ev) in events.iter().enumerate() {
        let cat = classify_event(ev);
        match cat {
            MessageCategory::HardNoise => continue,
            MessageCategory::Ai => {
                match &mut current_ai {
                    Some(ai) => ai.push(idx, ev),
                    None => {
                        let mut ai = AiInProgress::new(next_id);
                        ai.push(idx, ev);
                        current_ai = Some(ai);
                        next_id += 1;
                    }
                }
            }
            other => {
                if let Some(ai) = current_ai.take() {
                    chunks.push(ai.finish(&linked));
                }
                let header = ChunkHeader {
                    id: next_id,
                    start_ts: event_ts(ev),
                    end_ts: event_ts(ev),
                    metrics: metrics_for_single(ev),
                };
                next_id += 1;
                let chunk = match other {
                    MessageCategory::User => SessionChunk::User {
                        header,
                        event_index: idx,
                    },
                    MessageCategory::System => SessionChunk::System {
                        header,
                        event_index: idx,
                    },
                    MessageCategory::Compact => SessionChunk::Compact {
                        header,
                        event_index: idx,
                    },
                    MessageCategory::Ai | MessageCategory::HardNoise => unreachable!(),
                };
                chunks.push(chunk);
            }
        }
    }
    if let Some(ai) = current_ai.take() {
        chunks.push(ai.finish(&linked));
    }
    chunks
}

// ---------------------------------------------------------------------------
// AI chunk accumulator
// ---------------------------------------------------------------------------

struct AiInProgress {
    id: usize,
    event_indices: Vec<usize>,
    start_ts: Option<DateTime<Utc>>,
    end_ts: Option<DateTime<Utc>>,
    metrics: ChunkMetrics,
    /// UUIDs we've already charged `usage` for. One assistant JSONL
    /// line expands into N `AssistantText`/`ToolUse`/`Thinking`
    /// events, all carrying the same usage field — we only want to
    /// add it to the running total once per source message. Events
    /// without a UUID (rare) are always charged.
    usage_counted_uuids: HashSet<String>,
}

impl AiInProgress {
    fn new(id: usize) -> Self {
        Self {
            id,
            event_indices: Vec::new(),
            start_ts: None,
            end_ts: None,
            metrics: ChunkMetrics::default(),
            usage_counted_uuids: HashSet::new(),
        }
    }

    fn push(&mut self, idx: usize, ev: &SessionEvent) {
        self.event_indices.push(idx);
        self.metrics.message_count += 1;
        match ev {
            SessionEvent::AssistantText {
                usage: Some(u),
                uuid,
                ..
            } => {
                if should_charge_usage(uuid, &mut self.usage_counted_uuids) {
                    add_usage(&mut self.metrics.tokens, u);
                }
            }
            SessionEvent::AssistantThinking { .. } => {
                self.metrics.thinking_count += 1;
            }
            SessionEvent::AssistantToolUse { .. } => {
                self.metrics.tool_call_count += 1;
            }
            _ => {}
        }
        let ts = event_ts(ev);
        if let Some(t) = ts {
            if self.start_ts.is_none_or(|s| t < s) {
                self.start_ts = Some(t);
            }
            if self.end_ts.is_none_or(|e| t > e) {
                self.end_ts = Some(t);
            }
        }
    }

    fn finish(mut self, linked: &[LinkedTool]) -> SessionChunk {
        let low = self.event_indices.first().copied().unwrap_or(usize::MAX);
        let high = self.event_indices.last().copied().unwrap_or(0);
        let tools: Vec<LinkedTool> = linked
            .iter()
            .filter(|t| t.call_index >= low && t.call_index <= high)
            .cloned()
            .collect();
        self.metrics.duration_ms = match (self.start_ts, self.end_ts) {
            (Some(a), Some(b)) => (b - a).num_milliseconds(),
            _ => 0,
        };
        SessionChunk::Ai {
            header: ChunkHeader {
                id: self.id,
                start_ts: self.start_ts,
                end_ts: self.end_ts,
                metrics: self.metrics,
            },
            event_indices: self.event_indices,
            tool_executions: tools,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn event_ts(ev: &SessionEvent) -> Option<DateTime<Utc>> {
    match ev {
        SessionEvent::UserText { ts, .. }
        | SessionEvent::UserToolResult { ts, .. }
        | SessionEvent::AssistantText { ts, .. }
        | SessionEvent::AssistantToolUse { ts, .. }
        | SessionEvent::AssistantThinking { ts, .. }
        | SessionEvent::Summary { ts, .. }
        | SessionEvent::System { ts, .. }
        | SessionEvent::Attachment { ts, .. }
        | SessionEvent::FileHistorySnapshot { ts, .. }
        | SessionEvent::Other { ts, .. } => *ts,
        SessionEvent::Malformed { .. } => None,
    }
}

fn metrics_for_single(ev: &SessionEvent) -> ChunkMetrics {
    let mut m = ChunkMetrics {
        message_count: 1,
        ..ChunkMetrics::default()
    };
    // Only AssistantText carries usage, and single-event chunks are
    // non-AI categories anyway, so the usage sum here is essentially
    // cosmetic. Left in place for consistency with AiInProgress.
    if let SessionEvent::AssistantText { usage: Some(u), .. } = ev {
        add_usage(&mut m.tokens, u);
    }
    m
}

fn add_usage(acc: &mut TokenUsage, u: &TokenUsage) {
    acc.input += u.input;
    acc.output += u.output;
    acc.cache_creation += u.cache_creation;
    acc.cache_read += u.cache_read;
}

/// Return `true` when this `uuid` hasn't been charged yet, and record
/// it as charged. `None` UUIDs are always charged — they're rare
/// enough that a single fragment per missing-UUID turn won't matter
/// in practice, and we'd rather over-count than silently drop data.
pub(crate) fn should_charge_usage(
    uuid: &Option<String>,
    seen: &mut HashSet<String>,
) -> bool {
    match uuid {
        Some(u) => seen.insert(u.clone()),
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(s.parse::<DateTime<Utc>>().unwrap())
    }

    fn user(s: &str, t: Option<DateTime<Utc>>) -> SessionEvent {
        SessionEvent::UserText {
            ts: t,
            uuid: None,
            text: s.into(),
        }
    }

    fn assistant_text(s: &str, t: Option<DateTime<Utc>>, tokens: Option<TokenUsage>) -> SessionEvent {
        SessionEvent::AssistantText {
            ts: t,
            uuid: None,
            model: Some("claude-opus-4-7".into()),
            text: s.into(),
            usage: tokens,
            stop_reason: None,
        }
    }

    fn tool_use(id: &str, name: &str, t: Option<DateTime<Utc>>) -> SessionEvent {
        SessionEvent::AssistantToolUse {
            ts: t,
            uuid: None,
            model: None,
            tool_name: name.into(),
            tool_use_id: id.into(),
            input_preview: "{}".into(),
        }
    }

    fn tool_result(id: &str, body: &str, t: Option<DateTime<Utc>>) -> SessionEvent {
        SessionEvent::UserToolResult {
            ts: t,
            uuid: None,
            tool_use_id: id.into(),
            content: body.into(),
            is_error: false,
        }
    }

    #[test]
    fn simple_turn_becomes_user_then_ai_chunks() {
        let events = vec![
            user("hi", ts("2026-04-10T10:00:00Z")),
            assistant_text("hello", ts("2026-04-10T10:00:01Z"), None),
        ];
        let chunks = build_chunks(&events);
        assert_eq!(chunks.len(), 2);
        match &chunks[0] {
            SessionChunk::User { event_index, header, .. } => {
                assert_eq!(*event_index, 0);
                assert_eq!(header.id, 0);
            }
            other => panic!("wanted User, got {other:?}"),
        }
        match &chunks[1] {
            SessionChunk::Ai { event_indices, tool_executions, header, .. } => {
                assert_eq!(event_indices, &vec![1]);
                assert!(tool_executions.is_empty());
                assert_eq!(header.id, 1);
            }
            other => panic!("wanted Ai, got {other:?}"),
        }
    }

    #[test]
    fn consecutive_ai_events_coalesce() {
        let events = vec![
            user("hi", ts("2026-04-10T10:00:00Z")),
            assistant_text(
                "first",
                ts("2026-04-10T10:00:01Z"),
                Some(TokenUsage {
                    input: 100,
                    output: 10,
                    ..TokenUsage::default()
                }),
            ),
            SessionEvent::AssistantThinking {
                ts: ts("2026-04-10T10:00:02Z"),
                uuid: None,
                text: "thinking".into(),
            },
            assistant_text(
                "second",
                ts("2026-04-10T10:00:03Z"),
                Some(TokenUsage {
                    output: 20,
                    ..TokenUsage::default()
                }),
            ),
        ];
        let chunks = build_chunks(&events);
        assert_eq!(chunks.len(), 2);
        if let SessionChunk::Ai { event_indices, header, .. } = &chunks[1] {
            assert_eq!(event_indices, &vec![1, 2, 3]);
            assert_eq!(header.metrics.message_count, 3);
            assert_eq!(header.metrics.thinking_count, 1);
            assert_eq!(header.metrics.tokens.input, 100);
            assert_eq!(header.metrics.tokens.output, 30);
            assert_eq!(header.metrics.duration_ms, 2000);
        } else {
            panic!("expected Ai chunk");
        }
    }

    #[test]
    fn hard_noise_events_are_filtered_but_chunk_continues() {
        let events = vec![
            user("hi", None),
            SessionEvent::FileHistorySnapshot {
                ts: None,
                uuid: None,
                file_count: 1,
            },
            assistant_text("hello", None, None),
        ];
        let chunks = build_chunks(&events);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(chunks[0], SessionChunk::User { .. }));
        if let SessionChunk::Ai { event_indices, .. } = &chunks[1] {
            assert_eq!(event_indices, &vec![2]);
        } else {
            panic!("expected Ai chunk");
        }
    }

    #[test]
    fn system_command_stdout_is_its_own_chunk() {
        let events = vec![
            user("hi", None),
            user("<local-command-stdout>fork sha</local-command-stdout>", None),
            assistant_text("k", None, None),
        ];
        let chunks = build_chunks(&events);
        assert_eq!(chunks.len(), 3);
        assert!(matches!(chunks[0], SessionChunk::User { .. }));
        assert!(matches!(chunks[1], SessionChunk::System { .. }));
        assert!(matches!(chunks[2], SessionChunk::Ai { .. }));
    }

    #[test]
    fn compact_boundary_splits_the_stream() {
        let events = vec![
            user("hi", None),
            assistant_text("hello", None, None),
            SessionEvent::Summary {
                ts: None,
                uuid: None,
                text: "compacted".into(),
            },
            user("second", None),
            assistant_text("there", None, None),
        ];
        let chunks = build_chunks(&events);
        assert_eq!(chunks.len(), 5);
        assert!(matches!(chunks[2], SessionChunk::Compact { .. }));
        assert!(matches!(chunks[3], SessionChunk::User { .. }));
        assert!(matches!(chunks[4], SessionChunk::Ai { .. }));
    }

    #[test]
    fn tool_executions_attached_to_surrounding_ai_chunk() {
        let events = vec![
            user("hi", None),
            tool_use("a", "Read", None),
            tool_result("a", "data", None),
            assistant_text("done", None, None),
        ];
        let chunks = build_chunks(&events);
        if let SessionChunk::Ai {
            tool_executions, ..
        } = &chunks[1]
        {
            assert_eq!(tool_executions.len(), 1);
            assert_eq!(tool_executions[0].tool_use_id, "a");
            assert!(tool_executions[0].result_content.is_some());
        } else {
            panic!("expected Ai chunk");
        }
    }

    #[test]
    fn session_starting_with_ai_still_makes_a_chunk() {
        // Adopted transcript that begins mid-flight.
        let events = vec![assistant_text("no user yet", None, None)];
        let chunks = build_chunks(&events);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(chunks[0], SessionChunk::Ai { .. }));
    }

    #[test]
    fn empty_transcript_yields_no_chunks() {
        assert!(build_chunks(&[]).is_empty());
    }

    #[test]
    fn chunk_ids_are_dense_and_sequential() {
        let events = vec![
            user("a", None),
            assistant_text("1", None, None),
            user("b", None),
            assistant_text("2", None, None),
        ];
        let chunks = build_chunks(&events);
        let ids: Vec<usize> = chunks.iter().map(|c| c.header().id).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn multi_fragment_assistant_message_counts_usage_once() {
        // A single JSONL line produces two AssistantText events that
        // share the same uuid and usage. The aggregated chunk should
        // NOT double-count tokens.
        let events = vec![
            user("hi", None),
            SessionEvent::AssistantText {
                ts: None,
                uuid: Some("asst-1".into()),
                model: None,
                text: "part one".into(),
                usage: Some(TokenUsage {
                    input: 100,
                    output: 50,
                    ..TokenUsage::default()
                }),
                stop_reason: None,
            },
            SessionEvent::AssistantText {
                ts: None,
                uuid: Some("asst-1".into()),
                model: None,
                text: "part two".into(),
                usage: Some(TokenUsage {
                    input: 100,
                    output: 50,
                    ..TokenUsage::default()
                }),
                stop_reason: None,
            },
        ];
        let chunks = build_chunks(&events);
        if let SessionChunk::Ai { header, .. } = &chunks[1] {
            assert_eq!(header.metrics.tokens.input, 100);
            assert_eq!(header.metrics.tokens.output, 50);
        } else {
            panic!("expected Ai chunk");
        }
    }

    #[test]
    fn missing_uuid_still_charges_usage() {
        // Defensive: events without a uuid (rare) should still be
        // aggregated so we don't lose data entirely.
        let events = vec![
            user("hi", None),
            SessionEvent::AssistantText {
                ts: None,
                uuid: None,
                model: None,
                text: "x".into(),
                usage: Some(TokenUsage {
                    output: 7,
                    ..TokenUsage::default()
                }),
                stop_reason: None,
            },
            SessionEvent::AssistantText {
                ts: None,
                uuid: None,
                model: None,
                text: "y".into(),
                usage: Some(TokenUsage {
                    output: 11,
                    ..TokenUsage::default()
                }),
                stop_reason: None,
            },
        ];
        let chunks = build_chunks(&events);
        if let SessionChunk::Ai { header, .. } = &chunks[1] {
            // Each fragment counted because we can't dedupe without a key.
            assert_eq!(header.metrics.tokens.output, 18);
        } else {
            panic!("expected Ai chunk");
        }
    }

    #[test]
    fn serde_roundtrip_preserves_shape() {
        let events = vec![user("hi", None), assistant_text("hello", None, None)];
        let chunks = build_chunks(&events);
        let s = serde_json::to_string(&chunks).unwrap();
        let back: Vec<SessionChunk> = serde_json::from_str(&s).unwrap();
        assert_eq!(back.len(), chunks.len());
        assert!(matches!(back[0], SessionChunk::User { .. }));
        assert!(matches!(back[1], SessionChunk::Ai { .. }));
    }
}

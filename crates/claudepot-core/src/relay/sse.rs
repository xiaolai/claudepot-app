//! A bounded, incremental usage tap beside the unchanged forward.
//!
//! The forward handler never re-encodes a response; it hands every chunk
//! it forwards to this tap as well, and reads the result when the stream
//! ends. The tap is a reader, not a filter: what it fails to parse can
//! only make the ledger less precise, never change what the client sees.
//!
//! What it extracts, per Anthropic's streaming contract:
//!
//! - `message_start` → the model that answered and the opening usage;
//! - `message_delta` → usage, **cumulative** (a later value replaces an
//!   earlier one; counters never fall, so a provider that sends a `0`
//!   placeholder there does not erase a real count);
//! - `message_stop` → the stream completed;
//! - `error` → an in-stream failure after an HTTP 200.
//!
//! EOF with no `message_stop` is a truncated stream (`Partial`). A JSON
//! (non-streaming) response is parsed whole, up to a cap.
//!
//! ## Bounds
//!
//! Only the four event types above are accumulated; content deltas,
//! which are most of the bytes, are skipped as soon as their `event:`
//! line names them. A data line over [`MAX_EVENT`] is dropped and the
//! event it belongs to is ignored — parsing resumes at the next event.
//! A JSON body over [`MAX_JSON`] yields no usage.

use super::upstream::{OutcomeKind, Usage};

/// Largest event payload the tap holds. `message_start` carries the
/// message envelope with empty content; a few KiB in practice.
pub const MAX_EVENT: usize = 64 * 1024;
/// Largest non-streaming body the tap parses for usage.
pub const MAX_JSON: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Sse,
    Json,
    /// Not inference (`count_tokens`), or a content type with no usage.
    Off,
}

impl Mode {
    /// Choose from the response's `content-type`.
    pub fn for_content_type(content_type: Option<&str>) -> Self {
        let ct = content_type
            .and_then(|v| v.split(';').next())
            .map(|v| v.trim().to_ascii_lowercase());
        match ct.as_deref() {
            Some("text/event-stream") => Mode::Sse,
            Some("application/json") => Mode::Json,
            _ => Mode::Off,
        }
    }
}

/// What the tap learned. `error` is an identifier (`overloaded_error`),
/// never upstream prose.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tapped {
    pub usage: Option<Usage>,
    pub model: Option<String>,
    pub error: Option<String>,
    pub stopped: bool,
}

impl Tapped {
    /// The outcome of a response that returned a 2xx and reached EOF.
    pub fn success_kind(&self, mode: Mode) -> OutcomeKind {
        if self.error.is_some() {
            OutcomeKind::Failed
        } else if mode == Mode::Sse && !self.stopped {
            OutcomeKind::Partial
        } else {
            OutcomeKind::Complete
        }
    }
}

/// The events worth holding a payload for.
const WANTED: &[&str] = &["message_start", "message_delta", "message_stop", "error"];

pub struct UsageTap {
    mode: Mode,
    out: Tapped,
    // SSE state.
    line: Vec<u8>,
    line_over: bool,
    /// The previous chunk ended in `\r`; a leading `\n` now belongs to it.
    pending_cr: bool,
    event: Option<String>,
    data: Vec<u8>,
    data_over: bool,
    has_data: bool,
    // JSON state.
    json: Vec<u8>,
    json_over: bool,
}

impl UsageTap {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            out: Tapped::default(),
            line: Vec::new(),
            line_over: false,
            pending_cr: false,
            event: None,
            data: Vec::new(),
            data_over: false,
            has_data: false,
            json: Vec::new(),
            json_over: false,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        match self.mode {
            Mode::Off => {}
            Mode::Json => {
                if self.json_over {
                    return;
                }
                if self.json.len() + chunk.len() > MAX_JSON {
                    self.json_over = true;
                    self.json = Vec::new();
                } else {
                    self.json.extend_from_slice(chunk);
                }
            }
            Mode::Sse => self.feed_sse(chunk),
        }
    }

    /// End of stream. A final event with no blank line after it is
    /// dispatched, as the SSE spec's EOF rule does not — but a stream cut
    /// mid-event must not lose a `message_delta` that fully arrived.
    pub fn finish(mut self) -> Tapped {
        match self.mode {
            Mode::Off => {}
            Mode::Json => {
                if !self.json_over {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&self.json) {
                        self.apply_json(&v);
                    }
                }
            }
            Mode::Sse => {
                if !self.line.is_empty() || self.line_over {
                    self.end_line();
                }
                self.dispatch();
            }
        }
        self.out
    }

    fn feed_sse(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        if self.pending_cr {
            self.pending_cr = false;
            if let Some(b'\n') = rest.first() {
                rest = &rest[1..];
            }
        }
        while let Some(i) = rest.iter().position(|b| *b == b'\n' || *b == b'\r') {
            self.push_line_bytes(&rest[..i]);
            self.end_line();
            if rest[i] == b'\r' {
                match rest.get(i + 1) {
                    Some(b'\n') => rest = &rest[i + 2..],
                    Some(_) => rest = &rest[i + 1..],
                    None => {
                        self.pending_cr = true;
                        rest = &[];
                    }
                }
            } else {
                rest = &rest[i + 1..];
            }
        }
        self.push_line_bytes(rest);
    }

    fn push_line_bytes(&mut self, bytes: &[u8]) {
        if self.line_over || bytes.is_empty() {
            return;
        }
        if self.line.len() + bytes.len() > MAX_EVENT {
            self.line_over = true;
            self.line = Vec::new();
        } else {
            self.line.extend_from_slice(bytes);
        }
    }

    fn end_line(&mut self) {
        let over = std::mem::take(&mut self.line_over);
        let line = std::mem::take(&mut self.line);
        if over {
            // Whatever field this was, the event is no longer whole.
            self.data_over = true;
            return;
        }
        if line.is_empty() {
            self.dispatch();
            return;
        }
        if line[0] == b':' {
            return;
        }
        let (field, value) = match line.iter().position(|b| *b == b':') {
            Some(i) => {
                let v = &line[i + 1..];
                (&line[..i], v.strip_prefix(b" ").unwrap_or(v))
            }
            None => (&line[..], &b""[..]),
        };
        match field {
            b"event" => {
                self.event = std::str::from_utf8(value).ok().map(str::to_owned);
            }
            b"data" => {
                if self.data_over || !self.wanted() {
                    return;
                }
                if self.data.len() + value.len() + 1 > MAX_EVENT {
                    self.data_over = true;
                    self.data = Vec::new();
                    return;
                }
                if self.has_data {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value);
                self.has_data = true;
            }
            _ => {}
        }
    }

    /// An event whose name is known and uninteresting is not buffered.
    /// An unnamed one is, because its `type` field is what names it.
    fn wanted(&self) -> bool {
        self.event.as_deref().is_none_or(|e| WANTED.contains(&e))
    }

    fn dispatch(&mut self) {
        let over = std::mem::take(&mut self.data_over);
        let has = std::mem::take(&mut self.has_data);
        let data = std::mem::take(&mut self.data);
        let event = self.event.take();
        if over || !has {
            return;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&data) else {
            return;
        };
        let kind = event
            .as_deref()
            .or_else(|| v.get("type").and_then(|t| t.as_str()))
            .unwrap_or("");
        match kind {
            "message_start" => {
                if let Some(m) = v.get("message") {
                    self.take_model(m);
                    if let Some(u) = m.get("usage") {
                        self.merge_usage(u);
                    }
                }
            }
            "message_delta" => {
                if let Some(u) = v.get("usage") {
                    self.merge_usage(u);
                }
            }
            "message_stop" => self.out.stopped = true,
            "error" => self.out.error = Some(error_type(&v)),
            _ => {}
        }
    }

    fn apply_json(&mut self, v: &serde_json::Value) {
        if v.get("type").and_then(|t| t.as_str()) == Some("error") {
            self.out.error = Some(error_type(v));
            return;
        }
        self.take_model(v);
        if let Some(u) = v.get("usage") {
            self.merge_usage(u);
        }
        self.out.stopped = true;
    }

    fn take_model(&mut self, m: &serde_json::Value) {
        if let Some(model) = m.get("model").and_then(|x| x.as_str()) {
            if let Some(id) = identifier(model, 128) {
                self.out.model = Some(id);
            }
        }
    }

    fn merge_usage(&mut self, u: &serde_json::Value) {
        let into = self.out.usage.get_or_insert_with(Usage::default);
        let set = |field: &str, slot: &mut u64| {
            if let Some(n) = u.get(field).and_then(|x| x.as_u64()) {
                *slot = (*slot).max(n);
            }
        };
        set("input_tokens", &mut into.input_tokens);
        set("output_tokens", &mut into.output_tokens);
        set(
            "cache_creation_input_tokens",
            &mut into.cache_creation_input_tokens,
        );
        set("cache_read_input_tokens", &mut into.cache_read_input_tokens);
    }
}

/// Anthropic's documented error `type`s — the only values the ledger
/// records verbatim. An allowlist, not a shape check: a credential is
/// identifier-shaped too, so "looks like an identifier" would let an
/// upstream that echoes a key write it into `relay-usage.jsonl`.
const ERROR_TYPES: &[&str] = &[
    "invalid_request_error",
    "authentication_error",
    "billing_error",
    "permission_error",
    "not_found_error",
    "request_too_large",
    "rate_limit_error",
    "api_error",
    "timeout_error",
    "overloaded_error",
];

/// The error's `type`, if it is one of [`ERROR_TYPES`]; `other`
/// otherwise. Upstream text never reaches the ledger.
fn error_type(v: &serde_json::Value) -> String {
    v.get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())
        .and_then(|t| ERROR_TYPES.iter().find(|k| **k == t))
        .map_or_else(|| "other".to_owned(), |k| (*k).to_owned())
}

/// A model id short and plain enough to record — what the ledger accepts
/// for a client-supplied `model` as well as an upstream one.
pub(crate) fn model_id(s: &str) -> Option<String> {
    identifier(s, 128)
}

/// `s` if it is a short token of `[A-Za-z0-9._:/@-]` or `[]` (CC's `[1m]`
/// suffix); model ids are, and anything else is not recorded.
fn identifier(s: &str, max: usize) -> Option<String> {
    let ok = !s.is_empty()
        && s.len() <= max
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/@-[]".contains(c));
    ok.then(|| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic stream: start, content, a cumulative delta, stop.
    const STREAM: &str = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":25,\"cache_creation_input_tokens\":7,\"cache_read_input_tokens\":100,\"output_tokens\":1}}}\n",
        "\n",
        ": keep-alive comment\n",
        "event: ping\n",
        "data: {\"type\":\"ping\"}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n",
        "\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":15}}\n",
        "\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"input_tokens\":0,\"output_tokens\":42}}\n",
        "\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n",
        "\n",
    );

    fn expected() -> Tapped {
        Tapped {
            usage: Some(Usage {
                input_tokens: 25,
                output_tokens: 42,
                cache_creation_input_tokens: 7,
                cache_read_input_tokens: 100,
            }),
            model: Some("claude-sonnet-4-5".into()),
            error: None,
            stopped: true,
        }
    }

    fn tap_chunks(chunks: &[&[u8]]) -> Tapped {
        let mut t = UsageTap::new(Mode::Sse);
        for c in chunks {
            t.feed(c);
        }
        t.finish()
    }

    #[test]
    fn a_whole_stream_yields_model_cumulative_usage_and_completion() {
        let got = tap_chunks(&[STREAM.as_bytes()]);
        assert_eq!(got, expected());
        assert_eq!(got.success_kind(Mode::Sse), OutcomeKind::Complete);
    }

    #[test]
    fn every_two_way_byte_split_gives_the_same_result() {
        let b = STREAM.as_bytes();
        for i in 0..=b.len() {
            assert_eq!(tap_chunks(&[&b[..i], &b[i..]]), expected(), "split at {i}");
        }
    }

    #[test]
    fn byte_at_a_time_gives_the_same_result() {
        let chunks: Vec<&[u8]> = STREAM.as_bytes().chunks(1).collect();
        assert_eq!(tap_chunks(&chunks), expected());
    }

    #[test]
    fn crlf_and_bare_cr_line_endings_split_anywhere() {
        for ending in ["\r\n", "\r"] {
            let s = STREAM.replace('\n', ending);
            let b = s.as_bytes();
            for i in 0..=b.len() {
                assert_eq!(
                    tap_chunks(&[&b[..i], &b[i..]]),
                    expected(),
                    "{ending:?} split at {i}"
                );
            }
        }
    }

    #[test]
    fn an_error_event_after_a_200_is_a_failure() {
        let s = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-opus-4-1\",\"usage\":{\"input_tokens\":9}}}\n\n",
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded, secret prose\"}}\n\n",
        );
        let got = tap_chunks(&[s.as_bytes()]);
        assert_eq!(got.error.as_deref(), Some("overloaded_error"));
        assert_eq!(got.usage.unwrap().input_tokens, 9);
        assert_eq!(got.success_kind(Mode::Sse), OutcomeKind::Failed);
    }

    #[test]
    fn an_error_type_that_is_not_an_identifier_is_recorded_as_other() {
        let s = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"bad type with spaces\"}}\n\n";
        assert_eq!(tap_chunks(&[s.as_bytes()]).error.as_deref(), Some("other"));
    }

    #[test]
    fn an_identifier_shaped_error_type_outside_the_allowlist_is_recorded_as_other() {
        // Identifier-shaped is not enough: a credential is identifier-shaped.
        for t in ["sk-ant-api03-AbCdEf0123456789", "made_up_error"] {
            let s = format!(
                "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"{t}\"}}}}\n\n"
            );
            assert_eq!(
                tap_chunks(&[s.as_bytes()]).error.as_deref(),
                Some("other"),
                "{t}"
            );
            let j = format!("{{\"type\":\"error\",\"error\":{{\"type\":\"{t}\"}}}}");
            let mut tap = UsageTap::new(Mode::Json);
            tap.feed(j.as_bytes());
            assert_eq!(tap.finish().error.as_deref(), Some("other"), "{t}");
        }
        for t in ERROR_TYPES {
            let s = format!(
                "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"{t}\"}}}}\n\n"
            );
            assert_eq!(tap_chunks(&[s.as_bytes()]).error.as_deref(), Some(*t));
        }
    }

    #[test]
    fn eof_without_message_stop_is_partial_but_keeps_what_arrived() {
        let cut = STREAM.find("event: message_stop").unwrap();
        let got = tap_chunks(&[&STREAM.as_bytes()[..cut]]);
        assert!(!got.stopped);
        assert_eq!(got.usage, expected().usage);
        assert_eq!(got.success_kind(Mode::Sse), OutcomeKind::Partial);
    }

    #[test]
    fn a_final_event_without_its_blank_line_still_counts() {
        let trimmed = STREAM.trim_end_matches('\n');
        assert_eq!(tap_chunks(&[trimmed.as_bytes()]), expected());
    }

    #[test]
    fn a_stream_cut_mid_event_keeps_earlier_usage() {
        let cut = STREAM.rfind("\"output_tokens\":42").unwrap();
        let got = tap_chunks(&[&STREAM.as_bytes()[..cut]]);
        assert_eq!(got.usage.unwrap().output_tokens, 15);
        assert_eq!(got.success_kind(Mode::Sse), OutcomeKind::Partial);
    }

    #[test]
    fn unnamed_events_are_typed_by_their_payload() {
        let s = STREAM
            .lines()
            .filter(|l| !l.starts_with("event:"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(tap_chunks(&[s.as_bytes()]), expected());
    }

    #[test]
    fn an_oversized_event_is_skipped_and_parsing_resumes() {
        let huge = format!(
            "event: message_delta\ndata: {{\"usage\":{{\"output_tokens\":999}},\"pad\":\"{}\"}}\n\n",
            "x".repeat(MAX_EVENT + 10)
        );
        let s = format!("{huge}{STREAM}");
        let got = tap_chunks(&[s.as_bytes()]);
        assert_eq!(got, expected(), "the oversized event's 999 never lands");
    }

    #[test]
    fn content_deltas_are_never_buffered() {
        let mut t = UsageTap::new(Mode::Sse);
        t.feed(b"event: content_block_delta\ndata: ");
        t.feed("y".repeat(MAX_EVENT / 2).as_bytes());
        assert!(t.data.is_empty(), "a named content event holds no payload");
        assert!(t.line.len() <= MAX_EVENT);
    }

    #[test]
    fn memory_stays_bounded_on_an_endless_line() {
        let mut t = UsageTap::new(Mode::Sse);
        let chunk = vec![b'z'; 4096];
        for _ in 0..(4 * MAX_EVENT / 4096) {
            t.feed(&chunk);
        }
        assert!(t.line.len() <= MAX_EVENT && t.data.len() <= MAX_EVENT);
        t.feed(b"\n\n");
        t.feed(STREAM.as_bytes());
        assert_eq!(t.finish(), expected());
    }

    #[test]
    fn a_json_response_is_parsed_whole() {
        let body = br#"{"id":"msg_1","type":"message","model":"claude-haiku-4-5","usage":{"input_tokens":3,"output_tokens":4,"cache_read_input_tokens":2}}"#;
        let mut t = UsageTap::new(Mode::Json);
        let (a, b) = body.split_at(17);
        t.feed(a);
        t.feed(b);
        let got = t.finish();
        assert_eq!(got.model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(
            got.usage,
            Some(Usage {
                input_tokens: 3,
                output_tokens: 4,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 2
            })
        );
        assert_eq!(got.success_kind(Mode::Json), OutcomeKind::Complete);
    }

    #[test]
    fn a_json_error_body_is_a_failure_with_its_type() {
        let mut t = UsageTap::new(Mode::Json);
        t.feed(br#"{"type":"error","error":{"type":"rate_limit_error","message":"x"}}"#);
        let got = t.finish();
        assert_eq!(got.error.as_deref(), Some("rate_limit_error"));
        assert_eq!(got.success_kind(Mode::Json), OutcomeKind::Failed);
    }

    #[test]
    fn an_oversized_json_body_yields_no_usage() {
        let mut t = UsageTap::new(Mode::Json);
        t.feed(&vec![b' '; MAX_JSON]);
        t.feed(br#"{"usage":{"input_tokens":1}}"#);
        assert!(t.json.is_empty());
        assert_eq!(t.finish().usage, None);
    }

    #[test]
    fn the_mode_follows_the_content_type() {
        assert_eq!(
            Mode::for_content_type(Some("text/event-stream; charset=utf-8")),
            Mode::Sse
        );
        assert_eq!(Mode::for_content_type(Some("Application/JSON")), Mode::Json);
        assert_eq!(Mode::for_content_type(Some("text/html")), Mode::Off);
        assert_eq!(Mode::for_content_type(None), Mode::Off);
    }

    #[test]
    fn off_mode_learns_nothing() {
        let mut t = UsageTap::new(Mode::Off);
        t.feed(STREAM.as_bytes());
        assert_eq!(t.finish(), Tapped::default());
    }
}

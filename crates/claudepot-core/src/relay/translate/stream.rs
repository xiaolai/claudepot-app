//! OpenAI Responses stream → Anthropic Messages stream (and the
//! non-streaming collection of one).
//!
//! A port of clai's `clcd-stream.jq` and `clcd-collect.jq`, with the
//! reviewed deviations (`testdata/translate/README.md`):
//!
//! - **E4** — a function call still open when the response completes is
//!   an error event, not a `tool_use` with partial arguments. CC would
//!   run a tool with arguments the model never finished.
//! - **E2** — an `incomplete` response whose reason is `content_filter` is
//!   `stop_reason: "refusal"`, not `max_tokens`. Telling CC the turn ran
//!   out of tokens makes it continue a refused turn.
//! - **E6** — every error event carries one of [`Fault`]'s fixed
//!   messages. Upstream text never reaches the client.
//! - **Malformed is fatal.** A duplicate block start, arguments for a
//!   block that was never started, a delta to a closed block, invalid
//!   JSON or UTF-8, or a line over [`MAX_LINE`] ends the stream with an
//!   error event. jq would emit `index: null` or overwrite state and
//!   carry on.
//! - **One `message_start`.** A second `response.created` is ignored; a
//!   content event before any is preceded by a synthesized one; anything
//!   after the terminal event is ignored.
//!
//! - **A finished call is validated, streamed or collected.** Its item
//!   must not say it did not complete, its arguments must be one JSON
//!   object, and terminal arguments must agree with streamed ones
//!   ([`finished_arguments`]). jq trusted the deltas and dropped the
//!   terminal copy unread. Streamed arguments are kept, within
//!   [`MAX_ARGS_PER_CALL`] and [`MAX_ARGS_TOTAL`], to make that check.
//!
//! Unknown event types are ignored (E7), as in clai.

use std::collections::BTreeMap;

use axum::body::Bytes;
use serde_json::{json, Value};

use super::request::Names;
use super::{MAX_ARGS_PER_CALL, MAX_ARGS_TOTAL, MAX_BLOCKS, MAX_LINE};
use crate::relay::upstream::ProviderUsage;

/// Why a translated stream ended early. The message is the whole of what
/// the client is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The provider sent an error or a failed response.
    Upstream,
    /// The provider's stream could not be read as its protocol.
    Malformed,
    /// The provider's stream ended before its terminal event.
    Truncated,
    /// A line, block count or retained value went over its cap.
    Oversize,
}

impl Fault {
    pub fn message(self) -> &'static str {
        match self {
            Self::Upstream => "The upstream provider returned an error.",
            Self::Malformed => "The upstream provider's stream was malformed.",
            Self::Truncated => "The upstream provider's stream ended before completion.",
            Self::Oversize => "The upstream provider's stream exceeded the relay's limits.",
        }
    }

    /// The Anthropic error event for this fault.
    pub fn event(self) -> Event {
        Event::new(
            "error",
            json!({"error": {"type": "api_error", "message": self.message()}}),
        )
    }
}

/// One server-sent line that matters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Data(String),
    Done,
}

/// Splits a byte stream into SSE events' data, framed as the [SSE
/// spec](https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation)
/// frames it: CR before LF is dropped; a `data` field's value loses one
/// leading space if it has one (`data:x` and `data: x` are the same);
/// several `data` lines of one event are joined with `\n`; a blank line
/// ends the event. Comments and other fields are ignored.
///
/// clai's providers also send one event per `data:` line with no blank
/// line between, so an event is also complete as soon as its data is one
/// whole JSON value (or `[DONE]`). That never cuts a real multi-line event
/// short: a further non-blank `data` line after a whole JSON value would
/// make the joined data invalid JSON anyway.
///
/// A line, or one event's data, longer than [`MAX_LINE`] is a fault, so a
/// provider cannot grow a buffer without bound. A fault never takes back
/// what came before it: [`Self::feed_until_fault`] returns both.
#[derive(Debug, Default)]
pub struct SseLines {
    buf: Vec<u8>,
    /// The current event's data so far, `None` before its first `data`.
    data: Option<String>,
    /// The fault that stopped the stream. Every later call reports it.
    fault: Option<Fault>,
}

/// The value of a `data` field, or `None` for any other line.
fn data_field(line: &str) -> Option<&str> {
    let (field, value) = line.split_once(':').unwrap_or((line, ""));
    (field == "data").then(|| value.strip_prefix(' ').unwrap_or(value))
}

/// The event `data` makes, if it is not empty.
fn event_of(data: String) -> Option<Line> {
    if data.trim().is_empty() {
        None
    } else if data == "[DONE]" {
        Some(Line::Done)
    } else {
        Some(Line::Data(data))
    }
}

/// Whether `data` is already a whole event (see [`SseLines`]).
fn complete(data: &str) -> bool {
    data == "[DONE]" || serde_json::from_str::<serde::de::IgnoredAny>(data).is_ok()
}

impl SseLines {
    fn line(&mut self, raw: &[u8], out: &mut Vec<Line>) -> Result<(), Fault> {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let line = std::str::from_utf8(raw).map_err(|_| Fault::Malformed)?;
        if line.is_empty() {
            out.extend(self.data.take().and_then(event_of));
            return Ok(());
        }
        let Some(value) = data_field(line) else {
            return Ok(());
        };
        let data = match self.data.take() {
            None => value.to_owned(),
            Some(mut d) => {
                if d.len() + 1 + value.len() > MAX_LINE {
                    return Err(Fault::Oversize);
                }
                d.push('\n');
                d.push_str(value);
                d
            }
        };
        if complete(&data) {
            out.extend(event_of(data));
        } else {
            self.data = Some(data);
        }
        Ok(())
    }

    fn lines(&mut self, chunk: &[u8], out: &mut Vec<Line>) -> Result<(), Fault> {
        let mut rest = chunk;
        while let Some(nl) = rest.iter().position(|b| *b == b'\n') {
            if self.buf.len() + nl > MAX_LINE {
                return Err(Fault::Oversize);
            }
            self.buf.extend_from_slice(&rest[..nl]);
            rest = &rest[nl + 1..];
            let line = std::mem::take(&mut self.buf);
            self.line(&line, out)?;
        }
        if self.buf.len() + rest.len() > MAX_LINE {
            return Err(Fault::Oversize);
        }
        self.buf.extend_from_slice(rest);
        Ok(())
    }

    /// Every event `chunk` completes, and the fault that stopped it, if
    /// one did. The events before a fault are returned with it, so the
    /// same bytes give the same events however the network split them.
    pub fn feed_until_fault(&mut self, chunk: &[u8]) -> (Vec<Line>, Option<Fault>) {
        let mut out = Vec::new();
        if self.fault.is_none() {
            if let Err(f) = self.lines(chunk, &mut out) {
                self.fault = Some(f);
            }
        }
        (out, self.fault)
    }

    /// [`Self::feed_until_fault`] for a caller that takes one or the
    /// other: events before a fault come back `Ok`, and the fault is
    /// returned by the next call to this or [`Self::finish`].
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<Line>, Fault> {
        match self.feed_until_fault(chunk) {
            (lines, Some(f)) if lines.is_empty() => Err(f),
            (lines, _) => Ok(lines),
        }
    }

    /// The last event, if the stream did not end with a blank line.
    pub fn finish(&mut self) -> Result<Option<Line>, Fault> {
        if let Some(f) = self.fault {
            return Err(f);
        }
        let mut out = Vec::new();
        let tail = std::mem::take(&mut self.buf);
        if !tail.is_empty() {
            if let Err(f) = self.line(&tail, &mut out) {
                self.fault = Some(f);
                return Err(f);
            }
        }
        // A tail `data` line either completes the pending event or joins
        // it, so the tail and the pending data make at most one event.
        out.extend(self.data.take().and_then(event_of));
        Ok(out.pop())
    }
}

/// Parse a `data:` payload as a JSON object.
pub fn parse_event(data: &str) -> Result<Value, Fault> {
    match serde_json::from_str::<Value>(data) {
        Ok(v) if v.is_object() => Ok(v),
        _ => Err(Fault::Malformed),
    }
}

/// One Anthropic stream event.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub kind: &'static str,
    pub data: Value,
}

impl Event {
    fn new(kind: &'static str, mut body: Value) -> Self {
        if let Some(o) = body.as_object_mut() {
            o.insert("type".into(), Value::String(kind.into()));
        }
        Self { kind, data: body }
    }
}

/// `event: T\ndata: json\n\n` for each event.
pub fn encode(events: &[Event]) -> Bytes {
    let mut out = String::new();
    for e in events {
        out.push_str("event: ");
        out.push_str(e.kind);
        out.push_str("\ndata: ");
        out.push_str(&e.data.to_string());
        out.push_str("\n\n");
    }
    Bytes::from(out)
}

struct Block {
    index: usize,
    open: bool,
    tool: bool,
    /// The arguments streamed so far; `None` until a delta arrives.
    arguments: Option<String>,
}

/// A function call's final arguments once its item is done: the streamed
/// ones, else the item's own. Shared by the stream and the [`Collector`].
///
/// - an item `status` other than `completed` is [`Fault::Truncated`] — a
///   call the model did not finish is not a call (as E4);
/// - item arguments that disagree with streamed ones are malformed;
/// - no arguments at all is malformed — never a fabricated `{}`;
/// - arguments that are not one JSON object are malformed — a tool's
///   input is an object.
fn finished_arguments<'a>(item: &'a Value, streamed: Option<&'a str>) -> Result<&'a str, Fault> {
    match item.get("status") {
        None | Some(Value::Null) => {}
        Some(Value::String(st)) if st == "completed" => {}
        Some(_) => return Err(Fault::Truncated),
    }
    let terminal = match item.get("arguments") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) => Some(a.as_str()),
        Some(_) => return Err(Fault::Malformed),
    };
    let args = match (streamed, terminal) {
        (Some(st), Some(t)) if st != t => return Err(Fault::Malformed),
        (Some(st), _) => st,
        (None, Some(t)) => t,
        (None, None) => return Err(Fault::Malformed),
    };
    if !serde_json::from_str::<Value>(args).is_ok_and(|v| v.is_object()) {
        return Err(Fault::Malformed);
    }
    Ok(args)
}

/// The request's argument total after a call grows to `call_len` bytes by
/// `added`, or [`Fault::Oversize`] past [`MAX_ARGS_PER_CALL`] or
/// [`MAX_ARGS_TOTAL`]. Every path that keeps or forwards arguments —
/// streamed deltas, terminal-only arguments, collected calls — charges
/// through here.
fn charge_args(call_len: usize, added: usize, total: usize) -> Result<usize, Fault> {
    let total = total.saturating_add(added);
    if call_len > MAX_ARGS_PER_CALL || total > MAX_ARGS_TOTAL {
        return Err(Fault::Oversize);
    }
    Ok(total)
}

/// What a Responses event says about the end of the response — the one
/// classification the stream and the [`Collector`] share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Terminal {
    /// `response.completed` or `response.incomplete`: the answer stands.
    Ended,
    /// `response.failed` or `error`.
    Failed,
}

fn terminal_of(e: &Value) -> Option<Terminal> {
    match s(e, "/type") {
        Some("response.completed" | "response.incomplete") => Some(Terminal::Ended),
        Some("response.failed" | "error") => Some(Terminal::Failed),
        _ => None,
    }
}

/// The Anthropic stop reason for a terminal event. Incomplete when the
/// event type or the response's own status says so — streaming once
/// trusted only the first and collection only the second, so the same
/// answer could end `max_tokens` one way and `end_turn` the other.
fn stop_reason(event: &Value, tool: bool) -> &'static str {
    let response = event.get("response").unwrap_or(&Value::Null);
    let incomplete = s(event, "/type") == Some("response.incomplete")
        || s(response, "/status") == Some("incomplete");
    if incomplete {
        if filtered(response) {
            "refusal"
        } else {
            "max_tokens"
        }
    } else if tool {
        "tool_use"
    } else {
        "end_turn"
    }
}

/// Responses events in, Anthropic events out.
pub struct ResponsesToAnthropic {
    model: String,
    names: Names,
    blocks: BTreeMap<String, Block>,
    next: usize,
    tool: bool,
    started: bool,
    ended: bool,
    native: Option<ProviderUsage>,
    /// Streamed argument bytes kept across every call.
    args_total: usize,
}

fn s<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr).and_then(Value::as_str)
}

fn n(v: &Value, ptr: &str) -> u64 {
    v.pointer(ptr).and_then(Value::as_u64).unwrap_or(0)
}

/// jq's `tostring` for a content index: a number or string as written,
/// anything else as JSON.
fn part_key(id: &str, index: Option<&Value>) -> String {
    let idx = match index {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => "null".to_owned(),
    };
    format!("{id}:{idx}")
}

fn is_text_part(e: &Value) -> bool {
    matches!(s(e, "/part/type"), Some("output_text" | "refusal"))
}

/// Anthropic usage from a Responses `usage` object. Cached tokens are
/// split out of the input count, never below zero.
fn usage(u: &Value) -> Value {
    let input = n(u, "/input_tokens");
    let cached = n(u, "/input_tokens_details/cached_tokens");
    json!({
        "input_tokens": input.saturating_sub(cached),
        "cache_read_input_tokens": cached,
        "output_tokens": n(u, "/output_tokens"),
    })
}

/// The provider's own counters from a Responses `usage` object, kept in
/// the provider's semantics (input includes cached). `None` when the
/// provider sent no usage object.
pub fn provider_usage(u: &Value) -> Option<ProviderUsage> {
    if !u.is_object() {
        return None;
    }
    Some(ProviderUsage {
        input_tokens: n(u, "/input_tokens"),
        output_tokens: n(u, "/output_tokens"),
        cached_input_tokens: n(u, "/input_tokens_details/cached_tokens"),
        reasoning_tokens: u
            .pointer("/output_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64),
    })
}

/// The model a Responses event's `response` says answered, if it names
/// one that is an identifier. The ledger records it as the answering
/// model (docs/notes/relay.md, "Usage and cost").
fn reported_model(e: &Value) -> Option<String> {
    s(e, "/response/model").and_then(crate::relay::sse::model_id)
}

fn filtered(response: &Value) -> bool {
    s(response, "/incomplete_details/reason") == Some("content_filter")
}

impl ResponsesToAnthropic {
    pub fn new(model: &str, names: Names) -> Self {
        Self {
            model: model.to_owned(),
            names,
            blocks: BTreeMap::new(),
            next: 0,
            tool: false,
            started: false,
            ended: false,
            native: None,
            args_total: 0,
        }
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    /// The provider's counters from the terminal event, once it arrived.
    pub fn native_usage(&self) -> Option<ProviderUsage> {
        self.native
    }

    /// End the stream with `fault`, unless it already ended.
    pub fn fail(&mut self, fault: Fault) -> Vec<Event> {
        if self.ended {
            return Vec::new();
        }
        self.ended = true;
        vec![fault.event()]
    }

    /// The upstream ended. Without a terminal event that is truncation.
    pub fn finish(&mut self) -> Vec<Event> {
        self.fail(Fault::Truncated)
    }

    pub fn feed_line(&mut self, line: &Line) -> Vec<Event> {
        match line {
            Line::Done => Vec::new(),
            Line::Data(d) => match parse_event(d) {
                Ok(e) => self.feed_event(&e),
                Err(f) => self.fail(f),
            },
        }
    }

    pub fn feed_event(&mut self, e: &Value) -> Vec<Event> {
        if self.ended {
            return Vec::new();
        }
        let mut out = Vec::new();
        match self.step(e, &mut out) {
            Ok(()) => out,
            Err(f) => self.fail(f),
        }
    }

    fn ensure_started(&mut self, id: Option<&str>, out: &mut Vec<Event>) {
        if self.started {
            return;
        }
        self.started = true;
        out.push(Event::new(
            "message_start",
            json!({"message": {
                "id": id.unwrap_or("msg_clcd"),
                "type": "message",
                "role": "assistant",
                "model": self.model,
                "content": [],
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                },
            }}),
        ));
    }

    fn start(
        &mut self,
        key: String,
        block: Value,
        tool: bool,
        out: &mut Vec<Event>,
    ) -> Result<(), Fault> {
        if self.blocks.contains_key(&key) {
            return Err(Fault::Malformed);
        }
        if self.next >= MAX_BLOCKS {
            return Err(Fault::Oversize);
        }
        let index = self.next;
        self.next += 1;
        self.blocks.insert(
            key,
            Block {
                index,
                open: true,
                tool,
                arguments: None,
            },
        );
        out.push(Event::new(
            "content_block_start",
            json!({"index": index, "content_block": block}),
        ));
        Ok(())
    }

    fn open_block(&mut self, key: &str) -> Result<&mut Block, Fault> {
        match self.blocks.get_mut(key) {
            Some(b) if b.open => Ok(b),
            _ => Err(Fault::Malformed),
        }
    }

    fn delta(index: usize, delta: Value) -> Event {
        Event::new(
            "content_block_delta",
            json!({"index": index, "delta": delta}),
        )
    }

    fn stop(&mut self, key: &str, out: &mut Vec<Event>) {
        if let Some(b) = self.blocks.get_mut(key) {
            if b.open {
                b.open = false;
                out.push(Event::new("content_block_stop", json!({"index": b.index})));
            }
        }
    }

    fn step(&mut self, e: &Value, out: &mut Vec<Event>) -> Result<(), Fault> {
        let kind = s(e, "/type").unwrap_or("");
        let id = s(e, "/item_id")
            .or_else(|| s(e, "/item/id"))
            .unwrap_or("")
            .to_owned();
        let function_item = s(e, "/item/type") == Some("function_call");
        match kind {
            "response.created" => {
                if !self.started {
                    if let Some(m) = reported_model(e) {
                        self.model = m;
                    }
                }
                self.ensure_started(s(e, "/response/id"), out)
            }
            "response.output_item.added" if function_item => self.tool_added(e, id, out)?,
            "response.content_part.added" if is_text_part(e) => {
                self.ensure_started(None, out);
                let key = part_key(&id, e.get("content_index"));
                self.start(key, json!({"type": "text", "text": ""}), false, out)?;
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                self.text_delta(e, &id, out)?
            }
            "response.function_call_arguments.delta" => self.arguments_delta(e, &id, out)?,
            "response.output_item.done" if function_item => self.tool_done(e, &id, out)?,
            "response.content_part.done" if is_text_part(e) => {
                self.stop(&part_key(&id, e.get("content_index")), out);
            }
            _ => match terminal_of(e) {
                Some(Terminal::Ended) => self.terminal(e, out)?,
                Some(Terminal::Failed) => return Err(Fault::Upstream),
                None => {}
            },
        }
        Ok(())
    }

    fn tool_added(&mut self, e: &Value, id: String, out: &mut Vec<Event>) -> Result<(), Fault> {
        self.ensure_started(None, out);
        self.tool = true;
        let wire = s(e, "/item/name").unwrap_or("");
        let block = json!({
            "type": "tool_use",
            "id": e.pointer("/item/call_id").cloned().unwrap_or(Value::Null),
            "name": self.names.client(wire),
            "input": {},
        });
        self.start(id, block, true, out)
    }

    fn text_delta(&mut self, e: &Value, id: &str, out: &mut Vec<Event>) -> Result<(), Fault> {
        self.ensure_started(None, out);
        let text = e
            .get("delta")
            .and_then(Value::as_str)
            .ok_or(Fault::Malformed)?;
        let key = part_key(id, e.get("content_index"));
        if !self.blocks.contains_key(&key) {
            self.start(key.clone(), json!({"type": "text", "text": ""}), false, out)?;
        }
        let b = self.open_block(&key)?;
        if b.tool {
            return Err(Fault::Malformed);
        }
        let index = b.index;
        out.push(Self::delta(
            index,
            json!({"type": "text_delta", "text": text}),
        ));
        Ok(())
    }

    fn arguments_delta(&mut self, e: &Value, id: &str, out: &mut Vec<Event>) -> Result<(), Fault> {
        let part = e
            .get("delta")
            .and_then(Value::as_str)
            .ok_or(Fault::Malformed)?;
        let args_total = self.args_total;
        let b = self.open_block(id)?;
        if !b.tool {
            return Err(Fault::Malformed);
        }
        let args = b.arguments.get_or_insert_with(String::new);
        let total = charge_args(args.len() + part.len(), part.len(), args_total)?;
        args.push_str(part);
        let index = b.index;
        self.args_total = total;
        out.push(Self::delta(
            index,
            json!({"type": "input_json_delta", "partial_json": part}),
        ));
        Ok(())
    }

    /// The call's item is done: validate it, send the terminal arguments
    /// if none streamed, and close the block.
    fn tool_done(&mut self, e: &Value, id: &str, out: &mut Vec<Event>) -> Result<(), Fault> {
        // The field, not `open_block`, so `args_total` stays borrowable.
        let b = match self.blocks.get_mut(id) {
            Some(b) if b.open && b.tool => b,
            _ => return Err(Fault::Malformed),
        };
        let item = e.get("item").unwrap_or(&Value::Null);
        let args = finished_arguments(item, b.arguments.as_deref())?;
        if b.arguments.is_none() {
            // Terminal-only arguments are kept by no buffer here, but they
            // are forwarded whole, so they count against the same caps.
            self.args_total = charge_args(args.len(), args.len(), self.args_total)?;
            let delta = json!({"type": "input_json_delta", "partial_json": args});
            out.push(Self::delta(b.index, delta));
        }
        self.stop(id, out);
        Ok(())
    }

    fn terminal(&mut self, e: &Value, out: &mut Vec<Event>) -> Result<(), Fault> {
        // E4: a call whose arguments never finished is not a call.
        if self.blocks.values().any(|b| b.tool && b.open) {
            return Err(Fault::Truncated);
        }
        self.ensure_started(None, out);
        let keys: Vec<String> = self.blocks.keys().cloned().collect();
        for k in keys {
            self.stop(&k, out);
        }
        self.ended = true;
        let response = e.get("response").unwrap_or(&Value::Null);
        let reason = stop_reason(e, self.tool);
        let u = response.get("usage").unwrap_or(&Value::Null);
        self.native = provider_usage(u);
        out.push(Event::new(
            "message_delta",
            json!({
                "delta": {"stop_reason": reason, "stop_sequence": null},
                "usage": usage(u),
            }),
        ));
        out.push(Event::new("message_stop", json!({})));
        Ok(())
    }
}

/// [`Fault::Oversize`] if `content` already holds [`MAX_BLOCKS`] blocks —
/// the cap the stream enforces in `ResponsesToAnthropic::start`.
fn room_for_block(content: &[Value]) -> Result<(), Fault> {
    if content.len() >= MAX_BLOCKS {
        return Err(Fault::Oversize);
    }
    Ok(())
}

/// The non-streaming path: keep the first terminal event — anything after
/// it is ignored, as the stream ignores it — then build one Anthropic
/// message from its `response`.
#[derive(Default)]
pub struct Collector {
    last: Option<Value>,
    fault: Option<Fault>,
}

impl Collector {
    pub fn feed_event(&mut self, e: Value) {
        if self.done() {
            return;
        }
        match terminal_of(&e) {
            Some(Terminal::Ended) => self.last = Some(e),
            Some(Terminal::Failed) => self.fault = Some(Fault::Upstream),
            None => {}
        }
    }

    /// Stop with `fault`, unless the result is already decided — a fault
    /// after the terminal event no more changes it than it changes a stream.
    pub fn fail(&mut self, fault: Fault) {
        if !self.done() {
            self.fault = Some(fault);
        }
    }

    /// A terminal event or a fault arrived: nothing later can change the
    /// result, so the caller stops reading the provider.
    pub fn done(&self) -> bool {
        self.last.is_some() || self.fault.is_some()
    }

    /// The provider's counters from the terminal event, if any.
    pub fn native_usage(&self) -> Option<ProviderUsage> {
        self.last
            .as_ref()
            .and_then(|e| e.pointer("/response/usage"))
            .and_then(provider_usage)
    }

    pub fn finish(self, model: &str, names: &Names) -> Result<Value, Fault> {
        if let Some(f) = self.fault {
            return Err(f);
        }
        let event = self.last.ok_or(Fault::Truncated)?;
        let r = event.get("response").unwrap_or(&Value::Null);
        let output = r
            .get("output")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut content = Vec::new();
        let mut tool = false;
        let mut args_total = 0;
        for item in &output {
            match s(item, "/type") {
                Some("message") => {
                    for part in item
                        .get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if matches!(s(part, "/type"), Some("output_text" | "refusal")) {
                            room_for_block(&content)?;
                            let text = part
                                .get("text")
                                .filter(|t| !t.is_null())
                                .or_else(|| part.get("refusal"))
                                .cloned()
                                .unwrap_or(Value::Null);
                            content.push(json!({"type": "text", "text": text}));
                        }
                    }
                }
                Some("function_call") => {
                    tool = true;
                    room_for_block(&content)?;
                    let args = finished_arguments(item, None)?;
                    args_total = charge_args(args.len(), args.len(), args_total)?;
                    let input: Value = serde_json::from_str(args).map_err(|_| Fault::Malformed)?;
                    content.push(json!({
                        "type": "tool_use",
                        "id": item.get("call_id").cloned().unwrap_or(Value::Null),
                        "name": names.client(s(item, "/name").unwrap_or("")),
                        "input": input,
                    }));
                }
                _ => {}
            }
        }
        let reason = stop_reason(&event, tool);
        let model = reported_model(&event).unwrap_or_else(|| model.to_owned());
        let mut u = usage(r.get("usage").unwrap_or(&Value::Null));
        if let Some(o) = u.as_object_mut() {
            o.insert("cache_creation_input_tokens".into(), Value::from(0));
        }
        Ok(json!({
            "id": r.get("id").cloned().unwrap_or(Value::Null),
            "type": "message",
            "role": "assistant",
            "model": model,
            "stop_sequence": null,
            "stop_reason": reason,
            "content": content,
            "usage": u,
        }))
    }
}

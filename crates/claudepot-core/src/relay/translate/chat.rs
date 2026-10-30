//! OpenAI Responses ↔ chat completions, for providers that speak only
//! chat (Gemini's OpenAI-compatible endpoint, a local server).
//!
//! The relay translates in two hops — Anthropic → Responses → chat and
//! back — exactly as clai does (`chat-request.jq`, `chat-stream.jq`), so
//! the Responses translator is the only one that knows Anthropic.
//!
//! **Gemini replay metadata.** Gemini attaches a private
//! `extra_content` (a thought signature) to each tool call and rejects a
//! later request that replays the call without it. The normalizer hands
//! each one out as [`Out::Cache`]; the handler keeps it in
//! [`super::continuation::ContinuationStore`], scoped to one device, one
//! target and one model (reviewed E9), and [`to_chat`] asks for it back.
//! A miss is refused before anything is sent — never sent unsigned.
//!
//! Deviations from clai, each reviewed:
//!
//! - `finish_reason: "content_filter"` becomes an `incomplete` response
//!   with reason `content_filter`, which the Responses translator turns
//!   into `stop_reason: "refusal"` (E2). clai raised an error.
//! - After the finish reason, further content and tool fragments are
//!   ignored; usage still counts (`include_usage` sends it last).
//! - A `finish_reason` that is not one of `stop`, `length`, `tool_calls`,
//!   `content_filter` or `function_call` is malformed, not a completion.
//! - Fragment count, argument size, call id and name size, and metadata
//!   size are capped.
//! - Every failure is a [`Fault`], whose message is fixed.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::stream::{parse_event, Fault, Line};
use super::{
    MAX_ARGS_PER_CALL, MAX_ARGS_TOTAL, MAX_CALLS, MAX_CALL_FIELD, MAX_EXTRA_CONTENT,
    MAX_RETAINED_TEXT,
};

/// Why a Responses request cannot become a chat request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    /// A Gemini replay whose metadata the relay does not hold.
    #[error("Gemini tool replay metadata is missing; retain the private cache or start a new conversation")]
    MissingMetadata,
    #[error("{0}")]
    Unsupported(&'static str),
}

fn text_parts(content: &Value) -> Result<Vec<Value>, ChatError> {
    content
        .as_array()
        .ok_or(ChatError::Unsupported("Unsupported chat input part"))?
        .iter()
        .map(|p| match p.get("type").and_then(Value::as_str) {
            Some("input_text" | "output_text") => {
                Ok(json!({"type": "text", "text": p.get("text").cloned().unwrap_or(Value::Null)}))
            }
            Some("input_image") => Ok(json!({
                "type": "image_url",
                "image_url": {"url": p.get("image_url").cloned().unwrap_or(Value::Null)},
            })),
            _ => Err(ChatError::Unsupported("Unsupported chat input part")),
        })
        .collect()
}

/// Translate a Responses request (from [`super::request::to_responses`])
/// to chat completions. For Gemini, `lookup(call_id)` returns the stored
/// `extra_content` of each replayed call.
pub fn to_chat(
    req: &Value,
    gemini: bool,
    lookup: impl Fn(&str) -> Option<Value>,
) -> Result<Value, ChatError> {
    if req.get("parallel_tool_calls") == Some(&Value::Bool(false)) {
        return Err(ChatError::Unsupported(
            "Disabling parallel tools is unsupported by this chat adapter",
        ));
    }
    let mut messages: Vec<Value> = vec![json!({
        "role": "system",
        "content": req.get("instructions").cloned().unwrap_or(Value::Null),
    })];
    for item in req
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => messages.push(input_message(item)?),
            Some("function_call") => push_call(&mut messages, chat_call(item, gemini, &lookup)?),
            Some("function_call_output") => messages.push(json!({
                "role": "tool",
                "tool_call_id": item.get("call_id").cloned().unwrap_or(Value::Null),
                "content": item.get("output").cloned().unwrap_or(Value::Null),
            })),
            _ => return Err(ChatError::Unsupported("Unsupported chat input item")),
        }
    }
    let mut out = Map::new();
    out.insert(
        "model".into(),
        req.get("model").cloned().unwrap_or(Value::Null),
    );
    out.insert("messages".into(), Value::Array(messages));
    out.insert("stream".into(), Value::Bool(true));
    out.insert("stream_options".into(), json!({"include_usage": true}));
    out.insert("tools".into(), Value::Array(chat_tools(req)));
    options(req, gemini, &mut out)?;
    Ok(Value::Object(out))
}

/// A Responses `message` item as a chat message.
fn input_message(item: &Value) -> Result<Value, ChatError> {
    let role = match item.get("role").and_then(Value::as_str) {
        Some("developer") => "system",
        Some(r) => r,
        None => return Err(ChatError::Unsupported("Unsupported chat input item")),
    };
    let content = text_parts(item.get("content").unwrap_or(&Value::Null))?;
    Ok(json!({"role": role, "content": content}))
}

/// A Responses `function_call` item as a chat tool call, with Gemini's
/// replay metadata attached — or refused when the relay does not hold it.
fn chat_call(
    item: &Value,
    gemini: bool,
    lookup: &impl Fn(&str) -> Option<Value>,
) -> Result<Value, ChatError> {
    let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
    let mut call = json!({
        "id": call_id,
        "type": "function",
        "function": {
            "name": item.get("name").cloned().unwrap_or(Value::Null),
            "arguments": item.get("arguments").cloned().unwrap_or(Value::Null),
        },
    });
    if gemini {
        let extra = lookup(call_id).ok_or(ChatError::MissingMetadata)?;
        if let Some(o) = call.as_object_mut() {
            o.insert("extra_content".into(), extra);
        }
    }
    Ok(call)
}

/// Consecutive calls share one assistant message: append to the last
/// message when it is the assistant's, else start one.
fn push_call(messages: &mut Vec<Value>, call: Value) {
    let last_is_assistant = messages
        .last()
        .and_then(|m| m.get("role"))
        .and_then(Value::as_str)
        == Some("assistant");
    match messages.last_mut().and_then(Value::as_object_mut) {
        Some(last) if last_is_assistant => {
            let calls = last
                .entry("tool_calls")
                .or_insert_with(|| Value::Array(Vec::new()));
            match calls.as_array_mut() {
                Some(a) => a.push(call),
                None => *calls = Value::Array(vec![call]),
            }
        }
        _ => messages.push(json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [call],
        })),
    }
}

fn chat_tools(req: &Value) -> Vec<Value> {
    req.get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|t| {
            json!({"type": "function", "function": {
                "name": t.get("name").cloned().unwrap_or(Value::Null),
                "description": t.get("description").cloned().unwrap_or(Value::Null),
                "parameters": t.get("parameters").cloned().unwrap_or(Value::Null),
            }})
        })
        .collect()
}

/// `max_tokens`, `tool_choice` and, for Gemini, `reasoning_effort`.
fn options(req: &Value, gemini: bool, out: &mut Map<String, Value>) -> Result<(), ChatError> {
    out.insert(
        "max_tokens".into(),
        req.get("max_output_tokens")
            .filter(|m| !m.is_null())
            .cloned()
            .unwrap_or(Value::from(4096)),
    );
    match req.get("tool_choice") {
        None | Some(Value::Null) => {}
        Some(c @ Value::Object(_)) => {
            out.insert(
                "tool_choice".into(),
                json!({"type": "function", "function": {"name": c.get("name").cloned().unwrap_or(Value::Null)}}),
            );
        }
        Some(c) => {
            out.insert("tool_choice".into(), c.clone());
        }
    }
    if gemini {
        match req.pointer("/reasoning/effort").and_then(Value::as_str) {
            Some(e @ ("low" | "medium" | "high")) => {
                out.insert("reasoning_effort".into(), Value::String(e.into()));
            }
            _ => {
                return Err(ChatError::Unsupported(
                    "Gemini compatibility supports low, medium or high reasoning effort",
                ))
            }
        }
    }
    Ok(())
}

/// What the normalizer produces.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// A Responses stream event.
    Event(Value),
    /// A tool call's replay metadata, to be kept for the next turn.
    Cache {
        id: String,
        model: String,
        extra_content: Value,
    },
}

#[derive(Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
    extra_content: Map<String, Value>,
}

/// A finished call has an id, a name, and arguments that are one JSON
/// object.
fn finished_call(c: &Call) -> Result<(), Fault> {
    let args_ok = serde_json::from_str::<Value>(&c.arguments).is_ok_and(|v| v.is_object());
    if c.id.is_empty() || c.name.is_empty() || !args_ok {
        return Err(Fault::Malformed);
    }
    Ok(())
}

/// jq's `*=`: objects merge recursively, anything else replaces.
fn deep_merge(into: &mut Map<String, Value>, from: &Map<String, Value>) {
    for (k, v) in from {
        match (into.get_mut(k), v) {
            (Some(Value::Object(a)), Value::Object(b)) => deep_merge(a, b),
            _ => {
                into.insert(k.clone(), v.clone());
            }
        }
    }
}

/// A chat completions stream in, a Responses stream out.
pub struct ChatToResponses {
    model: String,
    /// Keep the reply text for the terminal event's `output`. Only the
    /// non-streaming path reads it; streaming leaves it off so a long
    /// reply is not held twice.
    retain_output: bool,
    started: bool,
    text: bool,
    text_content: String,
    calls: BTreeMap<u64, Call>,
    args_total: usize,
    finished: bool,
    finish: Option<String>,
    ended: bool,
    usage: Value,
    id: String,
    /// The model the provider says answered, once a chunk named one that
    /// is an identifier. Reported downstream as the answering model; the
    /// continuation key stays [`Self::model`], the one requested.
    reported_model: Option<String>,
}

fn response(kind: &str, mut body: Value) -> Out {
    if let Some(o) = body.as_object_mut() {
        o.insert("type".into(), Value::String(kind.into()));
    }
    Out::Event(body)
}

impl ChatToResponses {
    pub fn new(model: &str, retain_output: bool) -> Self {
        Self {
            model: model.to_owned(),
            retain_output,
            started: false,
            text: false,
            text_content: String::new(),
            calls: BTreeMap::new(),
            args_total: 0,
            finished: false,
            finish: None,
            ended: false,
            usage: Value::Null,
            id: "resp_chat".to_owned(),
            reported_model: None,
        }
    }

    /// Whether the provider sent a `usage` object. The synthesized
    /// Responses usage is zeros without one, and zeros are not counters.
    pub fn usage_reported(&self) -> bool {
        self.usage.is_object()
    }

    pub fn feed_line(&mut self, line: &Line) -> Result<Vec<Out>, Fault> {
        if self.ended {
            return Ok(Vec::new());
        }
        match line {
            Line::Done => self.complete(),
            Line::Data(d) => {
                let e = parse_event(d)?;
                self.feed_event(&e)
            }
        }
    }

    /// The upstream ended.
    pub fn finish(&mut self) -> Result<Vec<Out>, Fault> {
        if self.ended {
            return Ok(Vec::new());
        }
        self.complete()
    }

    fn complete(&mut self) -> Result<Vec<Out>, Fault> {
        if !self.finished {
            return Err(Fault::Truncated);
        }
        self.ended = true;
        let incomplete = matches!(self.finish.as_deref(), Some("length" | "content_filter"));
        let mut output = Vec::new();
        if self.retain_output {
            if self.text {
                output.push(json!({
                    "type": "message",
                    "content": [{"type": "output_text", "text": self.text_content}],
                }));
            }
            for c in self.calls.values() {
                output.push(json!({
                    "type": "function_call",
                    "call_id": c.id,
                    "name": c.name,
                    "arguments": c.arguments,
                }));
            }
        }
        let u = &self.usage;
        // Zeros when the provider sent no usage, as the reference does;
        // `usage_reported` is what keeps those out of the native counters.
        let usage = {
            let mut usage = json!({
                "input_tokens": u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                "output_tokens": u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0),
                "input_tokens_details": {
                    "cached_tokens": u.pointer("/prompt_tokens_details/cached_tokens")
                        .and_then(Value::as_u64).unwrap_or(0),
                },
            });
            if let (Some(r), Some(o)) = (
                u.pointer("/completion_tokens_details/reasoning_tokens")
                    .and_then(Value::as_u64),
                usage.as_object_mut(),
            ) {
                o.insert(
                    "output_tokens_details".into(),
                    json!({"reasoning_tokens": r}),
                );
            }
            usage
        };
        let mut body = json!({"response": {
            "id": self.id,
            "status": if incomplete { "incomplete" } else { "completed" },
            "output": output,
            "usage": usage,
        }});
        self.add_reported_model(&mut body);
        if self.finish.as_deref() == Some("content_filter") {
            if let Some(r) = body.get_mut("response").and_then(Value::as_object_mut) {
                r.insert(
                    "incomplete_details".into(),
                    json!({"reason": "content_filter"}),
                );
            }
        }
        let kind = if incomplete {
            "response.incomplete"
        } else {
            "response.completed"
        };
        Ok(vec![response(kind, body)])
    }

    /// Which call a fragment belongs to. Provider indexes can be sparse,
    /// so an indexless fragment never assumes the keys are `0..len`: a
    /// known id is its call, a new id takes the key after the highest,
    /// and a fragment with neither extends the sole call — by its own key
    /// — or is malformed.
    fn call_index(&self, call: &Value) -> Result<u64, Fault> {
        if let Some(i) = call.get("index") {
            return i.as_u64().ok_or(Fault::Malformed);
        }
        match call.get("id").and_then(Value::as_str) {
            Some(id) => match self.calls.iter().find(|(_, c)| c.id == id) {
                Some((k, _)) => Ok(*k),
                None => self
                    .calls
                    .keys()
                    .next_back()
                    .map_or(Some(0), |k| k.checked_add(1))
                    .ok_or(Fault::Malformed),
            },
            None => match (self.calls.keys().next(), self.calls.len()) {
                (Some(k), 1) => Ok(*k),
                _ => Err(Fault::Malformed),
            },
        }
    }

    fn merge(&mut self, frag: &Value) -> Result<(), Fault> {
        let index = self.call_index(frag)?;
        if !self.calls.contains_key(&index) && self.calls.len() >= MAX_CALLS {
            return Err(Fault::Oversize);
        }
        let id = frag.get("id").and_then(Value::as_str);
        let name = frag.pointer("/function/name").and_then(Value::as_str);
        if [id, name]
            .into_iter()
            .flatten()
            .any(|f| f.len() > MAX_CALL_FIELD)
        {
            return Err(Fault::Oversize);
        }
        let call = self.calls.entry(index).or_default();
        if let Some(id) = id {
            call.id = id.to_owned();
        }
        if let Some(name) = name {
            call.name = name.to_owned();
        }
        if let Some(args) = frag.pointer("/function/arguments").and_then(Value::as_str) {
            if call.arguments.len() + args.len() > MAX_ARGS_PER_CALL
                || self.args_total + args.len() > MAX_ARGS_TOTAL
            {
                return Err(Fault::Oversize);
            }
            call.arguments.push_str(args);
            self.args_total += args.len();
        }
        if let Some(extra) = frag.get("extra_content").and_then(Value::as_object) {
            deep_merge(&mut call.extra_content, extra);
            let size = serde_json::to_vec(&call.extra_content).map_or(usize::MAX, |v| v.len());
            if size > MAX_EXTRA_CONTENT {
                return Err(Fault::Oversize);
            }
        }
        Ok(())
    }

    /// One chat chunk: the stream's start, then its text, tool-call and
    /// terminal transitions, in that order.
    fn feed_event(&mut self, e: &Value) -> Result<Vec<Out>, Fault> {
        if e.get("error").is_some_and(|v| !v.is_null()) {
            return Err(Fault::Upstream);
        }
        if let Some(u) = e.get("usage").filter(|u| !u.is_null()) {
            self.usage = u.clone();
        }
        let mut out = Vec::new();
        if self.finished {
            return Ok(out);
        }
        self.start(e, &mut out);
        let choice = e.pointer("/choices/0").unwrap_or(&Value::Null);
        let delta = choice.get("delta").unwrap_or(&Value::Null);
        self.text_delta(delta.get("content"), &mut out)?;
        // A refusal is the model's answer, in its own field: dropping it
        // would turn a refused turn into an empty successful one.
        self.text_delta(delta.get("refusal"), &mut out)?;
        self.tool_deltas(delta.get("tool_calls"))?;
        if let Some(reason) = choice.get("finish_reason").filter(|r| !r.is_null()) {
            self.terminal(reason, &mut out)?;
        }
        Ok(out)
    }

    fn start(&mut self, e: &Value, out: &mut Vec<Out>) {
        if self.started {
            return;
        }
        self.started = true;
        if let Some(id) = e.get("id").and_then(Value::as_str) {
            self.id = id.to_owned();
        }
        self.reported_model = e
            .get("model")
            .and_then(Value::as_str)
            .and_then(crate::relay::sse::model_id);
        let mut body = json!({"response": {"id": self.id}});
        self.add_reported_model(&mut body);
        out.push(response("response.created", body));
    }

    /// Name the provider's answering model on a Responses `response`, as
    /// a Responses provider does, when a chunk named one.
    fn add_reported_model(&self, body: &mut Value) {
        if let (Some(m), Some(r)) = (
            &self.reported_model,
            body.get_mut("response").and_then(Value::as_object_mut),
        ) {
            r.insert("model".into(), Value::String(m.clone()));
        }
    }

    fn text_delta(&mut self, content: Option<&Value>, out: &mut Vec<Out>) -> Result<(), Fault> {
        let t = match content {
            None | Some(Value::Null) => return Ok(()),
            Some(Value::String(t)) if t.is_empty() => return Ok(()),
            Some(Value::String(t)) => t,
            Some(_) => return Err(Fault::Malformed),
        };
        if !self.text {
            self.text = true;
            out.push(response(
                "response.content_part.added",
                json!({"item_id": "chat_text", "content_index": 0,
                       "part": {"type": "output_text", "text": ""}}),
            ));
        }
        if self.retain_output {
            if self.text_content.len() + t.len() > MAX_RETAINED_TEXT {
                return Err(Fault::Oversize);
            }
            self.text_content.push_str(t);
        }
        out.push(response(
            "response.output_text.delta",
            json!({"item_id": "chat_text", "content_index": 0, "delta": t}),
        ));
        Ok(())
    }

    fn tool_deltas(&mut self, tool_calls: Option<&Value>) -> Result<(), Fault> {
        match tool_calls {
            None | Some(Value::Null) => Ok(()),
            Some(Value::Array(frags)) => frags.iter().try_for_each(|f| self.merge(f)),
            Some(_) => Err(Fault::Malformed),
        }
    }

    /// `finish_reason` arrived: close the text part, then emit each call,
    /// every one validated by [`finished_call`] before it is emitted.
    fn terminal(&mut self, reason: &Value, out: &mut Vec<Out>) -> Result<(), Fault> {
        // An unknown or non-string reason is not a completion: reporting
        // it as `response.completed` would turn a malformed answer into a
        // successful turn.
        let reason = match reason.as_str() {
            Some(r @ ("stop" | "length" | "tool_calls" | "content_filter" | "function_call")) => r,
            _ => return Err(Fault::Malformed),
        };
        self.finished = true;
        self.finish = Some(reason.to_owned());
        if self.text {
            out.push(response(
                "response.content_part.done",
                json!({"item_id": "chat_text", "content_index": 0,
                       "part": {"type": "output_text"}}),
            ));
        }
        for c in self.calls.values() {
            finished_call(c)?;
            out.push(Out::Cache {
                id: c.id.clone(),
                model: self.model.clone(),
                extra_content: Value::Object(c.extra_content.clone()),
            });
            let item = json!({"id": c.id, "type": "function_call",
                              "call_id": c.id, "name": c.name});
            out.push(response(
                "response.output_item.added",
                json!({"item": item}),
            ));
            let mut done = item;
            if let Some(o) = done.as_object_mut() {
                o.insert("arguments".into(), Value::String(c.arguments.clone()));
            }
            out.push(response("response.output_item.done", json!({"item": done})));
        }
        Ok(())
    }
}

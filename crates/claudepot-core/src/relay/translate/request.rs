//! Anthropic Messages request → OpenAI Responses request.
//!
//! A port of clai's `clcd` request program (`testdata/translate/filters/
//! clcd-request.jq`), checked against its captured outputs. Two things
//! differ, both on purpose:
//!
//! - **The model is not chosen here.** clai maps Claude role names
//!   through its own table; the relay maps through the route
//!   ([`crate::relay::passthrough::map_model`]) so a published route means
//!   the same thing whether it is passed through or translated. The caller
//!   passes the mapped model in [`RequestOpts::model`].
//! - **A malformed shape is refused, never coerced.** Where jq would
//!   iterate `null` or index a number and fail with its own diagnostic,
//!   this returns a [`RequestError`] naming what was unsupported.
//!
//! Every refusal becomes `content_unsupported` on the wire (a fixed enum,
//! plan §5.6). The message here is for tests and debug logs; it is built
//! only from the request the client sent, never from upstream text.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::Translation;

/// Why a request could not be translated. Shown only in debug logs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RequestError(pub String);

fn err<T>(msg: impl Into<String>) -> Result<T, RequestError> {
    Err(RequestError(msg.into()))
}

/// A client-supplied block type, as it may appear in a debug log: only an
/// identifier of at most 64 characters is named, so request content
/// smuggled into the `type` field never reaches a log line.
fn block_type_for_log(s: &str) -> &str {
    let ident =
        !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if ident {
        s
    } else {
        "<not an identifier>"
    }
}

/// Tool names on the wire. Responses accepts `^[A-Za-z0-9_-]{1,64}$`;
/// anything else (an MCP name with a dot, say) is sent as
/// `clcd_tool_<index>` and mapped back on the way out. A client name that
/// already starts with the prefix is renamed too, so a generated name can
/// never collide with a real one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    wire: BTreeMap<String, String>,
    client: BTreeMap<String, String>,
}

pub const WIRE_PREFIX: &str = "clcd_tool_";

fn wire_safe(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !name.starts_with(WIRE_PREFIX)
}

impl Names {
    /// The map for one request, from its `tools` in order.
    pub fn for_request(req: &Value) -> Self {
        let mut wire = BTreeMap::new();
        for (i, tool) in req
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let Some(name) = tool.get("name").and_then(Value::as_str) {
                let w = if wire_safe(name) {
                    name.to_owned()
                } else {
                    format!("{WIRE_PREFIX}{i}")
                };
                wire.insert(name.to_owned(), w);
            }
        }
        Self::from_wire_map(wire)
    }

    /// `client name → wire name`, as the fixtures record it.
    pub fn from_wire_map(wire: BTreeMap<String, String>) -> Self {
        let client = wire.iter().map(|(c, w)| (w.clone(), c.clone())).collect();
        Self { wire, client }
    }

    pub fn wire<'a>(&'a self, client: &'a str) -> &'a str {
        self.wire.get(client).map_or(client, String::as_str)
    }

    pub fn client<'a>(&'a self, wire: &'a str) -> &'a str {
        self.client.get(wire).map_or(wire, String::as_str)
    }
}

/// What the caller decides; the rest comes from the request.
#[derive(Debug, Clone)]
pub struct RequestOpts {
    /// The upstream model, already mapped through the route.
    pub model: String,
    /// Used when the client names no effort (clai's `CLCD_EFFORT`).
    pub effort: String,
    /// Send [`Self::effort`] whatever the client asked for.
    pub effort_override: bool,
    pub translation: Translation,
}

pub const DEFAULT_EFFORT: &str = "medium";

const SUPPORTED_EFFORT: [&str; 4] = ["low", "medium", "high", "xhigh"];

fn text_part(role: &str, text: &Value) -> Value {
    let kind = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    json!({"type": kind, "text": text})
}

fn image_part(block: &Value) -> Result<Value, RequestError> {
    let source = block.get("source").unwrap_or(&Value::Null);
    let url = match source.get("type").and_then(Value::as_str) {
        Some("base64") => {
            let mt = source.get("media_type").and_then(Value::as_str);
            let data = source.get("data").and_then(Value::as_str);
            match (mt, data) {
                (Some(mt), Some(data)) => format!("data:{mt};base64,{data}"),
                _ => return err("Unsupported image source"),
            }
        }
        Some("url") => match source.get("url").and_then(Value::as_str) {
            Some(u) => u.to_owned(),
            None => return err("Unsupported image source"),
        },
        _ => return err("Unsupported image source"),
    };
    Ok(json!({"type": "input_image", "image_url": url}))
}

fn block_type(block: &Value) -> &str {
    block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn parts(role: &str, content: &Value) -> Result<Vec<Value>, RequestError> {
    if let Some(s) = content.as_str() {
        return Ok(vec![text_part(role, &Value::String(s.to_owned()))]);
    }
    let Some(blocks) = content.as_array() else {
        return err("Unsupported message content");
    };
    blocks
        .iter()
        .map(|b| match block_type(b) {
            "text" => Ok(text_part(role, b.get("text").unwrap_or(&Value::Null))),
            "image" if role == "user" => image_part(b),
            other => err(format!(
                "Unsupported content block: {}",
                block_type_for_log(other)
            )),
        })
        .collect()
}

fn message(role: &str, content: &Value) -> Result<Value, RequestError> {
    Ok(json!({"type": "message", "role": role, "content": parts(role, content)?}))
}

fn tool_output(block: &Value) -> Result<String, RequestError> {
    let mut out = String::new();
    if block.get("is_error").and_then(Value::as_bool) == Some(true) {
        out.push_str("[Tool error]\n");
    }
    match block.get("content").unwrap_or(&Value::Null) {
        Value::Null => {}
        Value::String(s) => out.push_str(s),
        Value::Array(items) => {
            let mut texts = Vec::new();
            for item in items {
                match block_type(item) {
                    "text" => texts.push(item.get("text").and_then(Value::as_str).unwrap_or("")),
                    "image" => {}
                    _ => return err("Unsupported tool result block"),
                }
            }
            out.push_str(&texts.join("\n"));
        }
        _ => return err("Unsupported tool result block"),
    }
    Ok(out)
}

fn inputs(req: &Value, names: &Names) -> Result<Vec<Value>, RequestError> {
    let messages = match req.get("messages") {
        Some(Value::Array(m)) => m.as_slice(),
        None | Some(Value::Null) => &[],
        Some(_) => return err("Unsupported messages"),
    };
    let mut out = Vec::new();
    for m in messages {
        let role = match m.get("role").and_then(Value::as_str) {
            Some("system") => "developer",
            Some(r) => r,
            None => return err("Unsupported message role"),
        };
        let content = m.get("content").unwrap_or(&Value::Null);
        match content.as_array() {
            Some(blocks) => message_blocks(role, blocks, names, &mut out)?,
            None => out.push(message(role, content)?),
        }
    }
    Ok(out)
}

/// One message's blocks as input items, in order.
///
/// Images lifted out of tool results (E3) wait until the run of results
/// ends: a user message between two results would break the tool-result
/// sequence a Chat provider requires.
fn message_blocks(
    role: &str,
    blocks: &[Value],
    names: &Names,
    out: &mut Vec<Value>,
) -> Result<(), RequestError> {
    let mut lifted = Vec::new();
    for b in blocks {
        let kind = block_type(b);
        if !matches!(kind, "tool_result" | "thinking" | "redacted_thinking") {
            out.append(&mut lifted);
        }
        match kind {
            "tool_use" => out.push(function_call(b, names)?),
            "tool_result" => {
                out.push(function_call_output(b)?);
                lifted.extend(lifted_images(b)?);
            }
            // Reasoning is not replayed to another provider (reviewed E1).
            "thinking" | "redacted_thinking" => {}
            _ => out.push(message(role, &Value::Array(vec![b.clone()]))?),
        }
    }
    out.append(&mut lifted);
    Ok(())
}

fn function_call(b: &Value, names: &Names) -> Result<Value, RequestError> {
    let Some(name) = b.get("name").and_then(Value::as_str) else {
        return err("Unsupported content block: tool_use");
    };
    let input = b.get("input").unwrap_or(&Value::Null);
    Ok(json!({
        "type": "function_call",
        "call_id": b.get("id").unwrap_or(&Value::Null),
        "name": names.wire(name),
        "arguments": serde_json::to_string(input).unwrap_or_default(),
    }))
}

fn function_call_output(b: &Value) -> Result<Value, RequestError> {
    Ok(json!({
        "type": "function_call_output",
        "call_id": b.get("tool_use_id").unwrap_or(&Value::Null),
        "output": tool_output(b)?,
    }))
}

/// Responses has no image in a tool output, so each one follows the
/// results as a user message (clai, E3).
fn lifted_images(b: &Value) -> Result<Vec<Value>, RequestError> {
    b.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|i| block_type(i) == "image")
        .map(|img| message("user", &Value::Array(vec![img.clone()])))
        .collect()
}

fn present(v: Option<&Value>) -> bool {
    !matches!(v, None | Some(Value::Null))
}

fn check(req: &Value, opts: &RequestOpts) -> Result<(), RequestError> {
    let stops = match req.get("stop_sequences") {
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::String(s)) => !s.is_empty(),
        _ => false,
    };
    if present(req.get("temperature"))
        || present(req.get("top_p"))
        || present(req.get("top_k"))
        || stops
    {
        return err("Sampling parameters and stop_sequences are unsupported by this Codex bridge");
    }
    let config = req.get("output_config");
    if present(config.and_then(|c| c.get("format"))) || present(req.get("output_format")) {
        return err("Structured output format controls are unsupported by this Codex bridge");
    }
    if req.pointer("/thinking/type").and_then(Value::as_str) == Some("enabled") {
        return err("Fixed thinking budgets are unsupported; use adaptive thinking or CLCD_EFFORT");
    }
    let effort = config.and_then(|c| c.get("effort"));
    if present(effort) {
        if matches!(opts.translation, Translation::Grok | Translation::Chat) {
            return err("Reasoning effort is unsupported by this route");
        }
        let known = effort
            .and_then(Value::as_str)
            .is_some_and(|e| SUPPORTED_EFFORT.contains(&e));
        if !opts.effort_override && !known {
            return err("Unsupported reasoning effort; supported levels: low, medium, high, xhigh");
        }
    }
    Ok(())
}

fn instructions(req: &Value) -> String {
    match req.get("system") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|b| b.get("text").and_then(Value::as_str).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The request's tools. Absent or `null` is no tools; any other shape
/// that is not an array is refused — sending the request on with its tools
/// silently dropped would let the model answer without them.
fn tools(req: &Value, names: &Names) -> Result<Vec<Value>, RequestError> {
    let tools = match req.get("tools") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(t)) => t.as_slice(),
        Some(_) => return err("Unsupported tools"),
    };
    tools
        .iter()
        .map(|t| {
            let name = t.get("name").and_then(Value::as_str);
            let schema = t.get("input_schema").filter(|s| !s.is_null());
            let (Some(name), Some(schema)) = (name, schema) else {
                return err("Unsupported server-side tool");
            };
            Ok(json!({
                "type": "function",
                "name": names.wire(name),
                "description": t.get("description").filter(|d| !d.is_null())
                    .cloned().unwrap_or(Value::String(String::new())),
                "parameters": schema,
                "strict": false,
            }))
        })
        .collect()
}

/// Translate one Messages request. `names` must be
/// [`Names::for_request`] of the same request, so the reply maps back.
pub fn to_responses(req: &Value, names: &Names, opts: &RequestOpts) -> Result<Value, RequestError> {
    if !req.is_object() {
        return err("Unsupported request");
    }
    check(req, opts)?;
    let choice = req.get("tool_choice");
    let client_effort = req
        .pointer("/output_config/effort")
        .filter(|e| !matches!(e, Value::Null | Value::Bool(false)));
    let effort = match client_effort {
        Some(e) if !opts.effort_override => e.clone(),
        _ => Value::String(opts.effort.clone()),
    };
    let mut out = Map::new();
    out.insert("model".into(), Value::String(opts.model.clone()));
    out.insert("instructions".into(), Value::String(instructions(req)));
    out.insert("input".into(), Value::Array(inputs(req, names)?));
    out.insert("store".into(), Value::Bool(false));
    out.insert("stream".into(), Value::Bool(true));
    out.insert("reasoning".into(), json!({ "effort": effort }));
    out.insert(
        "parallel_tool_calls".into(),
        Value::Bool(
            choice
                .and_then(|c| c.get("disable_parallel_tool_use"))
                .and_then(Value::as_bool)
                != Some(true),
        ),
    );
    out.insert("tools".into(), Value::Array(tools(req, names)?));
    match choice.and_then(|c| c.get("type")).and_then(Value::as_str) {
        Some("any") => {
            out.insert("tool_choice".into(), "required".into());
        }
        Some("none") => {
            out.insert("tool_choice".into(), "none".into());
        }
        Some("tool") => {
            let name = choice
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            out.insert(
                "tool_choice".into(),
                json!({"type": "function", "name": names.wire(name)}),
            );
        }
        _ => {}
    }
    if opts.translation != Translation::Codex {
        let max = req
            .get("max_tokens")
            .filter(|m| !m.is_null())
            .cloned()
            .unwrap_or(Value::from(4096));
        out.insert("max_output_tokens".into(), max);
        if opts.translation == Translation::Grok {
            out.remove("reasoning");
        }
    }
    Ok(Value::Object(out))
}

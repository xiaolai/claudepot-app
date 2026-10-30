//! Replays clai's captured fixtures (`testdata/translate/`) through the
//! Rust port, one step at a time.
//!
//! Each step names a clai filter; [`run_step`] maps it to the port's
//! function and compares the result with what clai printed. A step the
//! relay does not reimplement (shell glue, clai's bash metadata lookup,
//! the test-suite helpers) contributes clai's recorded output so the
//! chain behind it still runs.
//!
//! The fixtures record clai, not a specification, so four deviations are
//! asserted rather than matched (README "Reviewed exceptions"):
//!
//! - **Model.** The relay sends the route's mapped model; clai resolved
//!   Claude names through its own role table. Model-role cases compare
//!   everything but `model`, and assert the route's model went out.
//! - **E4.** A call that never finishes is refused (an `error` event),
//!   where clai emitted a finished `tool_use`.
//! - **E5/E6, rejections.** The relay's refusals are fixed codes, so a
//!   reject step must be refused; its text is not compared.
//! - **Upstream errors.** Messages are fixed, so the provider's text
//!   (and any key it echoed) never reaches the client; type, status and
//!   `Retry-After` must match.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use serde_json::{json, Value};

use super::chat::{to_chat, ChatError, ChatToResponses, Out};
use super::errors::anthropic_error;
use super::request::{to_responses, Names, RequestOpts};
use super::stream::{parse_event, Collector, Event, Line, ResponsesToAnthropic, SseLines};
use super::Translation;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/translate")
}

fn cases() -> Vec<(PathBuf, Value)> {
    let mut out = Vec::new();
    for family in ["responses", "chat"] {
        let dir = root().join(family);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                n.ends_with(".expect.json") || n.ends_with(".reject.json")
            })
            .collect();
        paths.sort();
        for p in paths {
            let v: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            out.push((dir.clone(), v));
        }
    }
    out
}

/// A step's output: parsed JSON, raw SSE bytes, or both.
#[derive(Clone, Debug, Default)]
struct Output {
    json: Option<Value>,
    raw: Option<String>,
}

fn recorded(step: &Value) -> Output {
    Output {
        json: step.get("output").cloned(),
        raw: step
            .get("output_raw")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

struct Ctx<'a> {
    dir: &'a Path,
    case: &'a str,
    outputs: HashMap<String, Output>,
}

impl Ctx<'_> {
    fn input(&self, step: &Value) -> Output {
        match step.get("input") {
            Some(Value::Object(o)) if o.contains_key("file") => {
                let name = o["file"].as_str().unwrap();
                let raw = String::from_utf8(std::fs::read(self.dir.join(name)).unwrap()).unwrap();
                Output {
                    json: serde_json::from_str(&raw).ok(),
                    raw: Some(raw),
                }
            }
            Some(Value::Object(o)) if o.contains_key("step") => {
                let label = o["step"].as_str().unwrap();
                self.outputs
                    .get(label)
                    .cloned()
                    .unwrap_or_else(|| panic!("{}: no step {label}", self.case))
            }
            _ => Output::default(),
        }
    }
}

fn arg<'a>(step: &'a Value, key: &str) -> Option<&'a str> {
    step.pointer(&format!("/params/arg/{key}"))
        .and_then(Value::as_str)
}

fn slurp_map(step: &Value, key: &str) -> BTreeMap<String, Value> {
    step.pointer(&format!("/params/slurpfile/{key}/0"))
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn string_map(m: BTreeMap<String, Value>) -> BTreeMap<String, String> {
    m.into_iter()
        .map(|(k, v)| (k, v.as_str().unwrap().to_owned()))
        .collect()
}

/// The request's names arrive `client → wire`; a stream's `wire → client`.
fn request_names(step: &Value) -> Names {
    Names::from_wire_map(string_map(slurp_map(step, "names")))
}

fn stream_names(step: &Value) -> Names {
    let reverse = string_map(slurp_map(step, "names"));
    Names::from_wire_map(reverse.into_iter().map(|(w, c)| (c, w)).collect())
}

fn translation(step: &Value) -> Translation {
    // A fixture without a transport is clcd's default: Codex.
    // clai's `local` route is the relay's plain `chat`.
    arg(step, "transport").map_or(Translation::Codex, |t| match t {
        "local" => Translation::Chat,
        t => Translation::parse(t).unwrap_or_else(|| panic!("transport {t}")),
    })
}

fn lines(raw: &str) -> Vec<Line> {
    let mut sse = SseLines::default();
    let mut out = sse.feed(raw.as_bytes()).unwrap();
    out.extend(sse.finish().unwrap());
    out
}

fn event_lines(events: &[Event]) -> Vec<Value> {
    events
        .iter()
        .flat_map(|e| [json!({"event": e.kind}), json!({"data": e.data})])
        .collect()
}

fn anthropic_stream(raw: &str, model: &str, names: Names) -> Vec<Event> {
    let mut conv = ResponsesToAnthropic::new(model, names);
    let mut events = Vec::new();
    for line in lines(raw) {
        events.extend(conv.feed_line(&line));
    }
    events.extend(conv.finish());
    events
}

fn collect(raw: &str, model: &str, names: &Names) -> Result<Value, super::stream::Fault> {
    let mut c = Collector::default();
    for line in lines(raw) {
        if let Line::Data(d) = line {
            match parse_event(&d) {
                Ok(e) => c.feed_event(e),
                Err(f) => c.fail(f),
            }
        }
    }
    c.finish(model, names)
}

/// `Ok(lines)` for a chat stream that ended cleanly, `Err(partial)` for
/// one refused part way.
fn chat_stream(raw: &str, model: &str) -> Result<Vec<Value>, Vec<Value>> {
    let mut conv = ChatToResponses::new(model, true);
    let mut out = Vec::new();
    let push = |outs: Vec<Out>, out: &mut Vec<Value>| {
        for o in outs {
            out.push(match o {
                Out::Event(e) => json!({"data": e}),
                Out::Cache {
                    id,
                    model,
                    extra_content,
                } => json!({"cache": {"id": id, "model": model, "extra_content": extra_content}}),
            });
        }
    };
    let mut sse = SseLines::default();
    let mut all = match sse.feed(raw.as_bytes()) {
        Ok(l) => l,
        Err(_) => return Err(out),
    };
    match sse.finish() {
        Ok(l) => all.extend(l),
        Err(_) => return Err(out),
    }
    for line in &all {
        match conv.feed_line(line) {
            Ok(o) => push(o, &mut out),
            Err(_) => return Err(out),
        }
    }
    match conv.finish() {
        Ok(o) => push(o, &mut out),
        Err(_) => return Err(out),
    }
    Ok(out)
}

fn raw_of(lines: &[Value]) -> String {
    let mut s = String::new();
    for l in lines {
        if let Some(d) = l.get("data") {
            s.push_str("data: ");
            s.push_str(&d.to_string());
            s.push('\n');
        } else if let Some(c) = l.get("cache") {
            s.push_str("cache: ");
            s.push_str(&c.to_string());
            s.push('\n');
        }
    }
    s
}

fn without_model(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.remove("model");
    }
    v
}

fn mask_errors(lines: &[Value]) -> Vec<Value> {
    lines
        .iter()
        .map(|l| {
            let mut l = l.clone();
            if l.pointer("/data/type") == Some(&json!("error")) {
                l["data"]["error"]["message"] = Value::Null;
            }
            l
        })
        .collect()
}

fn expected_lines(step: &Value, key: &str) -> Vec<Value> {
    step.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// What happened to one step, for the summary assertion.
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
enum Ran {
    Matched,
    Refused,
    Deviation,
    Recorded,
}

fn run_step(ctx: &mut Ctx<'_>, step: &Value) -> Ran {
    let case = ctx.case.to_owned();
    let label = step["label"].as_str().unwrap().to_owned();
    let kind = step["kind"].as_str().unwrap();
    let filter = step.get("filter").and_then(Value::as_str).unwrap_or("");
    let reject = kind == "reject";
    let at = format!("{case}/{label}");
    let input = ctx.input(step);

    let (ran, out) = match filter {
        "clcd-names.jq" => {
            let req = input.json.unwrap();
            let names = Names::for_request(&req);
            let want = step["output"].as_object().unwrap();
            let tools: Vec<&str> = req
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| t.get("name").and_then(Value::as_str))
                .collect();
            assert_eq!(tools.len(), want.len(), "{at}: one entry per tool");
            for name in tools {
                assert_eq!(Some(&json!(names.wire(name))), want.get(name), "{at}");
            }
            (Ran::Matched, recorded(step))
        }
        "clcd-reverse.jq" => {
            let names = Names::from_wire_map(string_map(
                input
                    .json
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .clone()
                    .into_iter()
                    .collect(),
            ));
            for (wire, client) in step["output"].as_object().unwrap() {
                assert_eq!(names.client(wire), client.as_str().unwrap(), "{at}");
            }
            (Ran::Matched, recorded(step))
        }
        "clcd-request.jq" => {
            let req = input.json.unwrap();
            let opts = RequestOpts {
                model: arg(step, "model").unwrap().to_owned(),
                effort: arg(step, "effort").unwrap_or("medium").to_owned(),
                effort_override: arg(step, "effort_override") == Some("true"),
                translation: translation(step),
            };
            let got = to_responses(&req, &request_names(step), &opts);
            if reject {
                assert!(got.is_err(), "{at}: must be refused, got {got:?}");
                (Ran::Refused, Output::default())
            } else {
                let got = got.unwrap_or_else(|e| panic!("{at}: {e:?}"));
                let want = &step["output"];
                // clai resolves the request's model through its role and
                // profile tables; the relay always sends the route's map.
                assert_eq!(got["model"], json!(opts.model), "{at}: route model");
                assert_eq!(
                    without_model(got.clone()),
                    without_model(want.clone()),
                    "{at}"
                );
                let ran = if got["model"] == want["model"] {
                    Ran::Matched
                } else {
                    Ran::Deviation
                };
                (
                    ran,
                    Output {
                        json: Some(got),
                        raw: None,
                    },
                )
            }
        }
        "clcd-stream.jq" => {
            let raw = input.raw.unwrap();
            let events = anthropic_stream(&raw, arg(step, "model").unwrap(), stream_names(step));
            let got = event_lines(&events);
            let ended_in_error = events.last().map(|e| e.kind) == Some("error");
            if reject {
                let partial = expected_lines(step, "partial_output_lines");
                assert!(ended_in_error, "{at}: must end in an error event");
                assert_eq!(
                    &got[..partial.len()],
                    &partial[..],
                    "{at}: what was already sent"
                );
                (Ran::Refused, Output::default())
            } else if case == "exception-unfinished-tool-call" {
                // E4: clai finishes the call; the relay refuses it.
                assert!(ended_in_error, "{at}: an unfinished call must be refused");
                assert_ne!(got, expected_lines(step, "output_lines"), "{at}");
                (Ran::Deviation, Output::default())
            } else {
                let want = expected_lines(step, "output_lines");
                // E6: an error event's text is the relay's fixed message.
                assert_eq!(mask_errors(&got), mask_errors(&want), "{at}");
                let ran = if got == want {
                    Ran::Matched
                } else {
                    Ran::Deviation
                };
                (ran, recorded(step))
            }
        }
        "clcd-collect.jq" => {
            let raw = input.raw.unwrap();
            let got = collect(&raw, arg(step, "model").unwrap(), &stream_names(step))
                .unwrap_or_else(|f| panic!("{at}: {f:?}"));
            assert_eq!(got, step["output"], "{at}");
            (
                Ran::Matched,
                Output {
                    json: Some(got),
                    raw: None,
                },
            )
        }
        "chat-request.jq" => {
            let req = input.json.unwrap();
            let meta = slurp_map(step, "metadata");
            let gemini = arg(step, "provider") == Some("gemini");
            let got = to_chat(&req, gemini, |id| {
                meta.get(id).and_then(|e| e.get("extra_content")).cloned()
            });
            if reject {
                assert_eq!(got, Err(ChatError::MissingMetadata), "{at}");
                (Ran::Refused, Output::default())
            } else {
                let got = got.unwrap_or_else(|e| panic!("{at}: {e:?}"));
                assert_eq!(got, step["output"], "{at}");
                (
                    Ran::Matched,
                    Output {
                        json: Some(got),
                        raw: None,
                    },
                )
            }
        }
        "chat-stream.jq" => {
            let raw = input.raw.unwrap();
            let got = chat_stream(&raw, arg(step, "model").unwrap());
            if reject {
                let partial = expected_lines(step, "partial_output_lines");
                let sent = got.expect_err(&format!("{at}: must be refused"));
                assert_eq!(sent, partial, "{at}: what was already sent");
                (Ran::Refused, Output::default())
            } else {
                let got = got.unwrap_or_else(|_| panic!("{at}: refused"));
                assert_eq!(got, expected_lines(step, "output_lines"), "{at}");
                (
                    Ran::Matched,
                    Output {
                        json: None,
                        raw: Some(raw_of(&got)),
                    },
                )
            }
        }
        f if f.starts_with("clcd-upstream-error.jq") => {
            let status: u16 = arg(step, "status").unwrap().parse().unwrap();
            let body = input.raw.unwrap();
            let got = anthropic_error(StatusCode::from_u16(status).unwrap(), body.as_bytes());
            let want = &step["output"];
            assert_eq!(got["type"], "error", "{at}");
            assert_eq!(got["error"]["type"], want["body"]["error"]["type"], "{at}");
            assert!(
                want["status_line"]
                    .as_str()
                    .unwrap()
                    .starts_with(&status.to_string()),
                "{at}: the status passes through"
            );
            let text = got.to_string();
            let theirs = want["body"]["error"]["message"].as_str().unwrap();
            let msg = got["error"]["message"].as_str().unwrap();
            if case == "upstream-error-context" {
                assert!(msg.starts_with("prompt is too long"), "{at}");
            }
            // Fixed messages: neither clai's echo of the provider nor the
            // raw body reaches the client.
            assert!(
                !text.contains(theirs) || theirs.starts_with("prompt is too long"),
                "{at}"
            );
            assert!(!text.contains("00000000000000000000000000000000"), "{at}");
            assert!(!body.is_empty() && !text.contains(body.trim()), "{at}");
            // Retry-After is forwarded by the handler from the headers.
            let headers = std::fs::read_to_string(
                ctx.dir
                    .join(step["input"]["headers_file"].as_str().unwrap()),
            )
            .unwrap();
            let retry = headers
                .lines()
                .find_map(|l| l.strip_prefix("Retry-After: "))
                .map(|v| v.trim().to_owned());
            assert_eq!(retry.as_deref(), want["retry_after"].as_str(), "{at}");
            (Ran::Deviation, Output::default())
        }
        // Not ported: shell glue, clai's bash cache lookup, the suites'
        // client helpers, and clcd's message formatter (the relay answers
        // with fixed refusal codes instead).
        _ => {
            assert!(
                kind == "glue"
                    || matches!(
                        filter,
                        "emulated-metadata-lookup.jq"
                            | "chat-sh-metadata.jq"
                            | "bridge-api-replay.jq"
                            | "bridge-api-client-collect.jq"
                            | "clcd-http-error.jq"
                    ),
                "{at}: unmapped filter {filter:?}"
            );
            (Ran::Recorded, recorded(step))
        }
    };
    ctx.outputs.insert(label, out);
    ran
}

#[test]
fn every_fixture_step_replays_through_the_port() {
    let mut tally: HashMap<Ran, usize> = HashMap::new();
    let all = cases();
    assert_eq!(all.len(), 40, "the manifest counts 40 cases");
    for (dir, case) in &all {
        let name = case["case"].as_str().unwrap();
        let mut ctx = Ctx {
            dir,
            case: name,
            outputs: HashMap::new(),
        };
        let steps = case["steps"].as_array().unwrap();
        let mut refused = false;
        for step in steps {
            let ran = run_step(&mut ctx, step);
            refused |= ran == Ran::Refused;
            *tally.entry(ran).or_default() += 1;
        }
        if case["kind"] == "reject" && name != "reject-replay-missing-metadata" {
            assert!(refused, "{name}: a reject case must refuse somewhere");
        }
    }
    // All 89 steps, by outcome. A step that moves category (a match that
    // became a deviation, a refusal that stopped refusing) changes these.
    let count = |r| tally.get(&r).copied().unwrap_or(0);
    assert_eq!(count(Ran::Matched), 45, "{tally:?}");
    assert_eq!(count(Ran::Deviation), 18, "{tally:?}");
    assert_eq!(count(Ran::Refused), 12, "{tally:?}");
    assert_eq!(count(Ran::Recorded), 14, "{tally:?}");
}

/// R2-7: the call ids must survive the round trip as a bijection. Swapping
/// two ids in the replay must change the request the provider sees —
/// otherwise a test comparing it would pass on a relay that crossed them.
#[test]
fn swapping_two_call_ids_breaks_the_replay() {
    let dir = root().join("chat");
    let case: Value = serde_json::from_slice(
        &std::fs::read(dir.join("roundtrip-chat-sh-signature.expect.json")).unwrap(),
    )
    .unwrap();
    let step = case["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["label"] == "upstream_request")
        .unwrap();
    let req: Value = serde_json::from_slice(
        &std::fs::read(dir.join("roundtrip-chat-sh-signature.input.json")).unwrap(),
    )
    .unwrap();
    let meta = slurp_map(step, "metadata");
    let lookup = |id: &str| meta.get(id).and_then(|e| e.get("extra_content")).cloned();
    let straight = to_chat(&req, true, lookup).unwrap();
    assert_eq!(straight, step["output"]);

    let swapped_req: Value =
        serde_json::from_str(&swap(&req.to_string(), "call_one", "call_two")).unwrap();
    let crossed = to_chat(&swapped_req, true, lookup).unwrap();
    assert_ne!(crossed, step["output"], "a crossed id must not pass");
}

fn swap(s: &str, a: &str, b: &str) -> String {
    s.replace(a, "\u{0}").replace(b, a).replace('\u{0}', b)
}

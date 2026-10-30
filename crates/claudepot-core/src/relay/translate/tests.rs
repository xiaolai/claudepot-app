//! Translated targets: what decides the target, and where requests go.

use uuid::Uuid;

use super::*;
use crate::routes::{AuthScheme, GatewayConfig};

pub(crate) fn route(base_url: &str, model: &str) -> Route {
    Route {
        id: Uuid::from_u128(0x7a),
        name: "Translated".into(),
        provider: RouteProvider::Gateway(GatewayConfig {
            base_url: base_url.into(),
            api_key: "sk-translated-CANARY".into(),
            auth_scheme: AuthScheme::Bearer,
            enable_tool_search: false,
            use_keychain: false,
        }),
        model: model.into(),
        small_fast_model: Some(format!("{model}-mini")),
        additional_models: vec![],
        wrapper_name: "claude-translated".into(),
        deployment_organization_uuid: Uuid::nil(),
        active_on_desktop: false,
        installed_on_cli: true,
        is_private_cloud: false,
        capabilities_override: None,
    }
}

#[test]
fn every_translation_round_trips_its_name() {
    for t in Translation::ALL {
        assert_eq!(Translation::parse(t.as_str()), Some(t));
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, format!("\"{}\"", t.as_str()));
    }
    assert_eq!(Translation::parse("anthropic"), None);
}

/// `clcd-names.jq`: a name the wire accepts goes as itself; anything else,
/// and anything already carrying the prefix, becomes `clcd_tool_<index>`.
/// No captured fixture has a name that needs renaming, so this is the test
/// that holds the rule.
#[test]
fn unsafe_tool_names_go_out_by_index_and_come_back() {
    let long = "x".repeat(65);
    let max = "x".repeat(64);
    let req = serde_json::json!({"tools": [
        {"name": "read_file"},
        {"name": "mcp__srv.tool"},
        {"name": long},
        {"name": "clcd_tool_9"},
        {"name": max},
    ]});
    let names = request::Names::for_request(&req);
    let want = [
        ("read_file", "read_file".to_owned()),
        ("mcp__srv.tool", "clcd_tool_1".to_owned()),
        (long.as_str(), "clcd_tool_2".to_owned()),
        ("clcd_tool_9", "clcd_tool_3".to_owned()),
        (max.as_str(), max.clone()),
    ];
    for (client, wire) in &want {
        assert_eq!(names.wire(client), wire, "{client}");
        assert_eq!(names.client(wire), *client, "{wire}");
    }
}

#[test]
fn the_endpoint_follows_the_protocol_and_is_not_doubled() {
    let o = "https://api.openai.com/v1";
    assert_eq!(
        endpoint(o, Translation::Openai),
        "https://api.openai.com/v1/responses"
    );
    assert_eq!(
        endpoint("https://api.openai.com/v1/responses", Translation::Openai),
        "https://api.openai.com/v1/responses"
    );
    assert_eq!(
        endpoint(
            "https://generativelanguage.googleapis.com/v1beta/openai",
            Translation::Gemini
        ),
        "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
    );
    assert_eq!(
        endpoint(CODEX_ORIGIN, Translation::Codex),
        format!("{CODEX_ORIGIN}/responses")
    );
}

/// The URL a user saves goes through the save path's normalisation
/// (which strips a bare `/v1`) before it is dispatched to: the request
/// must still reach the provider's `/v1` API.
#[test]
fn a_saved_v1_base_url_still_dispatches_under_v1() {
    for (typed, t, want) in [
        (
            "https://api.openai.com/v1",
            Translation::Openai,
            "https://api.openai.com/v1/responses",
        ),
        (
            "https://api.x.ai/v1/",
            Translation::Grok,
            "https://api.x.ai/v1/responses",
        ),
        (
            "http://localhost:11434/v1",
            Translation::Chat,
            "http://localhost:11434/v1/chat/completions",
        ),
        (
            "https://api.openai.com",
            Translation::Openai,
            "https://api.openai.com/v1/responses",
        ),
        (
            "https://openrouter.ai/api/v1",
            Translation::Chat,
            "https://openrouter.ai/api/v1/chat/completions",
        ),
        (
            "https://generativelanguage.googleapis.com/v1beta/openai",
            Translation::Gemini,
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
        ),
    ] {
        let saved = crate::routes::normalize_gateway_base_url(typed).unwrap();
        let origin = origin_of(&route(&saved, "m")).unwrap();
        assert_eq!(endpoint(&origin, t), want, "{typed}");
    }
    let codex = origin_of(&route(CODEX_ORIGIN, "m")).unwrap();
    assert_eq!(
        endpoint(&codex, Translation::Codex),
        format!("{CODEX_ORIGIN}/responses")
    );
}

#[test]
fn a_translated_spec_differs_from_the_pass_through_spec_of_the_same_route() {
    let r = route("https://api.x.ai/v1/", "grok-4");
    let pass = crate::relay::passthrough::route_target_spec(&r).unwrap();
    let grok = route_target_spec(&r, Translation::Grok).unwrap();
    let openai = route_target_spec(&r, Translation::Openai).unwrap();
    assert_eq!(grok.kind, TargetKind::Translated);
    assert_eq!(grok.origin, "https://api.x.ai/v1");
    assert_eq!(grok.protocol.as_deref(), Some("grok"));
    assert_ne!(grok.digest(), pass.digest());
    assert_ne!(grok.digest(), openai.digest());
    // A keyed route pays with its key, as a pass-through route does.
    assert_eq!(grok.credential, pass.credential);
}

#[test]
fn a_codex_route_must_name_the_codex_backend_and_pays_with_the_relay_login() {
    let wrong = route("https://api.openai.com/v1", "gpt-5");
    assert_eq!(
        route_target_spec(&wrong, Translation::Codex).unwrap_err(),
        TranslateRouteError::CodexOrigin
    );
    let r = route(CODEX_ORIGIN, "gpt-5-codex");
    let spec = route_target_spec(&r, Translation::Codex).unwrap();
    let keyed = route_target_spec(&r, Translation::Openai).unwrap();
    assert_ne!(spec.credential, keyed.credential);
}

#[test]
fn plain_http_is_refused_except_on_loopback() {
    for ok in [
        "http://127.0.0.1:9000",
        "http://localhost:1234/v1",
        "http://[::1]:80",
    ] {
        assert!(
            route_target_spec(&route(ok, "m"), Translation::Chat).is_ok(),
            "{ok}"
        );
    }
    for bad in [
        "http://10.0.0.5/v1",
        "http://example.com",
        "http://127.0.0.1.evil.com",
    ] {
        assert_eq!(
            route_target_spec(&route(bad, "m"), Translation::Chat).unwrap_err(),
            TranslateRouteError::Insecure,
            "{bad}"
        );
    }
}

#[test]
fn a_loopback_lookalike_or_a_url_with_userinfo_is_refused() {
    for bad in [
        // The host is example.invalid; `localhost:80` is userinfo.
        "http://localhost:80@example.invalid/v1",
        "http://127.0.0.1@example.invalid",
        "http://[::1]@example.invalid",
        "http://localhost.example.invalid",
        "http://localhost%2eexample.invalid",
        // Userinfo is refused on https too: reqwest would send it.
        "https://user:pw@api.example.com/v1",
        "https://user@api.example.com/v1",
        "http://user:pw@127.0.0.1:9000",
        "ftp://localhost/v1",
        "not a url",
    ] {
        assert_eq!(
            route_target_spec(&route(bad, "m"), Translation::Chat).unwrap_err(),
            TranslateRouteError::Insecure,
            "{bad}"
        );
    }
    for ok in [
        "http://127.0.0.2:9000",
        "http://LOCALHOST:1234/v1",
        "https://api.example.com/v1",
    ] {
        assert!(
            route_target_spec(&route(ok, "m"), Translation::Chat).is_ok(),
            "{ok}"
        );
    }
}

/// §4.2: a refusal's reason reaches only a debug log, and a block type is
/// client-controlled, so anything that is not an identifier is not named.
#[test]
fn a_refused_block_type_is_named_only_when_it_is_an_identifier() {
    let opts = request::RequestOpts {
        model: "m".into(),
        effort: "medium".into(),
        effort_override: false,
        translation: Translation::Openai,
    };
    let with_type =
        |t: &str| serde_json::json!({"messages": [{"role": "user", "content": [{"type": t}]}]});
    let reason = |req: &serde_json::Value| {
        let names = request::Names::for_request(req);
        request::to_responses(req, &names, &opts)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        reason(&with_type("unknown")),
        "Unsupported content block: unknown"
    );
    for hostile in [
        "my password is hunter2",
        "sk-ant-oat01-CANARY",
        &"x".repeat(65),
    ] {
        let r = reason(&with_type(hostile));
        assert!(!r.contains(hostile), "{r}");
        assert_eq!(r, "Unsupported content block: <not an identifier>");
    }
}

/// E3 lifts a tool result's images into a user message; with parallel
/// results the lifted message must follow all of them, or Chat history
/// reads tool(A), user, tool(B) and breaks the tool-result sequence.
#[test]
fn images_lifted_from_parallel_tool_results_follow_every_result() {
    use serde_json::json;
    let opts = request::RequestOpts {
        model: "m".into(),
        effort: "medium".into(),
        effort_override: false,
        translation: Translation::Chat,
    };
    let image = |d: &str| {
        json!({"type": "image",
        "source": {"type": "base64", "media_type": "image/png", "data": d}})
    };
    let req = json!({"messages": [{"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "a",
         "content": [{"type": "text", "text": "A"}, image("AAA")]},
        {"type": "tool_result", "tool_use_id": "b",
         "content": [image("BBB")]},
        {"type": "text", "text": "next"},
    ]}]});
    let names = request::Names::for_request(&req);
    let out = request::to_responses(&req, &names, &opts).unwrap();
    let shape: Vec<String> = out["input"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| match i["type"].as_str().unwrap() {
            "function_call_output" => format!("tool:{}", i["call_id"].as_str().unwrap()),
            _ => format!(
                "{}:{}",
                i["role"].as_str().unwrap(),
                i["content"][0]["type"]
            ),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "tool:a",
            "tool:b",
            "user:\"input_image\"",
            "user:\"input_image\"",
            "user:\"input_text\""
        ]
    );
}

// ---------------------------------------------------------------------
// Chat tool-call indexing
// ---------------------------------------------------------------------

mod chat_calls {
    use serde_json::{json, Value};

    use super::super::chat::{ChatToResponses, Out};
    use super::super::stream::{Fault, Line};

    fn feed(events: &[Value]) -> Result<Vec<Value>, Fault> {
        let mut c = ChatToResponses::new("m", false);
        let mut out = Vec::new();
        for e in events {
            for o in c.feed_line(&Line::Data(e.to_string()))? {
                if let Out::Event(v) = o {
                    out.push(v);
                }
            }
        }
        Ok(out)
    }

    fn tool(frags: Value) -> Value {
        json!({"id": "c1", "choices": [{"delta": {"tool_calls": frags}}]})
    }

    fn finish() -> Value {
        json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]})
    }

    /// `(call_id, arguments)` of every finished call, in order.
    fn done_calls(out: &[Value]) -> Vec<(String, String)> {
        out.iter()
            .filter(|v| v["type"] == "response.output_item.done")
            .map(|v| {
                let item = &v["item"];
                (
                    item["call_id"].as_str().unwrap().to_owned(),
                    item["arguments"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn an_indexless_continuation_of_a_sole_sparse_call_extends_that_call() {
        let out = feed(&[
            tool(json!([{"index": 3, "id": "call_a",
                         "function": {"name": "f", "arguments": "{\"a\":"}}])),
            tool(json!([{"function": {"arguments": "1}"}}])),
            finish(),
        ])
        .unwrap();
        assert_eq!(
            done_calls(&out),
            [("call_a".to_owned(), "{\"a\":1}".to_owned())]
        );
    }

    #[test]
    fn a_new_indexless_call_never_lands_on_an_existing_sparse_key() {
        let out = feed(&[
            tool(json!([{"index": 1, "id": "call_a",
                         "function": {"name": "f", "arguments": "{}"}}])),
            tool(json!([{"id": "call_b", "function": {"name": "g", "arguments": "{}"}}])),
            finish(),
        ])
        .unwrap();
        assert_eq!(
            done_calls(&out),
            [
                ("call_a".to_owned(), "{}".to_owned()),
                ("call_b".to_owned(), "{}".to_owned())
            ]
        );
    }

    #[test]
    fn an_indexless_idless_fragment_with_several_calls_is_malformed() {
        let r = feed(&[
            tool(json!([
                {"index": 0, "id": "call_a", "function": {"name": "f", "arguments": "{}"}},
                {"index": 4, "id": "call_b", "function": {"name": "g", "arguments": "{}"}},
            ])),
            tool(json!([{"function": {"arguments": "x"}}])),
        ]);
        assert_eq!(r.unwrap_err(), Fault::Malformed);
    }

    #[test]
    fn an_oversized_call_id_or_name_is_refused_before_it_is_kept() {
        let long = "x".repeat(super::super::MAX_CALL_FIELD + 1);
        for frag in [
            json!([{"index": 0, "id": long, "function": {"name": "f", "arguments": "{}"}}]),
            json!([{"index": 0, "id": "call_a", "function": {"name": long, "arguments": "{}"}}]),
        ] {
            assert_eq!(feed(&[tool(frag)]).unwrap_err(), Fault::Oversize);
        }
    }

    #[test]
    fn an_unknown_or_non_string_finish_reason_is_malformed() {
        for reason in [json!("exploded"), json!(1), json!({"why": "stop"})] {
            let r = feed(
                &[json!({"id": "c1", "choices": [{"delta": {"content": "hi"},
                                                         "finish_reason": reason}]})],
            );
            assert_eq!(r.unwrap_err(), Fault::Malformed, "{reason}");
        }
        for reason in [
            "stop",
            "length",
            "tool_calls",
            "content_filter",
            "function_call",
        ] {
            let r = feed(
                &[json!({"id": "c1", "choices": [{"delta": {"content": "hi"},
                                                         "finish_reason": reason}]})],
            );
            assert!(r.is_ok(), "{reason}");
        }
    }
}

// ---------------------------------------------------------------------
// Function-call completion, streaming and collected
// ---------------------------------------------------------------------

mod function_calls {
    use serde_json::{json, Value};

    use super::super::request::Names;
    use super::super::stream::{Collector, Fault, ResponsesToAnthropic};
    use super::super::{MAX_ARGS_PER_CALL, MAX_ARGS_TOTAL};

    fn added() -> Value {
        json!({"type": "response.output_item.added",
               "item": {"id": "fc_1", "type": "function_call", "call_id": "call_1", "name": "f"}})
    }

    fn args(part: &str) -> Value {
        json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": part})
    }

    fn done(item: Value) -> Value {
        let mut base =
            json!({"id": "fc_1", "type": "function_call", "call_id": "call_1", "name": "f"});
        if let (Some(b), Some(extra)) = (base.as_object_mut(), item.as_object()) {
            for (k, v) in extra {
                b.insert(k.clone(), v.clone());
            }
        }
        json!({"type": "response.output_item.done", "item": base})
    }

    fn completed() -> Value {
        json!({"type": "response.completed", "response": {"id": "r", "usage": {}}})
    }

    /// The stream's events as `(type, error message or partial_json)`.
    fn stream(events: &[Value]) -> Vec<(String, Option<String>)> {
        let mut t = ResponsesToAnthropic::new("m", Names::default());
        let mut out = Vec::new();
        for e in events {
            for ev in t.feed_event(e) {
                let detail = ev
                    .data
                    .pointer("/delta/partial_json")
                    .or_else(|| ev.data.pointer("/error/message"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                out.push((ev.kind.to_owned(), detail));
            }
        }
        out
    }

    fn partial_json(out: &[(String, Option<String>)]) -> String {
        out.iter()
            .filter(|(k, _)| k == "content_block_delta")
            .filter_map(|(_, d)| d.clone())
            .collect()
    }

    fn failed_with(out: &[(String, Option<String>)], f: Fault) -> bool {
        out.last() == Some(&("error".to_owned(), Some(f.message().to_owned())))
    }

    #[test]
    fn streamed_arguments_that_complete_a_json_object_close_the_call() {
        let out = stream(&[
            added(),
            args("{\"a\":"),
            args("1}"),
            done(json!({"arguments": "{\"a\":1}", "status": "completed"})),
            completed(),
        ]);
        assert_eq!(partial_json(&out), "{\"a\":1}");
        assert_eq!(out.last().unwrap().0, "message_stop");
    }

    #[test]
    fn terminal_arguments_alone_are_sent_once() {
        let out = stream(&[
            added(),
            done(json!({"arguments": "{\"a\":1}"})),
            completed(),
        ]);
        assert_eq!(partial_json(&out), "{\"a\":1}");
    }

    #[test]
    fn streamed_fragments_that_are_not_a_json_object_fail_the_stream() {
        for frags in [&["{\"a\":"][..], &["[1,", "2]"], &["not json"]] {
            let mut events = vec![added()];
            events.extend(frags.iter().map(|f| args(f)));
            events.push(done(json!({})));
            events.push(completed());
            let out = stream(&events);
            assert!(failed_with(&out, Fault::Malformed), "{frags:?}: {out:?}");
            assert!(!out.iter().any(|(k, _)| k == "message_stop"), "{frags:?}");
        }
    }

    #[test]
    fn terminal_arguments_that_disagree_with_the_stream_fail_it() {
        let out = stream(&[
            added(),
            args("{\"a\":1}"),
            done(json!({"arguments": "{\"a\":2}"})),
            completed(),
        ]);
        assert!(failed_with(&out, Fault::Malformed), "{out:?}");
    }

    #[test]
    fn terminal_arguments_that_are_not_an_object_fail_the_stream() {
        let out = stream(&[added(), done(json!({"arguments": "[]"})), completed()]);
        assert!(failed_with(&out, Fault::Malformed), "{out:?}");
    }

    #[test]
    fn a_call_done_without_completing_is_truncated() {
        for status in ["incomplete", "in_progress"] {
            let out = stream(&[
                added(),
                done(json!({"arguments": "{}", "status": status})),
                completed(),
            ]);
            assert!(failed_with(&out, Fault::Truncated), "{status}: {out:?}");
        }
    }

    #[test]
    fn streamed_arguments_are_bounded_per_call_and_in_total() {
        let big = "x".repeat(MAX_ARGS_PER_CALL / 2 + 1);
        let out = stream(&[added(), args(&big), args(&big)]);
        assert!(failed_with(&out, Fault::Oversize), "per call");

        let mut t = ResponsesToAnthropic::new("m", Names::default());
        let chunk = "x".repeat(MAX_ARGS_PER_CALL / 2);
        let calls = MAX_ARGS_TOTAL / MAX_ARGS_PER_CALL * 2 + 1;
        let mut last = Vec::new();
        for i in 0..calls {
            let id = format!("fc_{i}");
            t.feed_event(&json!({"type": "response.output_item.added",
                "item": {"id": id, "type": "function_call", "call_id": id, "name": "f"}}));
            last = t.feed_event(&json!({"type": "response.function_call_arguments.delta",
                "item_id": id, "delta": chunk}));
            if t.ended() {
                break;
            }
        }
        assert!(t.ended(), "the total is bounded");
        assert_eq!(
            last.last().and_then(|e| e.data.pointer("/error/message")),
            Some(&Value::from(Fault::Oversize.message()))
        );
    }

    fn collected(item: Value) -> Result<Value, Fault> {
        let mut c = Collector::default();
        c.feed_event(json!({"type": "response.completed",
            "response": {"id": "r", "status": "completed", "usage": {}, "output": [item]}}));
        c.finish("m", &Names::default())
    }

    fn call(extra: Value) -> Value {
        let mut base = json!({"type": "function_call", "call_id": "call_1", "name": "f"});
        if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                b.insert(k.clone(), v.clone());
            }
        }
        base
    }

    #[test]
    fn a_collected_call_is_validated_like_a_streamed_one() {
        let ok = collected(call(
            json!({"arguments": "{\"a\":1}", "status": "completed"}),
        ))
        .unwrap();
        assert_eq!(ok["content"][0]["input"], json!({"a": 1}));
        assert!(
            collected(call(json!({"arguments": "{}"}))).is_ok(),
            "status is optional"
        );
        for status in ["incomplete", "in_progress"] {
            assert_eq!(
                collected(call(json!({"arguments": "{}", "status": status}))).unwrap_err(),
                Fault::Truncated,
                "{status}"
            );
        }
        for bad in ["[1]", "1", "{\"a\":"] {
            assert_eq!(
                collected(call(json!({"arguments": bad}))).unwrap_err(),
                Fault::Malformed,
                "{bad}"
            );
        }
        assert_eq!(collected(call(json!({}))).unwrap_err(), Fault::Malformed);
    }

    #[test]
    fn a_streamed_call_with_no_arguments_anywhere_is_malformed_like_a_collected_one() {
        let out = stream(&[added(), done(json!({"status": "completed"})), completed()]);
        assert!(failed_with(&out, Fault::Malformed), "{out:?}");
        assert!(!out.iter().any(|(k, _)| k == "message_stop"));
    }

    /// A JSON object whose text is `len` bytes or a little more.
    fn object_of(len: usize) -> String {
        format!("{{\"a\":\"{}\"}}", "x".repeat(len))
    }

    #[test]
    fn terminal_only_arguments_are_bounded_per_call_and_in_total() {
        let big = object_of(MAX_ARGS_PER_CALL);
        let out = stream(&[added(), done(json!({ "arguments": big })), completed()]);
        assert!(failed_with(&out, Fault::Oversize), "per call");

        let half = object_of(MAX_ARGS_PER_CALL / 2);
        let mut t = ResponsesToAnthropic::new("m", Names::default());
        let mut last = Vec::new();
        for i in 0..(MAX_ARGS_TOTAL / (MAX_ARGS_PER_CALL / 2) + 1) {
            let id = format!("fc_{i}");
            t.feed_event(&json!({"type": "response.output_item.added",
                "item": {"id": id, "type": "function_call", "call_id": id, "name": "f"}}));
            last = t.feed_event(&json!({"type": "response.output_item.done",
                "item": {"id": id, "type": "function_call", "call_id": id, "name": "f",
                         "arguments": half}}));
            if t.ended() {
                break;
            }
        }
        assert!(t.ended(), "the total is bounded");
        assert_eq!(
            last.last().and_then(|e| e.data.pointer("/error/message")),
            Some(&Value::from(Fault::Oversize.message()))
        );
    }

    #[test]
    fn collected_arguments_are_bounded_per_call_and_in_total() {
        let big = object_of(MAX_ARGS_PER_CALL);
        assert_eq!(
            collected(call(json!({ "arguments": big }))).unwrap_err(),
            Fault::Oversize
        );
        let half = object_of(MAX_ARGS_PER_CALL / 2);
        let output: Vec<Value> = (0..(MAX_ARGS_TOTAL / (MAX_ARGS_PER_CALL / 2) + 1))
            .map(|_| call(json!({ "arguments": half })))
            .collect();
        let mut c = Collector::default();
        c.feed_event(json!({"type": "response.completed",
            "response": {"id": "r", "status": "completed", "usage": {}, "output": output}}));
        assert_eq!(
            c.finish("m", &Names::default()).unwrap_err(),
            Fault::Oversize
        );
    }

    /// The stop reason the stream reports and the one collection reports
    /// for the same terminal event.
    fn both_stop_reasons(terminal: Value) -> (String, String) {
        let mut t = ResponsesToAnthropic::new("m", Names::default());
        let streamed = t
            .feed_event(&terminal)
            .iter()
            .find_map(|e| e.data.pointer("/delta/stop_reason").cloned())
            .unwrap();
        let mut c = Collector::default();
        c.feed_event(terminal);
        let collected = c.finish("m", &Names::default()).unwrap()["stop_reason"].clone();
        (
            streamed.as_str().unwrap().to_owned(),
            collected.as_str().unwrap().to_owned(),
        )
    }

    #[test]
    fn streamed_and_collected_stop_reasons_agree() {
        for (terminal, want) in [
            (
                json!({"type": "response.incomplete", "response": {"id": "r", "usage": {}}}),
                "max_tokens",
            ),
            (
                json!({"type": "response.completed",
                       "response": {"id": "r", "status": "incomplete", "usage": {}}}),
                "max_tokens",
            ),
            (
                json!({"type": "response.incomplete", "response": {"id": "r", "usage": {},
                       "incomplete_details": {"reason": "content_filter"}}}),
                "refusal",
            ),
            (
                json!({"type": "response.completed",
                       "response": {"id": "r", "status": "completed", "usage": {}}}),
                "end_turn",
            ),
        ] {
            let (s, c) = both_stop_reasons(terminal.clone());
            assert_eq!((s.as_str(), c.as_str()), (want, want), "{terminal}");
        }
    }

    #[test]
    fn collection_keeps_the_first_terminal_event() {
        let mut c = Collector::default();
        assert!(!c.done());
        c.feed_event(json!({"type": "response.completed",
            "response": {"id": "first", "status": "completed", "usage": {}, "output": []}}));
        assert!(c.done());
        c.feed_event(json!({"type": "response.completed",
            "response": {"id": "second", "status": "completed", "usage": {}, "output": []}}));
        c.feed_event(json!({"type": "response.failed"}));
        assert_eq!(c.finish("m", &Names::default()).unwrap()["id"], "first");
    }
}

// ---------------------------------------------------------------------
// Request shape refusals
// ---------------------------------------------------------------------

#[test]
fn a_tools_value_that_is_not_an_array_is_refused_not_dropped() {
    use serde_json::json;
    let opts = request::RequestOpts {
        model: "m".into(),
        effort: "medium".into(),
        effort_override: false,
        translation: Translation::Openai,
    };
    let msgs = json!([{"role": "user", "content": "hi"}]);
    for bad in [
        json!("read_file"),
        json!({"name": "f"}),
        json!(1),
        json!(true),
    ] {
        let req = json!({"messages": msgs, "tools": bad});
        let names = request::Names::for_request(&req);
        assert_eq!(
            request::to_responses(&req, &names, &opts)
                .unwrap_err()
                .to_string(),
            "Unsupported tools",
            "{bad}"
        );
    }
    for ok in [
        json!({"messages": msgs}),
        json!({"messages": msgs, "tools": null}),
    ] {
        let names = request::Names::for_request(&ok);
        let out = request::to_responses(&ok, &names, &opts).unwrap();
        assert_eq!(out["tools"], json!([]), "{ok}");
    }
}

// ---------------------------------------------------------------------
// SSE framing
// ---------------------------------------------------------------------

mod sse_framing {
    use super::super::stream::{Fault, Line, SseLines};

    /// Every event and the fault, fed in chunks split at `cuts`.
    fn read(raw: &[u8], cuts: &[usize]) -> (Vec<Line>, Option<Fault>) {
        let mut sse = SseLines::default();
        let mut out = Vec::new();
        let mut at = 0;
        for &c in cuts.iter().chain([raw.len()].iter()) {
            let (lines, fault) = sse.feed_until_fault(&raw[at..c]);
            out.extend(lines);
            if fault.is_some() {
                return (out, fault);
            }
            at = c;
        }
        match sse.finish() {
            Ok(last) => {
                out.extend(last);
                (out, None)
            }
            Err(f) => (out, Some(f)),
        }
    }

    fn data(s: &str) -> Line {
        Line::Data(s.to_owned())
    }

    #[test]
    fn a_data_field_without_a_space_is_read() {
        let (lines, fault) = read(b"data:{\"a\":1}\n\ndata:[DONE]\n\n", &[]);
        assert_eq!(fault, None);
        assert_eq!(lines, [data("{\"a\":1}"), Line::Done]);
    }

    #[test]
    fn multi_line_data_is_one_event_joined_by_newlines() {
        let raw = b"event: x\ndata: {\"a\":\ndata: 1}\n\n: a comment\nid: 7\ndata: {}\n\n";
        let (lines, fault) = read(raw, &[]);
        assert_eq!(fault, None);
        assert_eq!(lines, [data("{\"a\":\n1}"), data("{}")]);
    }

    #[test]
    fn line_per_event_streams_without_blank_lines_still_split() {
        let (lines, fault) = read(b"data: {\"a\":1}\ndata: {\"b\":2}\ndata: [DONE]\n", &[]);
        assert_eq!(fault, None);
        assert_eq!(lines, [data("{\"a\":1}"), data("{\"b\":2}"), Line::Done]);
    }

    #[test]
    fn an_event_that_never_becomes_json_is_handed_on_whole() {
        let (lines, _) = read(b"data: not\ndata: json\n\n", &[]);
        assert_eq!(lines, [data("not\njson")]);
    }

    /// A terminal event then invalid UTF-8: the event is delivered however
    /// the bytes were split, and the fault follows it.
    #[test]
    fn events_before_a_fault_survive_every_chunk_boundary() {
        let mut raw = b"data: {\"type\":\"response.completed\"}\n\n".to_vec();
        raw.extend_from_slice(b"data: \xff\xfe\n");
        let whole = read(&raw, &[]);
        assert_eq!(
            whole,
            (
                vec![data("{\"type\":\"response.completed\"}")],
                Some(Fault::Malformed)
            )
        );
        for cut in 1..raw.len() {
            assert_eq!(read(&raw, &[cut]), whole, "cut at {cut}");
        }
    }

    /// `feed` hands back the events before a fault and reports the fault
    /// on the next call.
    #[test]
    fn feed_defers_a_fault_behind_the_events_before_it() {
        let mut sse = SseLines::default();
        let got = sse.feed(b"data: {}\n\ndata: \xff\n").unwrap();
        assert_eq!(got, [data("{}")]);
        assert_eq!(sse.feed(b"data: {}\n\n").unwrap_err(), Fault::Malformed);
        assert_eq!(sse.finish().unwrap_err(), Fault::Malformed);
    }

    #[test]
    fn one_events_data_is_bounded_across_its_lines() {
        let line = format!("data: {}\n", "x".repeat(super::super::MAX_LINE / 2));
        let raw = line.repeat(3);
        let (_, fault) = read(raw.as_bytes(), &[]);
        assert_eq!(fault, Some(Fault::Oversize));
    }
}

// ---------------------------------------------------------------------
// Block cap, refusals and the answering model
// ---------------------------------------------------------------------

mod responses_out {
    use serde_json::{json, Value};

    use super::super::chat::{ChatToResponses, Out};
    use super::super::request::Names;
    use super::super::stream::{Collector, Fault, Line, ResponsesToAnthropic};
    use super::super::MAX_BLOCKS;

    fn text_parts(n: usize) -> Value {
        let content: Vec<Value> = (0..n)
            .map(|i| json!({"type": "output_text", "text": format!("t{i}")}))
            .collect();
        json!({"type": "response.completed", "response": {"id": "r", "status": "completed",
            "usage": {}, "output": [{"type": "message", "content": content}]}})
    }

    fn collected(terminal: Value) -> Result<Value, Fault> {
        let mut c = Collector::default();
        c.feed_event(terminal);
        c.finish("requested", &Names::default())
    }

    /// The stream's error message, if it ended with one.
    fn streamed_fault(n: usize) -> Option<String> {
        let mut t = ResponsesToAnthropic::new("requested", Names::default());
        let mut last = None;
        for i in 0..n {
            for ev in t.feed_event(&json!({"type": "response.output_text.delta",
                "item_id": format!("m{i}"), "content_index": 0, "delta": "x"}))
            {
                last = ev
                    .data
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
        }
        last
    }

    #[test]
    fn both_modes_hold_the_block_cap_at_and_above_it() {
        let blocks = collected(text_parts(MAX_BLOCKS)).unwrap()["content"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(blocks, MAX_BLOCKS);
        assert_eq!(
            collected(text_parts(MAX_BLOCKS + 1)).unwrap_err(),
            Fault::Oversize
        );
        assert_eq!(streamed_fault(MAX_BLOCKS), None);
        assert_eq!(
            streamed_fault(MAX_BLOCKS + 1).as_deref(),
            Some(Fault::Oversize.message())
        );
    }

    #[test]
    fn the_collected_cap_counts_tool_calls_too() {
        let mut t = text_parts(MAX_BLOCKS);
        if let Some(out) = t
            .pointer_mut("/response/output")
            .and_then(Value::as_array_mut)
        {
            out.push(json!({"type": "function_call", "call_id": "c", "name": "f",
                            "arguments": "{}"}));
        }
        assert_eq!(collected(t).unwrap_err(), Fault::Oversize);
    }

    /// A chat stream through both hops, streaming or collected.
    fn through_chat(chunks: &[Value], stream: bool) -> (Vec<Value>, Vec<Out>) {
        let mut chat = ChatToResponses::new("requested", !stream);
        let mut responses = Vec::new();
        let mut caches = Vec::new();
        let lines = chunks
            .iter()
            .map(|c| Line::Data(c.to_string()))
            .chain([Line::Done]);
        for line in lines {
            for o in chat.feed_line(&line).unwrap() {
                match o {
                    Out::Event(e) => responses.push(e),
                    cache => caches.push(cache),
                }
            }
        }
        (responses, caches)
    }

    fn anthropic_stream(events: &[Value]) -> Vec<Value> {
        let mut t = ResponsesToAnthropic::new("requested", Names::default());
        events
            .iter()
            .flat_map(|e| t.feed_event(e))
            .map(|e| e.data)
            .collect()
    }

    fn collect(events: Vec<Value>) -> Value {
        let mut c = Collector::default();
        for e in events {
            c.feed_event(e);
        }
        c.finish("requested", &Names::default()).unwrap()
    }

    fn streamed_text(events: &[Value]) -> String {
        events
            .iter()
            .filter_map(|e| e.pointer("/delta/text").and_then(Value::as_str))
            .collect()
    }

    fn refusal_chunks() -> Vec<Value> {
        vec![
            json!({"id": "c1", "choices": [{"delta": {"role": "assistant", "refusal": "I can't "}}]}),
            json!({"choices": [{"delta": {"refusal": "help with that."}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
        ]
    }

    #[test]
    fn a_chat_refusal_is_the_answer_streamed_and_collected() {
        let (events, _) = through_chat(&refusal_chunks(), true);
        let out = anthropic_stream(&events);
        assert_eq!(streamed_text(&out), "I can't help with that.");

        let (events, _) = through_chat(&refusal_chunks(), false);
        let msg = collect(events);
        assert_eq!(
            msg["content"],
            json!([{"type": "text", "text": "I can't help with that."}])
        );
    }

    #[test]
    fn a_non_string_refusal_is_malformed() {
        let mut chat = ChatToResponses::new("m", false);
        let line = Line::Data(json!({"choices": [{"delta": {"refusal": 1}}]}).to_string());
        assert_eq!(chat.feed_line(&line).unwrap_err(), Fault::Malformed);
    }

    fn model_of_stream(events: &[Value]) -> Value {
        anthropic_stream(events)
            .iter()
            .find_map(|e| e.pointer("/message/model").cloned())
            .unwrap()
    }

    /// The provider's answering model reaches the client — and so the
    /// ledger, which reads it there — while the continuation key keeps
    /// the requested one.
    #[test]
    fn the_answering_model_is_the_providers_and_the_cache_key_the_requested() {
        let chunks = [
            json!({"id": "c1", "model": "provider-model-2026", "choices": [{"delta":
                {"tool_calls": [{"index": 0, "id": "call_1",
                                 "function": {"name": "f", "arguments": "{}"}}]}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
        ];
        let (events, caches) = through_chat(&chunks, true);
        assert_eq!(model_of_stream(&events), "provider-model-2026");
        match caches.as_slice() {
            [Out::Cache { model, .. }] => assert_eq!(model, "requested"),
            other => panic!("{}", other.len()),
        }
        let (events, _) = through_chat(&chunks, false);
        assert_eq!(collect(events)["model"], "provider-model-2026");
    }

    #[test]
    fn a_responses_provider_reported_model_is_the_answering_model() {
        let events = [
            json!({"type": "response.created",
                   "response": {"id": "r", "model": "gpt-5-2026-01-01"}}),
            json!({"type": "response.completed", "response": {"id": "r",
                   "model": "gpt-5-2026-01-01", "status": "completed", "usage": {}}}),
        ];
        assert_eq!(model_of_stream(&events), "gpt-5-2026-01-01");
        assert_eq!(collect(events.to_vec())["model"], "gpt-5-2026-01-01");
    }

    /// No model, or one that is not an identifier, leaves the requested
    /// model in place: provider text never reaches the client.
    #[test]
    fn a_missing_or_hostile_reported_model_falls_back_to_the_requested() {
        for model in [Value::Null, json!("has spaces and text"), json!(7)] {
            let events = [
                json!({"type": "response.created", "response": {"id": "r", "model": model}}),
                json!({"type": "response.completed", "response": {"id": "r",
                       "model": model, "status": "completed", "usage": {}}}),
            ];
            assert_eq!(model_of_stream(&events), "requested", "{model}");
            assert_eq!(collect(events.to_vec())["model"], "requested", "{model}");
        }
    }
}

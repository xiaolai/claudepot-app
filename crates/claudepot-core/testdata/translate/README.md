# Protocol-translation fixtures (captured from clai)

Recorded inputs and outputs of the protocol translation in clai
(`xiaolai/claude-driven-ai-providers`): Anthropic Messages to and from OpenAI
Responses (`clcd`), and Responses to and from chat completions (the Gemini
and local routes, `adapters/`). They are the acceptance data for relay plan
§9, rules R2-5, R2-6 and R2-7.

The expected outputs are what clai's own filters printed. Nothing in an
`*.expect.json` or `*.reject.json` file was written by hand, and none of it
is a specification of what Claudepot should do. Where clai is lossy or wrong,
the fixture records what clai does, and the loss is listed under
[Reviewed exceptions](#reviewed-exceptions--not-goldens).

| | |
|---|---|
| Source | `xiaolai/claude-driven-ai-providers` at `8da773c2c74d491795245155ab14f1d791532612` (clean tree) |
| Suites | `tests/clcd-protocol.sh`, `tests/clcd-regression.sh`, `tests/bridge-api.sh`, `tests/chat.sh` |
| Captured with | `jq-1.7.1-apple`, bash 3.2, macOS |
| Cases | 40: 32 in `responses/`, 8 in `chat/`; 28 expect, 12 reject; 38 from a suite, 2 derived |
| Assertions | 60 suite assertions, each re-executed against the captured output |

## Layout

| Path | Contents |
|---|---|
| `responses/` | Anthropic Messages to OpenAI Responses and back: Codex, OpenAI and Grok through `clcd` |
| `chat/` | Responses to chat completions and back: the Gemini and local routes through `adapters/` |
| `filters/` | The clai programs the outputs came from, extracted verbatim from the source commit (one exception, below) |
| `manifest.json` | Source commit, jq version, counts, and one row per case |
| `capture.sh` | Rebuilds everything above from a clai checkout, and fails if anything differs |
| `.gitattributes` | `* -text`, so a Windows checkout does not rewrite the CRLF and LF bytes in the `.sse` files |

`filters/emulated-metadata-lookup.jq` is the one program that is not clai
code. clai looks up Gemini replay metadata in bash (`prepare_chat`,
clcd:327-343): one file per call, named by sha256 of model and call id. The
filter reproduces that lookup over the captured cache entries so that the
chain can run as filters. It is labelled as an emulation in its header. The
upstream requests it produces were checked against the requests clai actually
sent; see [Cross-check](#cross-check-against-the-running-bridge).

## Files per case

A case is `<family>/<case>.<suffix>`.

| Suffix | Meaning |
|---|---|
| `.input.json` | The request fed to the first filter: an Anthropic request, or for `chat/roundtrip-chat-sh-signature` the suite's normalized Responses request |
| `.events.sse` | A provider stream, byte for byte (CRLF kept where the suite used CRLF) |
| `.reply.events.sse`, `.crlf.events.sse` | Further streams in the same case |
| `.cache.jsonl` | Private cache state a case starts from (one cache entry per line, write order) |
| `.upstream-headers.txt`, `.upstream-body.json` | A provider error response |
| `.expect.json` | Expected output of a case that succeeds |
| `.reject.json` | Expected output of a case that must be refused; at least one step has `kind: "reject"` |

Every JSON file is `jq -S` output: pretty-printed, keys sorted. The `.sse`,
`.txt` and `.jsonl` inputs are kept raw.

### `expect.json` / `reject.json`

```json
{
  "case": "stream-text-and-tool",
  "family": "responses",
  "kind": "expect",
  "origin": "suite",
  "covers": "one-line description",
  "source": { "repo": "...", "commit": "...", "lines": ["tests/clcd-protocol.sh:22-34"] },
  "steps": [ ... ],
  "assertions": [ ... ],
  "notes": [ "optional" ]
}
```

`origin` is `suite` when the input comes from a clai test, and `derived` when
it was assembled from suite lines to cover a case the suites do not (the
cited lines say which).

`steps` is the pipeline, in order. Each step names its `input`, which is
either `{"file": "<fixture file>"}`, `{"step": "<earlier label>"}`, or
`null` (for `jq -n`).

| `kind` | Fields |
|---|---|
| `json` | `filter`, `params`, `output` (the parsed JSON value) |
| `sse` | `filter`, `params`, `output_raw` (exact bytes), `output_lines` (each line parsed as `{event}`, `{data}`, `{cache}` or `{line}`; blank lines dropped) |
| `reject` | `filter`, `params`, `exit_code`, `error` (the jq message, cut as clcd cuts it at clcd:498), `partial_output_raw`, `partial_output_lines` (what a client had already received) |
| `glue` | `glue` (the shell transform and where clai does it), `output_raw` |

`params` records the jq invocation: `arg`, `argjson`, `slurpfile` (the slurped
content, inline) and `flags`. A filter run with those parameters on that input
must give exactly that output.

`assertions` are the suite's checks. `text` is copied verbatim from `source`
(file:line); `capture.sh` refuses to run if the text is not on that line.

| `subject` | How `text` is run |
|---|---|
| `json` | `jq -e <text>` on the step output; `args` holds any `--arg` |
| `json@<path>` | `jq -e <text>` on `<path>` of the step output |
| `data` | the suite's own form: `sed -n 's/^data: //p' \| jq -se <text>` |
| `grep` | `grep -q -- <text>` on `output_raw` |
| `rejected` | `text` is the suite command that must fail; the step is a reject |
| `same` | the step output is byte-identical to `other_step` |
| `equiv` | `text` is a shell test, a status code or a file lookup that cannot run on a fixture. `equivalent` is the jq that is run instead; `args.equivalent_inputs` names any other steps it reads |

## How the fixtures were made

```bash
crates/claudepot-core/testdata/translate/capture.sh <clai-checkout> --update   # write
crates/claudepot-core/testdata/translate/capture.sh <clai-checkout>            # verify
```

`capture.sh` is the whole generator; nothing was edited by hand afterwards.
It:

1. Extracts each filter from the clai checkout: the `request.jq` and
   `stream.jq` heredocs from `clcd` by their opening line, and the inline jq
   programs from `clcd` and the test suites by line number and prefix. The
   two adapter files are copied as they are.
2. Takes each suite input from its line in the suite, and runs `printf`
   formats through `printf` exactly as the suite does.
3. Runs each step with the parameters clai uses. The `bridge-*` and
   `roundtrip-gemini` cases chain the steps the way `clcd`'s `handle()` does:
   names (clcd:495), reverse (clcd:496), request (clcd:497), `prepare_chat`
   (clcd:327-343), `normalize_chat` (clcd:344-359), stream (clcd:509) or
   collect (clcd:530-537). The upstream-error cases repeat the bash in
   `upstream_error` (clcd:435-447): key substitution, the jq mapping, the
   Retry-After awk and its regex guard.
4. Runs every assertion against the captured output and stops on the first
   failure.
5. Writes into a temporary directory and compares it with the committed files
   using `diff -r`.

Exit status: 0 when identical, 1 on any difference, 2 when capture itself
fails (missing tool, moved source line, failed assertion, a reject case that
was accepted). The script warns when the checkout is not at the pinned commit
and appends `-dirty` to the recorded commit when its tree has changes.

Source line numbers are those of the pinned commit. After a clai update, a
moved line makes `capture.sh` stop with an error rather than extract the wrong
text. Fix the reference, rerun with `--update`, and review the diff: it is
the record of what changed in clai's translation.

The bridge parameters (selected model, `effort medium`, `effort_override
false`, empty role mapping outside the mapping case) are the ones
`bridge-api.sh` actually runs with. They were read from an instrumented copy
of that suite that saved every upstream request body. The copy lived in a
temporary directory; the clai checkout was not modified.

`upstream-error-auth-redaction` uses `00000000000000000000000000000000` as
the API key. `bridge-api.sh` generates a fresh random one with `openssl rand
-hex 16`; a fixture needs a fixed value, and an all-zero placeholder cannot be
mistaken for a real one. Every other input is the suites' synthetic data. No
fixture contains a credential.

### Cross-check against the running bridge

The filter-level chains were compared (`jq -S`, exact) with the request
bodies the real `clai` bridge sent to its mocked upstream during
`bridge-api.sh`:

| Captured step | Matched the bridge's upstream request for phase |
|---|---|
| `responses/bridge-openai-text` `request` | `responses` |
| `responses/bridge-grok-text` `request` | `grok` |
| `responses/bridge-openai-refusal` `request` | `refusal` |
| `responses/bridge-openai-mapping` `request` | `mapping` |
| `chat/roundtrip-gemini` `upstream_request` | `first` |
| `chat/roundtrip-gemini` `replay_upstream_request` | `replay` |
| `chat/bridge-local-text` `upstream_request` | `local` |
| `chat/roundtrip-gemini` `cache` | the private cache files the bridge wrote |

All eight matched. So the emulated metadata lookup and the chaining give what
clai gives.

## Cases

### `responses/`

| Case | Covers | Source |
|---|---|---|
| `request-history-tools-image-thinking` | system to `instructions`; mid-turn system to `developer`; user image to an `input_image` data URL; thinking block dropped; `tool_use` to `function_call` with a wire name; an error `tool_result` to a `[Tool error]` output; `tool_choice` and `disable_parallel_tool_use` | clcd-protocol.sh:16-19 |
| `request-adaptive-effort` | `output_config.effort` becomes `reasoning.effort` | clcd-protocol.sh:48 |
| `request-effort-override` | an explicit bridge effort overrides the request | clcd-protocol.sh:49 |
| `request-model-role-{opus,claude-opus-4-1,fable,claude-fable-5-1,sonnet,claude-3-5-sonnet,haiku,raw-provider-id}` | short and full Claude model names resolve through the role mapping; provider ids pass through (8 cases) | clcd-protocol.sh:58-61 |
| `stream-text-and-tool` | Responses SSE to Anthropic SSE: text, a function call with the wire name reversed, block indices, cached tokens split out of `input_tokens` | clcd-protocol.sh:22-34 |
| `stream-refusal-crlf` | CRLF framing; refusal text becomes text; `[DONE]` tolerated | clcd-protocol.sh:51-52 |
| `stream-terminal-incomplete` | `response.incomplete` becomes `stop_reason: max_tokens` | clcd-protocol.sh:53-56 |
| `stream-terminal-failed`, `stream-terminal-error` | `response.failed` and `error` become an Anthropic `error` event | clcd-protocol.sh:53-56 |
| `bridge-openai-text` | OpenAI route: `store:false`, `max_output_tokens`, streamed text back | bridge-api.sh:41-43, 129-130 |
| `bridge-grok-text` | Grok route: as OpenAI, with `reasoning` removed | bridge-api.sh:41-42, 152 |
| `bridge-openai-mapping` | `claude-opus-4-1` resolves through a provider profile mapping; non-streaming collection | bridge-api.sh:40-43, 127, 180 |
| `bridge-openai-refusal` | a refusal in `response.completed` is collected as text | bridge-api.sh:54, 116 |
| `bridge-codex-check` | `clcd --check` probe on the Codex route, collected | clcd-regression.sh:45, 57; clcd:572-573 |
| `upstream-error-quota` | 429 becomes `rate_limit_error`; `Retry-After` forwarded | clcd-regression.sh:40, 70-71 |
| `upstream-error-context` | context-length 400 becomes `invalid_request_error` "prompt is too long:" | clcd-regression.sh:41, 73 |
| `upstream-error-auth-redaction` | 403 echoing the key: status kept, key redacted, `authentication_error` | bridge-api.sh:34-35, 119-121 |
| `reject-unsupported-content` | an unknown content block is refused | clcd-protocol.sh:20-21 |
| `reject-control-{temperature,top_p,top_k,output_format}` | sampling and format controls are refused, not ignored (4 cases) | clcd-protocol.sh:42-45 |
| `reject-stop-sequences` | non-empty `stop_sequences` are refused | clcd-protocol.sh:46-47 |
| `reject-truncated-stream` | a stream with no terminal event is an error, after `message_start` was sent | clcd-protocol.sh:35 |
| `exception-unfinished-tool-call` | **derived, not a golden**: see the exceptions below | clcd-protocol.sh:23, 28, 31 |

### `chat/`

| Case | Covers | Source |
|---|---|---|
| `roundtrip-chat-sh-signature` | Gemini chat stream with two parallel calls, a fragmented argument and a thought signature, to Responses events plus cache entries, to the replay request: `extra_content` restored per call, both calls in one assistant message; CRLF input gives byte-identical output | chat.sh:6-19, 24-25 |
| `roundtrip-gemini` | the full R2-7 chain through clcd, four hops: (1) Anthropic request to Responses to chat request; (2) provider stream to private cache plus Anthropic message, both collected and streamed; (3) `tool_result` replay to the next chat request, with call ids, arguments and `extra_content` restored; (4) the final text reply | bridge-api.sh:57-68, 73, 80-99, 131-145 |
| `bridge-local-text` | local route: no metadata lookup, no `reasoning_effort` | bridge-api.sh:68, 73, 137-139 |
| `reject-missing-metadata` | a Gemini replay without its metadata is refused | chat.sh:20-21 |
| `reject-replay-missing-metadata` | the same replay under another model finds no entry (the cache is keyed by model and call id) and is refused with HTTP 400 before any upstream call | bridge-api.sh:141-143, 160-162; clcd:502-503 |
| `reject-truncated-stream` | a chat stream that ends before a finish reason | chat.sh:22 |
| `reject-malformed-stream` | unparseable SSE data; clcd reports a Chat stream problem (502) | bridge-api.sh:45, 107-114; clcd:459-465 |
| `reject-incomplete-tool-call-arguments` | **derived**: argument fragments that never complete are refused at the finish reason | chat.sh:7, 9-10 |

Call ids pass through verbatim in every case (`call1`, `call_one`,
`gemini_call`, `parallel_call`): clai never rewrites them. R2-7's id
bijection and its id-swap mutation test belong to Claudepot's test over
these fixtures. The fixtures supply the pairs: each `tool_use.id` in a
message step and the `tool_call_id` / `tool_calls[].id` in the next request.

## Reviewed exceptions — not goldens

These are clai behaviours that the fixtures record but that Claudepot must
not copy silently. Under R2-6 each is either a declared capability loss for
that backend or a refusal. Lines are at the pinned commit. `clcd:N` lines
inside `request.jq` / `stream.jq` are lines of `clcd` itself, not of the
extracted files (those start with a one-line header).

| # | Behaviour | Where | Seen in |
|---|---|---|---|
| E1 | `thinking` and `redacted_thinking` blocks in history are dropped without notice. The relay plan cited clcd ~236; the line is **249** (236-237 is the image branch) | clcd:249 | `request-history-tools-image-thinking` |
| E2 | Every `response.incomplete` becomes `stop_reason: max_tokens`, whatever the reason (content filter, max output, other). On the chat path `finish_reason: "length"` becomes `response.incomplete` first | clcd:316 (stream), clcd:534 (collect); chat-stream.jq:5-6 | `stream-terminal-incomplete` |
| E3 | Images inside a `tool_result` are moved out into a separate `user` message placed after the tool output; the tool output keeps only the text | clcd:247-248 | none: no suite has an image inside a tool result |
| E4 | A `function_call` that never gets `response.output_item.done` is still emitted as a finished `tool_use` with `input: {}` (or the partial argument text) and `stop_reason: tool_use`. `response.completed` closes every open block | clcd:310-316 | `exception-unfinished-tool-call` (it asserts clai's behaviour; Claudepot should refuse this input) |
| E5 | On the chat path, unfinished argument JSON fails as a jq parse error (`fromjson`) and not as "Incomplete chat tool call". clcd's `stream_problem` matches none of its patterns, so the client gets the generic "malformed or interrupted" message. The refusal is right; the diagnosis is lost | chat-stream.jq:40; clcd:459-465 | `reject-incomplete-tool-call-arguments`, `reject-malformed-stream` |
| E6 | `response.failed` and `error` events are replaced by a fixed "Codex returned an error."; the provider's message is dropped | clcd:317-318 | `stream-terminal-failed`, `stream-terminal-error` |
| E7 | Unknown Responses event types are ignored, including reasoning items, so provider reasoning never reaches the client as thinking | clcd:319 | none |
| E8 | `count_tokens` is an estimate, JSON length / 3, not a tokenization | clcd:488-490 | none (not a translation filter) |
| E9 | Gemini replay metadata is keyed only by sha256(model, call id), in one per-user cache directory: no device or target scoping (R2-5 requires device, target fingerprint, model, call id), and the last write wins | clcd:324-339, 350-355 | `roundtrip-gemini`, `reject-replay-missing-metadata` |
| E10 | `disable_parallel_tool_use` becomes `parallel_tool_calls: false`, which the chat adapter refuses, so such a request cannot use the Gemini or local route at all | clcd:269; chat-request.jq:14 | `request-history-tools-image-thinking` (Responses side only) |
| E11 | The upstream key is redacted by substituting the live key in bash before any JSON handling. A translator that does not hold that exact string would pass it through | clcd:438 | `upstream-error-auth-redaction` |
| E12 | Images are accepted only in `user` messages; one in an `assistant` or `system` message is refused as unsupported content | clcd:236-238 | none |

## Not captured

| What | Why |
|---|---|
| ShellCheck, bash syntax, `chunks` / disconnect, header timeout, the refresh lock and RPC ids | process plumbing in clcd and clai, not translation |
| a Responses (Codex/OpenAI) tool round trip: `tool_use` to `tool_result` to the next request | none of the suites has one; the Responses tool path is covered one hop at a time (`request-history-tools-image-thinking`, `stream-text-and-tool`) |
| images inside a `tool_result` (E3) | no suite input; recorded as an exception only |
| `adapters/model-mapping.jq` | exercised by `tests/models.sh`, not by the four suites in scope |
| the credential checks in `bridge-api.sh` (`Authorization` present or absent per route, no key leaking to other providers) | bridge and environment behaviour, not visible in a filter's input or output |
| the streaming variants of the Gemini first, replay and local phases | the same filters as the collected variants; `roundtrip-gemini` runs the stream filter on hop 2 and checks it against the collected message |

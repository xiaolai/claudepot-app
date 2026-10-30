# Provider upstream watch — the surfaces translated routes depend on

The sibling of `cc-upstream-watch.md`, for the other side of
`relay::translate`: what the **providers** must keep doing for a
translated route to work. `cargo xtask cc-drift` does not read this file
— it searches Claude Code's changelog, and a provider change never
appears there — so this list is checked by hand, and its rows are
deliberately few.

Each row names the module, the provider behaviour it relies on, and the
check that decides whether it still holds. **A row is verified only by a
live request** against the provider; the fixtures in
`crates/claudepot-core/testdata/translate/` were captured from clai's
suites (clai `8da773c2`, in plan phase P-D), which are synthetic, so they pin
clai's behaviour and say nothing about the provider's.

| Surface | Module | Depends on | Check | Last verified |
|---|---|---|---|---|
| Codex Responses endpoint | `relay::translate::handler` | `POST https://chatgpt.com/backend-api/codex/responses` with `chatgpt-account-id`, `openai-beta: responses=experimental`, `originator: codex_cli_rs`; a 401 for an expired token | one Bash tool round trip through a `--translate codex` route; compare the header set with the installed `codex` binary's (`strings` on it) | **never** — no relay Codex login yet |
| Codex token source | `relay::translate::codex` | `codex app-server` over stdio answering `getAuthStatus` (with `refreshToken`) in the `CODEX_HOME` given; a ChatGPT JWT carrying the account id claim | `CODEX_HOME=~/.claudepot/relay-codex codex app-server` by hand, then the module's own tests; a renamed method or claim is a protocol error, never a fallback | **never** live; the protocol was read from clai's `clcd` |
| Responses streaming events | `relay::translate::stream` | `response.created`, `content_part.added/done`, `output_text.delta`, `refusal.delta`, `function_call_arguments.delta`, `output_item.added/done`, `completed` / `incomplete` / `failed` | one streamed text turn and one tool turn on OpenAI and xAI; an event name the port does not know is ignored, so a renamed delta shows up as an empty reply and a renamed terminal event as a truncated-stream error | **never** — no vendor key on the build host |
| Gemini thought signatures | `relay::translate::chat` | Gemini's OpenAI-compatible Chat Completions returning `extra_content.google.thought_signature` per tool call, and requiring it back on replay | two parallel calls, then their results, on a live Gemini key; a replay the provider accepts without the signature would mean the refusal is now needless | **never** |
| Chat Completions streaming | `relay::translate::chat` | `choices[0].delta.content` / `tool_calls[]` with `index`, `finish_reason`, `usage` on the last chunk, `data: [DONE]` | one streamed tool turn on any Chat provider | **never** |

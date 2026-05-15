---
plugin: grill
version: 1.2.5
date: 2026-05-15
target: branch `shared-memory-sessions-live` vs `main` in /Users/joker/github/xiaolai/myprojects/claudepot-app
style: Paranoid Mode (Edge Case Gauntlet)
addons:
  - Principle violations
  - Assumptions audit
  - Compact & optimize
agents:
  - architecture
  - error-handling
  - security
  - testing
  - edge-cases
---

# Grill Report — `shared-memory-sessions-live` branch

Scope: 7 commits / ~4,400 LOC of new code adding cross-harness shared
memory + Codex transcript parsing + an MCP server on top of a v3 → v4
schema migration. Reviewed against `main`.

## Synthesis

Findings are deduplicated across agents. When a finding was raised by
multiple agents, the strongest evidence is kept and the agents are
cross-cited in parentheses.

### `[CRITICAL]` / `[HIGH]`

#### H1 — Tracing subscriber may write to stdout, breaking MCP framing

- **File**: `crates/claudepot-cli/src/commands/mcp.rs:47`, plus
  `crates/claudepot-cli/src/main.rs:895-897`
- **Observation**: `tracing::info!(db = …, "claudepot mcp memory-server starting")` runs before `server.serve(stdio())`. CLI's main installs a subscriber via `tracing_subscriber::fmt()` whose default writer is **stdout**. Any `info!`/`warn!` from `apply_schema`, `indexer`, or our own `mcp.rs` will land on stdout, interleaved with JSON-RPC frames — corrupting the MCP protocol. The existing `stdout_only_emits_jsonrpc_frames` integration test hides this by setting `RUST_LOG=warn`, which silences the `info!` but does NOT verify the writer target.
- **Severity**: `[HIGH]`
- **Evidence**: `tracing::info!(…)` at line 47; CLI subscriber init at `main.rs:895-897`; integration test override at `tests/mcp_memory_cli.rs:61` (`env("RUST_LOG", "warn")`).
- **Proposed change**: In `mcp::run`, install (or assert) a `tracing_subscriber::fmt().with_writer(std::io::stderr).init()` *before* constructing the server. Re-run `stdout_only_emits_jsonrpc_frames` **without** the `RUST_LOG=warn` override.
- **Tradeoff**: Five-line subscriber bootstrap; eliminates a silent corruption surface. The trade is that the test gets a tiny bit more stderr noise.
- **Cross-cited by**: architecture, error-handling.

#### H2 — `matches!(...)` no-op assertions in 6 negative-path tests

- **File**: `crates/claudepot-core/src/codex_session/tests.rs:36,52,202`; `crates/claudepot-core/src/shared_memory/durable.rs:741,878`; `crates/claudepot-core/src/shared_memory/read.rs:284`
- **Observation**: Six tests use bare `matches!(err, Variant)` without an `assert!` wrapper. `matches!` returns `bool` — without `assert!`, the call is a discarded expression. The tests pass even when the error variant is wrong. These are exactly the tests gating the public error API contract (MissingSessionMeta / InvalidScope / DecisionNotFound / NotIndexed).
- **Severity**: `[HIGH]`
- **Evidence**: e.g. `tests.rs:36`: `matches!(err, CodexError::MissingSessionMeta { .. });` — no `assert!`.
- **Proposed change**: Wrap every site in `assert!(matches!(...))`. Add a CI grep gate (`grep -rn '^\s*matches!' crates/claudepot-core/src/`) that fails if a bare `matches!` line appears outside expressions.
- **Tradeoff**: Five minutes of edits; the test matrix actually starts protecting what it claims to protect.
- **Cross-cited by**: testing (F1).

#### H3 — `shared_memory_index` / `shared_memory_mcp` feature flags are absent

- **File**: workspace-wide
- **Observation**: The plan says both flags should default *off* until the work is stable. The diff has zero `#[cfg(feature = ...)]` gates, no env var check, no runtime config switch. Every existing user's `~/.claudepot/sessions.db` will run the v4 migration on next launch — and once at v4 with `_min_compatible_version='4'` written, older binaries either silently wipe the cache (pre-this-plan v3) or refuse to migrate (post-this-plan downgrade guard). The MCP subcommand registers unconditionally in `main.rs`.
- **Severity**: `[HIGH]`
- **Evidence**: `grep -rn 'shared_memory_index\|shared_memory_mcp' crates/` returns zero hits in source.
- **Proposed change**: Two options. **(a)** Implement the flag as a Cargo feature on `claudepot-core` and gate `pub mod shared_memory` + the v4 DDL block + the `Mcp` clap variant on it. Default off. **(b)** Commit to v4 being unconditional, document the migration as one-way in release notes, and remove the feature-flag claim from the plan. Mixing the two stances — code says "go" while plan says "off by default" — is worse than either.
- **Tradeoff**: (a) is a half-day of plumbing. (b) is honest but locks in the migration. The plan's framing was (a); without code, the framing is misleading.
- **Cross-cited by**: testing (F2).

#### H4 — `backfill_codex` has zero production callers

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:30-105`
- **Observation**: The Codex indexer is fully built and tested but never called from `claudepot-cli` or `src-tauri`. The MCP server opens `SessionIndex`, queries via `search` / `read_locator` / `durable::*`, but `exchanges` and `tool_calls` stay empty until something runs `backfill_codex`. `claudepot_search_memory` will silently return `{hits: [], has_more: false}` in production. The MCP integration test only round-trips `remember` (durable path), not `search`.
- **Severity**: `[HIGH]`
- **Evidence**: `git grep backfill_codex` → zero non-test callers.
- **Proposed change**: Add a `claudepot codex-index` CLI verb (or fold into `claudepot session rebuild-index`) that invokes `backfill_codex`. Add a smoke integration test: stage a Codex rollout in a temp `CODEX_HOME`, run index, run `claudepot_search_memory` via MCP, assert a hit.
- **Tradeoff**: A few hours. The alternative — shipping the half-feature — guarantees a "search doesn't find anything" support thread.
- **Cross-cited by**: testing (F3).

#### H5 — `sms::search().unwrap_or_default()` silently swallows SQL errors

- **File**: `crates/claudepot-cli/src/commands/mcp.rs:289`
- **Observation**: Every other tool method in `mcp.rs` does `match … { Ok(_) => …, Err(e) => to_json(&error(…)) }`. `claudepot_search_memory` uniquely does `let hits = sms::search(…).unwrap_or_default();`. A `database is locked`, `FTS5 corruption`, or any other SQL error is indistinguishable from "no matches" to the LLM client.
- **Severity**: `[HIGH]`
- **Evidence**: Line 289.
- **Proposed change**: Replace with the standard `match` pattern. Also add `tracing::warn!(error = %e, "search failed")` so an operator looking at stderr can diagnose.
- **Tradeoff**: One pattern flip. The MCP client gets an actionable error instead of false-negative behavior.
- **Cross-cited by**: error-handling.

#### H6 — Parse-failure cache stickiness on re-indexing

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:243-258`, `crates/claudepot-core/src/codex_session/parser.rs:284-291`
- **Observation**: When `upsert_codex_session` fails on parse, the failure is pushed to `stats.failed` and processing continues. But the previously-indexed `sessions` row + `exchanges` rows + FTS rows for that file are *not* removed — search keeps returning the old snippets pointing at the now-corrupt file. Worse: the parser's `EventIter` silently terminates on mid-stream IO errors (`Err(_) => done = true`) — the parser returns a partial conversation as if complete, the indexer commits the partial result, and the staleness triple `(size, mtime_ns, inode)` is captured *before* parsing, so on subsequent runs the file looks "unchanged" and is skipped. Partial state becomes sticky.
- **Severity**: `[HIGH]`
- **Evidence**: `indexer.rs::upsert_codex_session` returns Err before any DELETE; `parser.rs:284-291` discards `read_line` errors; staleness tuple captured at walk time in `walk_dir_recursive` line 195.
- **Proposed change**: Two changes. **(1)** In `parser.rs`, surface partial-parse signals — return `(CodexConversation, ParseDiagnostics { malformed_lines, truncated_by_io })`. **(2)** In `indexer.rs`, when `truncated_by_io`, refuse to stamp the staleness triple (so next backfill re-tries); when `upsert_codex_session` fails on a previously-indexed file, `DELETE FROM sessions WHERE file_path=? AND source_kind='codex'` to clear the cascade.
- **Tradeoff**: Slightly more state to thread. Eliminates two distinct silent-corruption surfaces.
- **Cross-cited by**: error-handling, edge-cases (#1, #2).

#### H7 — MCP errors collapse all categories into one indistinguishable shape

- **File**: `crates/claudepot-cli/src/commands/mcp.rs:559-567`
- **Observation**: Every error is returned as a *successful* tool result containing `{schema_version, error: "<stringified>"}`. The MCP protocol provides `CallToolResult.isError: true` for transport-level error signaling, and rmcp's `Result<CallToolResult, ErrorData>` shape lets a handler distinguish 400-style (bad input) / 404-style (not found) / 409-style (decision not found) / 500-style (sql) errors. The current shape collapses all four into one. LLM clients will treat every error response as a successful tool call.
- **Severity**: `[HIGH]`
- **Evidence**: `ErrorPayload` struct + every tool method that uses `to_json(&error(…))`.
- **Proposed change**: Either (a) migrate to `Result<CallToolResult, ErrorData>`, or (b) at minimum add `error_code: String` (e.g. `invalid_scope`, `decision_not_found`, `locator_not_indexed`, `sql_error`) to `ErrorPayload`. Pipe every error string through `redact_apply` before crossing the boundary.
- **Tradeoff**: (a) is heavier; (b) is a minimum viable fix that preserves the current signature.
- **Cross-cited by**: error-handling.

### `[MEDIUM]`

#### M1 — `read_locator_bounded.truncated` flag is wrong on the boundary AND in the docstring

- **File**: `crates/claudepot-core/src/shared_memory/read.rs:87-126`
- **Observation**: Two distinct bugs at the same site. **(a)** `truncated = body.len() >= max_bytes` uses `>=`, so an exactly-fitting read is wrongly flagged truncated. **(b)** The docstring says "max_bytes is honored *after* `redaction::apply` runs"; the implementation evaluates `truncated` *before* redaction. Redaction can shrink (`sk-ant-oat01-Abcdef…` → `sk-ant-***-Foo`) or grow (`PathStrategy::Hash`) the body. The returned `body` length and the returned `truncated` flag can disagree.
- **Severity**: `[MEDIUM]`
- **Evidence**: Line 117 (`body.len() >= max_bytes`); line 87-88 docstring; redaction `apply` in `redaction.rs:11`.
- **Proposed change**: Pick one stance and align. Recommended: keep the byte cap pre-redaction (it's the DoS mitigation), use `>` instead of `>=`, and update the docstring to say "max_bytes caps the *pre-redaction* read; the redacted body may be shorter."
- **Tradeoff**: Three-line fix. Truth in advertising.
- **Cross-cited by**: architecture (#11), testing (F5), edge-cases (#6).

#### M2 — Migration validation rollback uses `Sql(QueryReturnedNoRows)` — misleading

- **File**: `crates/claudepot-core/src/session_index/mod.rs:561-569`
- **Observation**: When the v4 table-count validation fails, the code returns `Err(SessionIndexError::Sql(rusqlite::Error::QueryReturnedNoRows))`. This conflates a real "row not found" condition with "migration produced the wrong table set." A downstream caller pattern-matching `QueryReturnedNoRows` would silently swallow this. Worse, the existing `is_corrupt_error` quarantine logic keys off rusqlite variants; if anyone teaches it that `QueryReturnedNoRows` means "rebuild the DB", a transient migration glitch triggers cache nuking.
- **Severity**: `[MEDIUM]`
- **Evidence**: Line 569.
- **Proposed change**: Add `SessionIndexError::MigrationValidationFailed { expected: usize, found: usize, missing: Vec<String> }`. Compute the missing names by re-querying `sqlite_master`. Log at `tracing::error!` before returning.
- **Tradeoff**: One enum variant + a small re-query in the failure path.
- **Cross-cited by**: architecture (#7), error-handling, edge-cases (#4).

#### M3 — `snippet_text` written unredacted; comment claims pre-redacted

- **File**: `crates/claudepot-core/src/shared_memory/schema.rs:57` (`-- pre-redacted preview`); `crates/claudepot-core/src/shared_memory/indexer.rs:416-431`
- **Observation**: The schema's column comment says `snippet_text TEXT NOT NULL,                -- pre-redacted preview`. But `indexer.rs::build_snippet` takes the first 240 chars of raw `user_text` + `assistant_text` and writes verbatim. The current emission path (`search::search`) redacts on read, but a future caller that reads `exchanges.snippet_text` directly (CLI dump, debug UI, backup tool) gets raw tokens.
- **Severity**: `[MEDIUM]`
- **Evidence**: schema.rs:57 comment vs indexer.rs:416-431 code.
- **Proposed change**: Pre-redact in `build_snippet` so the docstring becomes true and direct readers are safe. Removes the next-developer footgun.
- **Tradeoff**: Tiny perf cost on indexing; eliminates a class of latent leak.
- **Cross-cited by**: security (#3g).

#### M4 — Symlink/path-containment hole in `read_locator`

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:148-198`; `crates/claudepot-core/src/shared_memory/read.rs:101-114`
- **Observation**: The path-containment claim "file_path must be in `sessions` table" is enforced — but the *indexer* doesn't canonicalize or reject symlinks. `walk_dir_recursive` uses `entry.metadata()` (follows symlinks), so a symlink under `$CODEX_HOME/sessions/<...>/poison.jsonl` pointing at `/etc/passwd` or `~/.config/Claude/.credentials.json` gets indexed → its path lands in `sessions` → readable via MCP `read_conversation`.
- **Severity**: `[MEDIUM]`
- **Evidence**: `walk_dir_recursive` uses `entry.metadata()` not `symlink_metadata` (line 153); `indexer.rs:181` writes `path.to_string_lossy()` un-canonicalized.
- **Proposed change**: In `walk_dir_recursive`, check `entry.file_type().is_symlink()` and skip. Independently: in `read::read_locator_bounded`, canonicalize the path before `File::open` and assert it lives under the expected root.
- **Tradeoff**: Symlinks under the sessions root become invisible. That's a deliberate trust posture; flag in the changelog.
- **Cross-cited by**: security (#2), edge-cases (#18).

#### M5 — Unbounded `read_line` allows OOM via crafted JSONL

- **File**: `crates/claudepot-core/src/codex_session/parser.rs:266`
- **Observation**: `BufReader::read_line(&mut self.buf)` has no max-line cap. A malicious rollout with no `\n` (or one multi-GB line) hangs/OOMs the indexer. Risk vector requires landing a file under `$CODEX_HOME/sessions/` (same as the symlink concern), but the indexer runs inside a transaction — a single bad file can hold up every other file's writes.
- **Severity**: `[MEDIUM]`
- **Evidence**: line 266; no `.take(N)` wrapper.
- **Proposed change**: Wrap with `(&mut reader).take(MAX_LINE_BYTES as u64)` (1 MiB is three orders above any real Codex line), drain to next `\n` on oversized lines, log at WARN.
- **Tradeoff**: Trivial code; eliminates a DoS surface.
- **Cross-cited by**: security (#5), edge-cases (#20).

#### M6 — `version_less_than` lex fallback breaks at v10

- **File**: `crates/claudepot-core/src/session_index/mod.rs:591-598`
- **Observation**: Two-arm `parse::<u32>` then lex fallback. If a future binary writes `_min_compatible_version = "10"`, every current v4 binary's guard compares `"4" < "10"` lexicographically (which is true — `"4"` > `"1"` lex, so actually it's false — `"4" < "10"` is FALSE in lex, meaning the v4 binary thinks it's *not less than* "10" and proceeds with the migration). The fallback inverts the safety story.
- **Severity**: `[MEDIUM]`
- **Evidence**: lines 593-597.
- **Proposed change**: On parse failure of either side, return `true` (treat unknown as "I cannot reason; refuse to migrate"). Reject non-numeric writes at write time.
- **Tradeoff**: Conservative; a corrupt marker blocks migration loudly instead of silently bypassing the guard.
- **Cross-cited by**: edge-cases (#5).

#### M7 — `RedactionPolicy::default()` doesn't cover emails or env assignments

- **File**: `crates/claudepot-cli/src/commands/mcp.rs:48-51`
- **Observation**: Default policy only masks `sk-ant-*`. The MCP module doc claims "raw secrets are redacted in snippets" — overpromise. Lines like `OPENAI_API_KEY=sk-proj-...`, `export GITHUB_TOKEN=ghp_...`, or `aws_secret_access_key="..."` pass through verbatim into search snippets and `read_conversation` bodies.
- **Severity**: `[MEDIUM]`
- **Evidence**: `policy: Arc::new(RedactionPolicy::default())`; `redaction.rs:43-48` shows `emails: false, env_assignments: false`.
- **Proposed change**: Construct a stricter policy at the MCP boundary: `RedactionPolicy { emails: true, env_assignments: true, ..Default::default() }`. Document that at-rest DB is unredacted (R9) but emission tightens. Update the module-level comment.
- **Tradeoff**: Slightly different output between MCP and CLI. The MCP is the riskier surface.
- **Cross-cited by**: error-handling.

#### M8 — Error format strings leak user input back to the caller

- **File**: `crates/claudepot-cli/src/commands/mcp.rs:336,381,414,443,484,524`
- **Observation**: `format!("{e}")` interpolates raw error Display. `ReadError::NotIndexed(path)` echoes the user-supplied path verbatim — if an MCP client probes `/Users/joker/.ssh/id_rsa`, the error echo confirms what was probed. `DurableError::InvalidScope` interpolates `project_path` verbatim. SQL errors may echo column values (UNIQUE constraint violations include row content). For `claudepot_remember`, a failed insert could echo the secret-containing `content` field.
- **Severity**: `[MEDIUM]`
- **Evidence**: Each tool method's `to_json(&error(&format!("{e}")))` call.
- **Proposed change**: Run every error string through `redact_apply` before MCP emission. Add a contract line to `rules/rust-conventions.md`'s Security section.
- **Tradeoff**: Tiny perf cost on error paths; defensive.
- **Cross-cited by**: security (#3a, #8a), error-handling.

#### M9 — `--db <path>` not canonicalized; umask race window

- **File**: `crates/claudepot-cli/src/commands/mcp.rs:36-46`; `crates/claudepot-core/src/session_index/mod.rs:70-100`
- **Observation**: CLI accepts any path. Between `Connection::open(path)` (which can *create* the file) and `set_permissions(0o600)`, the file exists with umask-default perms — typically `0644`. A local adversary watching the parent dir reads the file during that brief window. Combined with the lack of path containment, an attacker who controls the parent dir can also read the DB after chmod by holding an open fd from before.
- **Severity**: `[MEDIUM]`
- **Evidence**: `SessionIndex::open` (mod.rs:70-100) — chmod happens after `init_connection` returns successfully, including after the schema apply.
- **Proposed change**: Use `OpenOptions::new().mode(0o600).create(true)` on Unix (sets perms atomically at create time) instead of post-hoc chmod. Independently: clamp `--db` under `~/.claudepot/` with an env-var escape (`CLAUDEPOT_DATA_DIR`).
- **Tradeoff**: Two-line API change for the perms; the containment is design discipline.
- **Cross-cited by**: security (#7).

#### M10 — Migration validation count too lax for FTS5 internals

- **File**: `crates/claudepot-core/src/session_index/mod.rs:546-565`
- **Observation**: The post-write validation counts `V4_TABLE_NAMES` (7 names) in `sqlite_master`. FTS5's actual on-disk objects also include `exchange_fts_data`, `_idx`, `_content`, `_docsize`, `_config`. If any one of those FTS internal tables fails to create — or if the AFTER INSERT/DELETE/UPDATE triggers are dropped in a future edit — the count is still 7 and validation greenlights.
- **Severity**: `[MEDIUM]`
- **Evidence**: `V4_TABLE_NAMES` at `schema.rs:204-212` (7 names); validation at `mod.rs:546-565`.
- **Proposed change**: Also validate `type='trigger' AND name IN ('exchange_fts_ai','exchange_fts_ad','exchange_fts_au')` (expect 3) and at least one FTS internal table.
- **Tradeoff**: Three extra rows in the validation query.
- **Cross-cited by**: edge-cases (#3).

#### M11 — Integration test is flaky + has undeclared build dependency

- **File**: `crates/claudepot-cli/tests/mcp_memory_cli.rs`
- **Observation**: Two issues. **(a)** Fixed `thread::sleep(1500ms)` between writing frames and `child.kill()`. On Windows + first-build on a slow CI runner, 1.5s may not be enough; `child.kill()` before `wait_with_output()` can lose buffered stdout. **(b)** Test depends on `target/debug/claudepot` having been built — `cargo test` doesn't automatically build sibling binaries for integration tests. Without a `cargo build -p claudepot-cli` step before `cargo test` in CI, this test panics.
- **Severity**: `[MEDIUM]`
- **Evidence**: lines 60-78 sleep + kill order; `bin_path()` fallback at 47.
- **Proposed change**: Replace fixed sleeps with a wait-on-stdout-pattern (read frames in a loop, exit when expected ids received). Close stdin to signal end-of-input rather than kill. In CI workflow, ensure `cargo build -p claudepot-cli` runs before `cargo test`.
- **Tradeoff**: More test code; eliminates the flake class.
- **Cross-cited by**: testing (F4).

#### M12 — Durable write paths inconsistent on transaction wrapping

- **File**: `crates/claudepot-core/src/shared_memory/durable.rs:251-269,287-292,370-386,514-533,633-638`
- **Observation**: `create_memory`, `archive_memory`, `log_decision`, `submit_evidence`, `link` use bare `db.execute(...)` autocommit. Only `supersede_decision` opens an explicit transaction (because it does two statements). Future invariants ("every memory created via MCP also gets a default link", "log_decision must also stamp an audit trail row") can't be added without changing every caller, because the function returns `MemoryRecord` after autocommit.
- **Severity**: `[MEDIUM]`
- **Evidence**: see line numbers above.
- **Proposed change**: Add a `tx_scope<F, T>(&SessionIndex, F) -> Result<T, DurableError>` helper that all writers go through, even single-statement. Future-proofs the contract.
- **Tradeoff**: One thin helper; zero overhead per call.
- **Cross-cited by**: architecture (#3).

#### M13 — `list_decisions` accepts `archived` status but no API produces it

- **File**: `crates/claudepot-core/src/shared_memory/durable.rs` (DecisionStatus enum + DB CHECK)
- **Observation**: Schema CHECK + Rust enum include `Archived`, but no function transitions a decision to `archived`. The MCP `claudepot_list_decisions` filter accepts `"archived"` and returns the empty set. Either the column has a state the public API can't reach (dead branch), or there's a missing `archive_decision` API.
- **Severity**: `[MEDIUM]`
- **Evidence**: enum at durable.rs definitions; no `archive_*` function.
- **Proposed change**: Add `archive_decision(idx, id)` and surface in MCP, or drop `Archived` from CHECK + enum + filter parsing. Decide before clients depend on the listing semantics.
- **Tradeoff**: Asymmetric API invites confusion in cross-harness docs.
- **Cross-cited by**: architecture (#5).

#### M14 — TOCTOU between walk-stat and parse can poison cache

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:172-186`
- **Observation**: `walk_dir_recursive` reads `(size, mtime_ns, inode)` from `metadata()`. The transaction opens *after* the walk. The parser opens the file twice (head + events). If a Codex agent appends to the file between the walk's metadata read and the parser's opens, the parser reads content that doesn't match the cached tuple — so on the *next* backfill, the (size, mtime, inode) appears "unchanged" and the indexer skips a re-parse. Half-old, half-new content persists until the next file mutation.
- **Severity**: `[MEDIUM]`
- **Evidence**: walk_dir_recursive metadata at lines 172-186; transaction open after walk.
- **Proposed change**: Re-stat *after* parse and write the post-parse tuple (converges to truth on next mutation). Alternative: refuse to write if the tuple drifted (forces re-scan).
- **Tradeoff**: Two `metadata()` calls per file; eliminates the silent partial-state class.
- **Cross-cited by**: edge-cases (#2).

#### M15 — `tool_call` PK collision aborts the whole batch's transaction

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:386-401`
- **Observation**: Tool-call id is `<exchange_id>:<call_id>`. If Codex emits two function calls with the same `call_id` in one turn (rare but observed in some agent loops), the second `INSERT` fails with `UNIQUE constraint failed`, `?` bubbles, and the whole `tx` is unusable for subsequent files. Prior files in the same backfill that committed are saved; subsequent files all get listed as failures.
- **Severity**: `[MEDIUM]`
- **Evidence**: line 392; no `SAVEPOINT` around the per-file write.
- **Proposed change**: Wrap each `write_codex_conversation` call in a savepoint (`tx.savepoint()?`) so a per-file failure only kills that file's writes.
- **Tradeoff**: Three extra SQL operations per file; isolates failures.
- **Cross-cited by**: edge-cases (#7).

### `[LOW]`

#### L1 — `has_more` boundary false positive

- **File**: `crates/claudepot-cli/src/commands/mcp.rs:289-307`
- **Observation**: `has_more = hits.len() as u32 >= limit`. When the search returns exactly `limit` rows AND that's the total count, `has_more=true` and the next page is empty.
- **Severity**: `[LOW]`
- **Proposed change**: Query `limit + 1`, slice to `limit`, set `has_more = len > limit`. Standard pagination pattern.
- **Tradeoff**: One extra row read per page.

#### L2 — Index name misleading

- **File**: `crates/claudepot-core/src/shared_memory/schema.rs:62`
- **Observation**: `idx_exchanges_project_ts` is on `(file_path, timestamp_ms)`, not project_path. Misleads anyone reading `EXPLAIN QUERY PLAN`.
- **Severity**: `[LOW]`
- **Proposed change**: Rename to `idx_exchanges_file_ts`.

#### L3 — `tool_calls.id` separator collides on `:` in call_id

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:384-387`
- **Observation**: Composite key `<exchange_id>:<call_id>` would collide ambiguously if Codex ever emits a `call_id` containing `:`.
- **Severity**: `[LOW]`
- **Proposed change**: Either reject `call_id` containing `:`, URL-encode it, or use ASCII unit-separator `\x1f`.

#### L4 — `RememberRequest.confidence` not clamped

- **File**: `crates/claudepot-cli/src/commands/mcp.rs:145,362-372`
- **Observation**: `submit_evidence` clamps confidence to `[0, 100]`. `remember` passes it through verbatim with no clamp and no DB CHECK.
- **Severity**: `[LOW]`
- **Proposed change**: Clamp the same way.

#### L5 — LIKE pattern wildcards in user input not escaped

- **File**: `crates/claudepot-core/src/shared_memory/search.rs:144-145,154-155`
- **Observation**: User `project_path: "_"` becomes a wildcard. Behavioral quirk, not a security boundary (the column only contains paths the indexer wrote), but worth a comment.
- **Severity**: `[LOW]`
- **Proposed change**: Either escape `%`/`_`/`\` in user input, or document the wildcard semantics in the MCP tool description.

#### L6 — `escape_phrase` should strip NUL/control chars defensively

- **File**: `crates/claudepot-core/src/shared_memory/search.rs:220-233`
- **Observation**: FTS5 phrase syntax doesn't guarantee NUL-byte safety; a future tokenizer change could surprise us. Test fixtures don't cover NUL.
- **Severity**: `[LOW]`
- **Proposed change**: Add a `c == '\0' { skip }` line; add a test fixture.

#### L7 — Depth cap is silent

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:143-145`
- **Observation**: 8-level depth cap; no entry in `stats.failed` when hit. Silent if Codex ever moves to a deeper layout.
- **Severity**: `[LOW]`
- **Proposed change**: Add a `depth_capped_dirs` counter + WARN log.

#### L8 — 100k-line ceiling in read_lines is silent

- **File**: `crates/claudepot-core/src/shared_memory/read.rs:175`
- **Observation**: File-level reads stop at line 100,000. No indication to the caller.
- **Severity**: `[LOW]`
- **Proposed change**: Surface "ceiling hit" via the truncated flag (it would already be true from the byte cap in practice).

#### L9 — `derive_slug` fallback for non-UTF-8 paths

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:408-414`
- **Observation**: Non-UTF-8 file names all collapse to literal `"codex-session"`. Confusing in UI but not a correctness bug.
- **Severity**: `[LOW]`

#### L10 — `truncate_chars` not grapheme-aware

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:433-443`
- **Observation**: Boundary cuts mid-grapheme on emoji + modifiers. Minor display issue.
- **Severity**: `[LOW]`

#### L11 — `inode_of` returns 0 on Windows

- **File**: `crates/claudepot-core/src/shared_memory/indexer.rs:189-198`
- **Observation**: Atomic-replace tools on Windows that preserve size+mtime exactly would slip past the staleness guard. Risk low because Codex grows files; doesn't atomically replace.
- **Severity**: `[LOW]`
- **Proposed change**: Use `std::os::windows::fs::MetadataExt::file_index()`.

#### L12 — `read_locator` masks SQL errors as `NotIndexed`

- **File**: `crates/claudepot-core/src/shared_memory/read.rs:108-117`
- **Observation**: `.unwrap_or(false)` collapses every SQL error (busy, locked, FTS corruption) into "not indexed". Wrong attribution misleads triage.
- **Severity**: `[LOW]`
- **Proposed change**: Match explicitly on `QueryReturnedNoRows` for false; propagate other variants.

#### L13 — No rate limiting on MCP writes

- **File**: `crates/claudepot-cli/src/commands/mcp.rs`
- **Observation**: Misbehaving agent can issue unlimited `remember` / `log_decision` / `submit_evidence`. No UNIQUE constraint on `(scope, project_path, content)` in memories — dupes accumulate.
- **Severity**: `[LOW]`

### `[GOOD]` (worth preserving)

- **G1 — Parser leaf direction is clean.** `codex_session` imports nothing from `shared_memory` or `session_index`; the indexer treats it as a black box. Worth a CI gate to keep it that way. (architecture)
- **G2 — `Mutex<Connection>` not held across `.await`.** Every MCP tool is sync; the std mutex is dropped before control returns to rmcp's executor. Sidesteps the classic deadlock class. Document the invariant. (architecture)
- **G3 — FK CASCADE + CHECK constraints + AFTER INSERT/DELETE/UPDATE triggers form a coherent DB-level invariant story.** The `memory_links` exactly-one-parent + exactly-one-target CHECK plus the FK declarations make orphan rows physically impossible. (architecture, edge-cases)
- **G4 — `thiserror` / `anyhow` usage matches project conventions.** No `unwrap()` outside `#[cfg(test)]` in new code. No top-level domain noun added. (architecture)
- **G5 — `supersede_decision` + `apply_schema` rollback paths are transactionally correct.** Drop-without-commit triggers rusqlite's automatic ROLLBACK. The phase-numbered `apply_schema` design is the right shape. (error-handling)
- **G6 — No SQL injection.** Every dynamic clause binds parameters via `params_from_iter`; the `LIKE` wrap binds the percent-wrapped string rather than concatenating. (security)
- **G7 — rmcp dep tree is clean for the chosen features.** No HTTP, no TLS, no SSE/websocket transport. `transport-io` keeps the surface to stdio. (security)

---

## Edge Case Risk Matrix

Ranked by Risk (Likelihood × Impact). Top 10 from the edge-cases agent.

| Rank | Scenario | Likelihood | Impact | Risk | Component | File:Line |
|------|----------|------------|--------|------|-----------|-----------|
| 1 | Stale `sessions` row survives parse failure → search returns wrong content | Med | High | **High** | indexer | `shared_memory/indexer.rs:78-91` |
| 2 | TOCTOU walk-stat → parse → cache poisoning | Med | High | **High** | indexer | `shared_memory/indexer.rs:172-186` |
| 3 | Migration validation rollback uses misleading `QueryReturnedNoRows` | High | Med | **High** | session_index | `session_index/mod.rs:569` |
| 4 | Migration `BUSY` could trigger cache quarantine → cache-loss bug | Low | High | **Med** | session_index | `session_index/mod.rs:115` |
| 5 | Validation count too lax for FTS5 internals/triggers | Low | High | **Med** | session_index | `session_index/mod.rs:551-565` |
| 6 | `version_less_than` lex fallback inverts safety on v10+ | Low | High | **Med** | session_index | `session_index/mod.rs:591-598` |
| 7 | `truncated` flag wrong order + boundary | Med | Low | **Med** | shared_memory/read | `shared_memory/read.rs:117` |
| 8 | One bad `tool_call` PK aborts whole tick's tx | Low | High | **Med** | shared_memory/indexer | `shared_memory/indexer.rs:386` |
| 9 | Partial-parse cache stickiness (silent EventIter error) | Med | Med | **Med** | codex_session/parser | `codex_session/parser.rs:284-291` |
| 10 | Symlink under `$CODEX_HOME/sessions/` reaches `read_locator` | Low | High | **Med** | shared_memory/indexer | `shared_memory/indexer.rs:148-198` |

---

## Pressure-test sections

### Principle violations

- **Single Responsibility (SRP) — durable.rs is too broad.** `durable.rs` (970 lines) handles memories, decisions, evidence, and links in one file. Each noun has its own NewX struct, ListXFilter, list / create / supersede paths. Splitting into `durable/{mod,memories,decisions,evidence,links}.rs` would localize future changes; today's loc-guardian threshold (350) is already exceeded.
- **Dependency direction — clean.** The `session_live → codex_session ← shared_memory` graph that the plan committed to is upheld. `codex_session` has zero dependencies on `shared_memory`. Worth preserving with a CI grep gate (e.g. `! grep -rn 'shared_memory\|session_index' crates/claudepot-core/src/codex_session/`).
- **Least privilege — partial violation.** `--db <path>` accepts any path the user can read/write, and the post-hoc chmod 0600 leaves a brief umask-default window (M9). The MCP server runs as the user, so this isn't a privilege escalation, but it widens the trust boundary beyond `~/.claudepot/`.
- **Interface segregation — fine for now.** Each `shared_memory::*` module exposes a small surface (search has 3 public items, read has 5, durable has ~20). DTOs in mcp.rs mirror them rather than wrap them — that's tolerable since the MCP layer is the only client, but if a UI surface lands too, expect duplication pressure.

### Assumptions audit

The new code commits to these assumptions. Each needs a validation plan before the feature flips on by default.

| # | Assumption | How to validate quickly |
|---|------------|-------------------------|
| 1 | Codex's `session_meta.payload.id` is unique across all rollouts on a machine | Run a 1-line script over `~/.codex/sessions/**/*.jsonl` extracting payload.id; assert uniqueness on a populated workstation |
| 2 | Codex rollout format `{timestamp, type, payload}` is stable across CLI versions 0.30+ | Fixture corpus from at least 3 Codex versions; add to test suite |
| 3 | `$CODEX_HOME` defaults to `~/.codex` | Confirmed against current Codex CLI; document in CLAUDE.md |
| 4 | `sessions.db` at-rest 0600 perms hold on macOS APFS, Linux ext4, Linux Btrfs, Linux ZFS, Windows NTFS | Test on each; the test-host matrix covers macOS already, Linux/Windows pending |
| 5 | rmcp 1.7 stdio transport is reliable across Codex CLI and Claude Code MCP clients | The spike validated against a hand-rolled harness; need the same against a real Codex CLI invocation |
| 6 | Codex's `call_id` is unique within a session | Same script as (1) extracting call_ids per session |
| 7 | The 100k-line ceiling in `read_lines` is above the longest real Codex transcript | Largest seen in `~/.codex/sessions/` — measure |
| 8 | `_min_compatible_version` marker semantics will be honored by future Claudepot versions | Documentation discipline + a test that confirms a "future" marker (write `"99"`) blocks the migration |

The cheapest path is a 30-line Bash script that walks `~/.codex/sessions/`, extracts payload.id and call_ids, and reports any duplicates. Run it on a developer machine before flipping the flag.

### Compact & optimize

- **Enum-to-string helpers are duplicated.** `durable.rs::scope_from_str` etc. mirror `mcp.rs::scope_str`. Move to a single helper module — `shared_memory::enums` — exposing `Scope::from_str`, `Scope::as_str`, etc. Saves ~80 lines.
- **Seven MCP tool methods are nearly identical.** Each follows: deserialize → validate → call into core → serialize → handle errors. A `Result<T: Serialize, ErrorPayload>` helper that wraps `to_json` would cut boilerplate.
- **`build_snippet` and `truncate_chars`** in indexer.rs are general utilities that arguably belong in `shared_memory::utils`. The redaction-at-write fix (M3) is the right time to extract.
- **`apply_schema`'s placeholder builder** at `session_index/mod.rs:548-555` is awkward (`vec!["?"; N].join(",")`). The crate already uses `params_from_iter` elsewhere — `IN (?)` with a `Vec<rusqlite::types::Value>` reads cleaner. Saves ~10 lines.
- **Migration test scaffolding repeats the same `INSERT INTO sessions (...) VALUES (...)`** 30+ times across the 6 case tests. Extract a `fn seed_sessions_row(db, file_path, source_kind)` helper. Cuts ~150 lines from the test file without losing intent.

---

## Executive Summary

### One-paragraph verdict

The branch lands well-designed, well-tested core Rust scaffolding for cross-harness shared memory — schema migration is genuinely crash-safe, FK + CHECK invariants are coherent, the parser is a clean leaf, and the MCP server boots and round-trips on real `rmcp 1.7`. **But it's a half-shipped feature**: the Codex indexer has zero production callers (so MCP search returns empty in production), the feature-flag claim from the plan was dropped without acknowledgment (so every existing user's `sessions.db` will auto-migrate to v4 on next boot), and six negative-path tests are `matches!` no-ops that don't actually assert. The single biggest risk is the **tracing stdout pollution** (H1) — a single `tracing::info!` from `apply_schema` or `indexer` under the default subscriber will corrupt the MCP protocol stream silently, and the existing integration test hides this with `RUST_LOG=warn`. Fix the test discipline, wire `backfill_codex`, and decide the feature-flag stance before this merges.

### Top 3 actions

1. **Fix the tracing-to-stderr discipline (H1) before any deploy.** Install a stderr-pinned subscriber in `mcp::run` and re-run the stdout test without the `RUST_LOG` override. Estimated effort: < 1 day. Prevents silent protocol corruption.
2. **Wire `backfill_codex` to a CLI command and decide the feature-flag stance (H3 + H4).** Either implement `shared_memory_index` as a Cargo feature gating the v4 DDL + MCP subcommand + new modules, OR commit to v4 being unconditional and document in release notes. Today's state — code says go, plan says off-by-default — is the worst of both. Estimated effort: < 1 week.
3. **Wrap the six `matches!` no-op assertions in `assert!` (H2) and add a CI grep gate.** Five minutes of edits + a one-line grep that fails CI if a bare `matches!` re-appears. Unlocks the entire negative-path test value.

### Confidence levels

| Recommendation | Confidence | What would raise it |
|---|---|---|
| Tracing-to-stderr fix (H1) | **High** | Test that disables `RUST_LOG` and asserts stdout-only-JSON-RPC |
| `matches!` no-ops (H2) | **High** | Already verified by reading the test file |
| Feature flag absence (H3) | **High** | Already verified by grep |
| `backfill_codex` callers (H4) | **High** | Already verified by grep |
| Search error swallow (H5) | **High** | Direct code reading |
| Migration safety design (good) | **High** | Six test cases + manual code inspection |
| Symlink / containment (M4) | **Medium** | A symlink fuzz test in indexer tests |
| TOCTOU walk-stat-parse (M14) | **Medium** | Concurrent-modification test |
| Migration concurrency safety (edge-cases #4) | **Medium** | Two-process race test |
| Codex format stability (assumption 2) | **Low** | Need fixture from ≥3 Codex versions |

### Paranoid Verdict — the single scariest thing

**H1 — Tracing stdout pollution.** Picture this: a user opens Claude Code with the MCP server configured. Claude Code launches `claudepot mcp memory-server` as a subprocess. The subscriber initializes to stdout. `apply_schema` runs the v4 migration and emits `tracing::warn!` for a non-fatal warning (e.g. a downgrade-marker mismatch). That warning text — formatted, not JSON-RPC — lands on stdout, between the `initialize` response and the first `tools/list` response. The MCP client's JSON-RPC parser chokes. The server appears dead. The user never sees a memory tool work, and there's no signal in the logs that explains why — because the logs *were* the corruption. This is the failure mode that, once it ships, is the hardest to diagnose remotely. Fix the subscriber before the merge.

---

## Fixing Plan

Every item below traces back to a finding above by ID (H1/M3/L7/etc.). Phases are ordered by severity but items within a phase can be parallelized unless flagged with a dependency note.

### Phase 1: Critical fixes (do immediately — block merge)

- **H1 — Pin tracing subscriber to stderr in `mcp::run`**
  - Fix: In `crates/claudepot-cli/src/commands/mcp.rs::run`, install (or assert) `tracing_subscriber::fmt().with_writer(std::io::stderr).with_ansi(false).init()` before `SessionIndex::open`. Update the integration test `stdout_only_emits_jsonrpc_frames` to run without `RUST_LOG=warn`.
  - Effort: < 1 day
  - Files: `crates/claudepot-cli/src/commands/mcp.rs`, `crates/claudepot-cli/tests/mcp_memory_cli.rs`

- **H2 — Wrap six `matches!` no-ops in `assert!`**
  - Fix: Add `assert!(...)` around every bare `matches!` line in the listed tests. Add a CI grep gate: `! grep -rnE '^\s*matches!\b' crates/claudepot-core/src/`
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/codex_session/tests.rs:36,52,202`, `crates/claudepot-core/src/shared_memory/durable.rs:741,878`, `crates/claudepot-core/src/shared_memory/read.rs:284`, plus a CI config file (`.github/workflows/ci.yml` or similar)

- **H3 — Decide feature-flag stance**
  - Fix: Either (a) implement `shared_memory_index` and `shared_memory_mcp` as Cargo features on `claudepot-core` and `claudepot-cli`, gating `pub mod shared_memory`, the v4 DDL block (with a no-op v3-only `apply_schema` path on the feature-off build), and the `Mcp` clap variant; OR (b) commit to v4 being unconditional and remove the flag claim from the plan, documenting the one-way migration in release notes.
  - Effort: < 1 week (a) / < 1 day (b)
  - Files: `Cargo.toml` (workspace), `crates/claudepot-core/Cargo.toml`, `crates/claudepot-cli/Cargo.toml`, `crates/claudepot-core/src/lib.rs`, `crates/claudepot-core/src/session_index/mod.rs`, `crates/claudepot-cli/src/main.rs`, `dev-docs/codex-plans/20260515-1130-shared-memory.md`

- **H4 — Wire `backfill_codex` to a CLI verb and add an MCP-to-search integration test**
  - Fix: Add `claudepot codex-index [--codex-home <path>]` subcommand calling `backfill_codex`. Add a smoke test that stages a Codex rollout, runs the index, calls `claudepot_search_memory` over MCP, asserts a hit.
  - Effort: < 1 week
  - Files: `crates/claudepot-cli/src/commands/{codex.rs,mod.rs}`, `crates/claudepot-cli/src/main.rs`, `crates/claudepot-cli/tests/mcp_memory_cli.rs`
  - Depends on: H3 (decide whether this verb is feature-gated)

- **H5 — Stop swallowing search errors via `unwrap_or_default`**
  - Fix: In `mcp.rs::claudepot_search_memory`, replace `unwrap_or_default()` with the standard `match … { Ok(_) => …, Err(e) => return to_json(&error(&format!("{e}"))) }` plus a `tracing::warn!` line.
  - Effort: < 1 day
  - Files: `crates/claudepot-cli/src/commands/mcp.rs:289`

- **H6 — Eliminate parse-failure cache stickiness (two changes)**
  - Fix 1: In `codex_session/parser.rs::EventIter`, surface `truncated_by_io: bool` and `malformed_lines: u32` after iteration. Return `(CodexConversation, ParseDiagnostics)` from `parse_codex_rollout_jsonl`.
  - Fix 2: In `shared_memory/indexer.rs::upsert_codex_session`: (a) when `truncated_by_io`, refuse to stamp the staleness triple (so next backfill retries); (b) on parse failure for a previously-indexed file, `DELETE FROM sessions WHERE file_path=? AND source_kind='codex'`.
  - Effort: < 1 week
  - Files: `crates/claudepot-core/src/codex_session/{parser,types}.rs`, `crates/claudepot-core/src/shared_memory/indexer.rs`

- **H7 — Differentiate MCP error categories**
  - Fix: Add `error_code: String` to `ErrorPayload`. Populate per error variant (`invalid_scope`, `decision_not_found`, `locator_not_indexed`, `sql_error`, `parse_failed`). Pipe every error string through `redact_apply` before MCP emission.
  - Effort: < 1 day
  - Files: `crates/claudepot-cli/src/commands/mcp.rs` (the `ErrorPayload` struct + every `error()` call site)

### Phase 2: High-priority fixes (this sprint)

- **M1 — Fix `read_locator.truncated` flag**
  - Fix: Change `>=` to `>`. Update docstring to match implementation ("max_bytes caps the pre-redaction read; the redacted body may be shorter"). Add tests for the boundary case and for redaction shrink.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/shared_memory/read.rs:87-126`

- **M2 — Add `SessionIndexError::MigrationValidationFailed` variant**
  - Fix: New error variant `{ expected: usize, found: usize, missing: Vec<String> }`. Re-query `sqlite_master` for missing names. Log at `tracing::error!`. Update case4 migration test to assert on the new variant.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/session_index/{mod.rs,error.rs}`, `crates/claudepot-core/src/shared_memory/migration_tests.rs`

- **M3 — Pre-redact `snippet_text` at write time**
  - Fix: In `indexer.rs::build_snippet`, run the snippet through `redaction::apply` before returning. The schema comment becomes true; direct readers are safe.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/shared_memory/indexer.rs:416-431`

- **M4 — Symlink-safe indexer + canonical-path read**
  - Fix 1: In `indexer.rs::walk_dir_recursive`, check `entry.file_type().is_symlink()` and skip. Or use `symlink_metadata` instead of `metadata`.
  - Fix 2: In `read.rs::read_locator_bounded`, before `File::open`, canonicalize the path and assert it starts with one of the configured roots (`$CODEX_HOME/sessions/` or `~/.claude/projects/`).
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/shared_memory/{indexer,read}.rs`

- **M5 — Cap `read_line` to defeat OOM DoS**
  - Fix: Wrap the reader with `.take(1 << 20)` (1 MiB) before `read_line`. On oversize, drain to next `\n`, log WARN, continue.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/codex_session/parser.rs:266`

- **M6 — Conservative `version_less_than` fallback**
  - Fix: On parse failure of either side, return `true` (treat unknown as "I cannot reason; refuse to migrate"). Reject non-numeric writes at write time.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/session_index/mod.rs:591-598`

- **M7 — Stricter MCP redaction policy**
  - Fix: Build `RedactionPolicy { emails: true, env_assignments: true, ..Default::default() }` at MCP server construction. Update the module doc comment.
  - Effort: < 1 day
  - Files: `crates/claudepot-cli/src/commands/mcp.rs:48-51` (plus the module header)

- **M8 — Run all error format strings through `redact_apply`**
  - Fix: Wrap every `format!("{e}")` at the MCP boundary in `redact_apply`. Add a contract line to `rules/rust-conventions.md`'s Security section.
  - Effort: < 1 day
  - Files: `crates/claudepot-cli/src/commands/mcp.rs:336,381,414,443,484,524`, `.claude/rules/rust-conventions.md`

- **M9 — Atomic 0600 on DB create**
  - Fix: Use `OpenOptions::new().mode(0o600).create(true)` on Unix to set perms atomically. Clamp `--db` under `~/.claudepot/` with `CLAUDEPOT_DATA_DIR` env escape.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/session_index/mod.rs:70-100`, `crates/claudepot-cli/src/commands/mcp.rs:36-46`

- **M10 — Strengthen migration validation**
  - Fix: After the V4_TABLE_NAMES count check, also validate `type='trigger' AND name IN ('exchange_fts_ai','exchange_fts_ad','exchange_fts_au')` (expect 3) and at least one FTS internal table.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/session_index/mod.rs:551-565`

- **M11 — De-flake the integration test**
  - Fix: Replace fixed `sleep(1500ms)` with wait-on-stdout-pattern: read frames in a loop until expected ids are seen. Close stdin (drop the write half) instead of killing the child; that signals end-of-input cleanly. In CI workflow, ensure `cargo build -p claudepot-cli` runs before `cargo test`.
  - Effort: < 1 day
  - Files: `crates/claudepot-cli/tests/mcp_memory_cli.rs`, `.github/workflows/ci.yml`

- **M12 — Add `tx_scope` helper to durable.rs**
  - Fix: One helper that wraps every writer in `unchecked_transaction`. Migrate `create_memory`, `archive_memory`, `log_decision`, `submit_evidence`, `link` to use it. (`supersede_decision` already has the right shape.)
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/shared_memory/durable.rs`

- **M13 — Resolve `archived` decision status asymmetry**
  - Fix: Either add `archive_decision(idx, id)` + MCP tool, or drop `Archived` from CHECK + enum + filter parsing.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/shared_memory/{durable.rs,schema.rs}`, `crates/claudepot-cli/src/commands/mcp.rs`

- **M14 — Re-stat after parse to converge TOCTOU**
  - Fix: After `parse_codex_rollout_jsonl` succeeds, re-stat the file and write the post-parse triple to `sessions`. On the next backfill, mid-write files will be re-parsed because the triple has moved past whatever was captured during the walk.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/shared_memory/indexer.rs:172-258`

- **M15 — Savepoint per file in backfill**
  - Fix: Wrap each `write_codex_conversation` call in `tx.savepoint()?` so a PK collision or any per-file error only kills that file's writes.
  - Effort: < 1 day
  - Files: `crates/claudepot-core/src/shared_memory/indexer.rs:78-91`

### Phase 3: Medium-priority improvements (next sprint)

(All `[LOW]` findings, plus the deferred WIs.)

- **L1 — Pagination `has_more` boundary**: query `limit + 1`, slice to `limit`, set `has_more = len > limit`. `crates/claudepot-cli/src/commands/mcp.rs:289-307`. < 1 day.
- **L2 — Rename index**: `idx_exchanges_project_ts` → `idx_exchanges_file_ts`. `crates/claudepot-core/src/shared_memory/schema.rs:62`. < 1 day.
- **L3 — `tool_calls.id` separator**: switch to ASCII unit-separator or reject `:` in `call_id`. `crates/claudepot-core/src/shared_memory/indexer.rs:384-387`. < 1 day.
- **L4 — Clamp `confidence` in remember**: same `clamp(0, 100)` as `submit_evidence`. `crates/claudepot-cli/src/commands/mcp.rs:362-372`. < 1 day.
- **L5 — Escape `%`/`_`/`\` in LIKE filters**: or document the wildcard semantics. `crates/claudepot-core/src/shared_memory/search.rs:144-145,154-155`. < 1 day.
- **L6 — Strip NUL/control chars in `escape_phrase`**. `crates/claudepot-core/src/shared_memory/search.rs:220-233`. < 1 day.
- **L7 — Surface depth-cap hits as a counter + WARN log**. `crates/claudepot-core/src/shared_memory/indexer.rs:143-145`. < 1 day.
- **L8 — Surface 100k-line ceiling hits**. `crates/claudepot-core/src/shared_memory/read.rs:175`. < 1 day.
- **L11 — Use `file_index()` on Windows for inode**. `crates/claudepot-core/src/shared_memory/indexer.rs:189-198`. < 1 day.
- **L12 — Distinguish SQL errors from `NotIndexed`**. `crates/claudepot-core/src/shared_memory/read.rs:108-117`. < 1 day.

- **Wire the deferred UI surfaces** — WI-007 (Shared Memory section), WI-009 (Installer pane with dual-signal health badge), WI-L1..L4 (Sessions Live cards) per the plans. Includes the Settings → Cleanup → `_pending_rescan` / `Forget Shared Memory` wiring referenced by the schema doc-comment. Each is roughly < 1 week.

- **Claude-side exchange population** — extend `session_index::refresh` to also emit `exchanges` + `tool_calls` for Claude transcripts so the unified search actually covers both harnesses. < 1 week.

### Phase 4: Low-priority cleanup (when touching these files)

Grouped by file so a developer touching any file can address all low items in it at once.

- **`crates/claudepot-core/src/shared_memory/indexer.rs`**
  - L9: `derive_slug` falls back to literal `"codex-session"` for non-UTF-8 names — make slug carry an index suffix.
  - L10: `truncate_chars` not grapheme-aware — use `unicode-segmentation::graphemes`.
- **`crates/claudepot-cli/src/commands/mcp.rs`**
  - L13: No rate limiting / dedup on `remember` writes — add a UNIQUE on `(scope, project_path, content)` or a per-session call budget.

### Compact & Optimize follow-ups

(Not blocking; pick them up when the deferred UI work lands and the modules get further consumers.)

- Extract `shared_memory::enums` to hold all `from_str`/`as_str` helpers shared between `durable.rs` and `mcp.rs`. ~80 LOC saved.
- Add a `respond<T: Serialize>(result: Result<T, E>)` helper in `mcp.rs` to collapse the seven near-identical tool-method bodies. ~120 LOC saved.
- Move `build_snippet` + `truncate_chars` to `shared_memory::utils` alongside the M3 fix.
- Replace the `vec!["?"; N].join(",")` placeholder builder in `apply_schema` with direct `params_from_iter`.
- Extract a `seed_sessions_row(db, file_path, source_kind)` test helper for `migration_tests.rs`. ~150 LOC saved.

### Dependency graph

- **H4 depends on H3** — wiring the CLI verb depends on whether `shared_memory_index` is a Cargo feature or unconditional code.
- **M1 / M3 / M14 are independent** but should land in one PR ("indexer & read polish") because the test fixtures overlap.
- **M9 (atomic 0600) and M4 (symlink safety)** belong in the same "trust boundary tightening" PR; the test coverage overlaps.
- **H6 depends on no other fix** but enables M14's converging-tuple strategy (the partial-parse diagnostics give M14 a signal to refuse the stamp).
- **L1–L13 have no inter-dependencies**.

### Estimated total effort

- Phase 1 (Critical, 7 fixes): **~3–4 days** if parallelized across the H-items (H1/H2/H5/H7 are < 1 day each, H3/H4/H6 are < 1 week each).
- Phase 2 (High, 15 fixes): **~5–8 days** if batched into 4–5 PRs.
- Phase 3 (Medium, 10 LOW items + 6 deferred WIs): **~3–6 weeks** dominated by the UI work.
- Phase 4 (Low cleanup): **opportunistic**, no commitment.
- **Total Phase 1 + 2 (everything to make this mergeable + production-ready): ~10–14 working days.**

The minimum viable "merge with confidence" cut is **Phase 1 only**, plus M1, M3, M4, M11 from Phase 2 (the safety + privacy + flake fixes). That's **~5–7 days** and closes the cluster of issues that would otherwise hit users on day one.


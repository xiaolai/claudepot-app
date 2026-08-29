# Claude Code upstream watch — the surfaces that drift

**This is `cargo xtask cc-drift`'s input file, not documentation.**
`cc_drift::parse_watchlist` reads the table below on every run, and
`cargo xtask verify-docs` fails if it stops parsing or names a module
that no longer exists.

Claudepot mirrors Claude Code behaviour in ~20 places. **Claude Code
ships ~27 releases a month** (measured: 488 npm versions, 23–30 every
month of 2026). Nothing in this repo noticed when one of those releases
changed a behaviour we reimplemented — see "Precedent" at the end.

The table exists so a month of upstream change becomes a **token
search** rather than a reading assignment: 658 changelog bullets a month
is not a control a human can operate.

## Why it lives here and not in `.claude/rules/`

Everything under `.claude/rules/` is loaded into **every** Claude Code
session in this repo, so it costs context on every task. That is the
right trade for conventions you need constantly — paths, design,
architecture. It is the wrong trade for a monthly routine's target list,
which is consulted about twelve times a year and ran ~9 KB.

So the split is by *lifetime*, not by importance:

- the **rule** — every CC-facing surface needs a row, verify against the
  binary and never the mirror — stays at
  `.claude/rules/cc-upstream-watch.md`, because it applies to any task
  that touches CC behaviour;
- the **table and the method** live here, next to the tool that reads
  them, loaded only when someone actually runs the check.

Design, cadence and the extractor details are in
`dev-docs/cc-upstream-watch.md` (gitignored, like every plan here).

## What makes a row

**Every Claudepot surface that reimplements, parses, or depends on a
Claude Code behaviour has a row.** Adding a CC-facing module without
adding one is a review finding, the same way an event channel without a
subscriber is.

A row is only useful if it says how to *check* it. "Watch this" is not a
check. A row names the grep tokens that would appear in a changelog or a
`strings` dump, and the command that settles the question.

## Verification authority — the installed binary, not the mirror

`~/github/claude_code_src` is a third-party mirror pinned at **2.1.88**,
abandoned upstream on 2026-04-15. Treat it as archaeology only. Anything
it says about current behaviour is 145+ versions stale.

CC ships as a bun-compiled binary that **retains readable JS and string
literals**, so it is the authority:

```bash
strings -n 60 ~/.local/share/claude/versions/<ver> | grep '<pattern>'
claude --help | grep -- '--<flag>'
```

That is how the `cleanupPeriodDays` inversion was found: the complete
zod error message is sitting in the binary in plain text.

## Signals, in the order they pay

| Signal | Command | Catches |
|---|---|---|
| Changelog | `gh api repos/anthropics/claude-code/contents/CHANGELOG.md` | intent, rationale, **reverts and removals** |
| Docs | `curl https://code.claude.com/docs/llms.txt` then per-page `.md` | documented-surface change |
| Binary | `strings -n 6 <binary>` diffed against the previous version | **undocumented internals** |

The binary is the one that pays: across two releases it surfaced 17 new
`CLAUDE_*`/`ANTHROPIC_*` names of which the changelog announced 3.

**Filter, or the diff is noise** — two releases produce ~57,000 new raw
strings. `^(CLAUDE_|ANTHROPIC_)[A-Z0-9_]+$` reduces that to 17 real ones.
There is no generic filter: a shape heuristic for settings keys returns
dictionary words out of embedded data. Per-surface extractors, always.

**Tokens must be distinctive, and the first draft of this table proved
it.** `plan` and `policy` looked like reasonable tokens for the
permission-mode and settings-precedence rows; over 115 releases they
produced 51 hits, nearly all of them ordinary English in unrelated
bullets. A token that matches prose trains the reader to skim the
report, which is the same failure as a gate nobody watches. Prefer a
camelCase key (`defaultMode`), a flag with its dashes (`--setting-sources`),
or a screaming-snake family (`CLAUDE_*`) over any word that could appear
in a sentence.

Do not fetch `llms-full.txt` with a bare `curl` — it is 7.5 MB and
truncates mid-transfer, which diffs as "hundreds of pages removed".

## The watchlist

| CC surface | Claudepot owner | Grep tokens | Check |
|---|---|---|---|
| `cleanupPeriodDays` semantics + floor | `cc_retention` | `cleanupPeriodDays`, `session-persistence`, `persistSession` | binary strings; `claude --help`; `MIN_CLEANUP_PERIOD_DAYS` is the pin |
| `cleanupPeriodDays` sweep scope | `cc_sweep::SWEPT` | `shell-snapshots`, `dump-prompts`, `file-history`, `retention sweep` | binary: grep the cleanup module's sweep fns, then reconcile against the `SWEPT` table. A new directory is a new row; a changed sweep unit (files vs subdirs) silently makes a count read zero. **Reconciled 2.1.250 (2026-08-28) and it moved:** unlinking a top-level `<uuid>.jsonl` now also `rm -rf`s the sibling `<uuid>/` session folder with no per-file age check, and a session folder whose transcript is already gone gets `subagents` / `workflows` / `remote-agents` swept recursively by mtime, plus `mcp-tasks` by `.json`. That reversed this repo's standing claim that nested files are immortal — see `TranscriptRisk::nested_below_session`. Watch also the per-session sidecars the sweep now names: `.ccr-tip.json`, `.precompact.json`, `.dir-sync.json`, `.desktop-released.json`, `.jsonl.compact.tmp.`, and the `bagel` directory. |
| `desktopSessionCleanupPeriodDays` | `cc_retention`, `cc_sweep` | `desktopSessionCleanupPeriodDays`, `desktop-host`, `Cowork` | Added in 2.1.248 and **not modelled**. Binary schema: `int().nonnegative().optional()` — transcripts created or last written by a desktop-host surface (Claude Desktop, Cowork) are *exempt from the `cleanupPeriodDays` sweep entirely*, and this key is the ceiling on that exemption. Three traps: **`0` is the default and means "no ceiling, keep forever"**, the exact opposite of `cleanupPeriodDays`, which rejects `0` — so a control reusing the retention pane's validation is wrong; `TranscriptRisk` counts all of `projects/` and therefore now **over**-reports deletion risk (safe direction, but a wider gap than the one AGENTS.md records); and CC's suppression check loops over `["cleanupPeriodDays","desktopSessionCleanupPeriodDays"]`, so an unreadable settings file suppresses cleanup when *either* key may be hiding in it while `cleanup_suppressed` models one. **Verified 2.1.250 (2026-08-28).** |
| settings-validation → cleanup suppression | `cc_retention::RetentionMode::Invalid` / `LegacyZero` | `Skipping cleanup`, `validation errors` | binary strings |
| env var catalog + `SAFE_ENV_VARS` | `cc_env`, `data/cc-env-spec.json` | `CLAUDE_*`, `ANTHROPIC_*`, `SAFE_ENV_VARS`, `PROVIDER_MANAGED` | `scripts/build-cc-env-spec.py --rebuild-evidence` then `--check` |
| settings merge precedence | `config_view::effective_settings`, `parity-harness/` | `settings.local.json`, `managed-settings`, `--setting-sources`, `settingSources` | `cargo xtask verify-cc-parity` + re-pin |
| `permissions.defaultMode` | `permission::settings` | `defaultMode`, `bypassPermissions`, `acceptEdits`, `permissionMode` | binary strings for the mode wire strings |
| `availableModels` / `enforceAvailableModels` | `available_models` | `availableModels`, `enforceAvailableModels` | binary strings |
| model ids and rates | `pricing`, `session_live::pricing`, `src/costs.ts` | `claude-opus`, `claude-sonnet`, `claude-fable`, `claude-haiku`, `mythos`, `<synthetic>` | changelog grep; add a `RatePeriod` **and** a vector to `rate-resolution-vectors.json`. Also watch the set of **non-model placeholders** CC writes into a transcript's `model` field: `<synthetic>` is one today, and because the model list serializes sorted from a `BTreeSet` an angle-bracket value sorts ahead of every real id and silently takes a whole session's cost out of the report. A new placeholder needs adding to `pricing::is_synthetic_model` and to `src/costs.ts`. Check with `sqlite3 ~/.claudepot/sessions.db "SELECT DISTINCT models_json FROM sessions"`. **Verified 2.1.241 (2026-08-24).** |
| fast-mode billing (known gap) | `fast_mode_toggle`, `pricing` | `fastMode` | changelog grep |
| auto-update channel | `updates::settings_bridge`, `updates::version::CcChannel` | `autoUpdatesChannel`, `minimumVersion`, `autoUpdates` | binary strings for the accepted enum, **plus** `curl -sI downloads.claude.ai/claude-code-releases/<value>` for each one — a value CC accepts does not imply a feed exists. At 2.1.241 the enum is `["latest","stable","rc"]` and `rc` 404s (CC's own UI shows it as *"slow"*), so it is `CcChannel::Untracked`. A new value with a live feed becomes a `Channel` variant; one without joins `rc`. **Verified 2.1.241 (2026-08-24).** |
| auto-memory + consolidation | `settings_writer`, `memory_view`, `auto_dream` | `autoMemoryDirectory`, `autoDreamEnabled` | docs `memory.md` diff |
| artifact toggle | `artifact_toggle` | `enableArtifact`, `disableArtifact` | binary strings |
| commit/PR attribution | `attribution_settings` | `Co-Authored-By`, `includeCoAuthoredBy` | binary strings |
| tips ledger shape | `cc_tips::catalog`, `cc_tips::history` | `tipsHistory`, `numStartups`, `spinnerTipsEnabled`, `spinnerTipsOverride`, `tipsFile` | re-run catalog extraction. **2.1.247 gave `spinnerTipsOverride` structured entries** (`{id, text, cooldownSessions, priority}`) plus `tipsFile` and `label`. Checked against the code rather than inferred: **this needs no change.** `cc_tips` extracts the catalog from the CC *binary* and joins it with `tipsHistory`; it never reads the `spinnerTipsOverride` setting. (`categories::spinner_override_prose` is a different thing — a hand-mirrored set of CC's own built-in override tips.) The standing limitation is older than 2.1.247 and unchanged by it: an org supplying its own tips through that setting would see the ledger render the binary's tips instead. The *history* half is unaffected — CC's defaults still carry `tipsHistory:{}` and `numStartups:0`, so the counter→wall-clock mapping in `cc_tips_snapshots.jsonl` is intact. Note CC ignores a `tipsFile` arriving from remote managed settings. **Verified 2.1.250 (2026-08-28).** |
| `claude doctor` output | `cc_doctor` | — | run it, diff against the parser |
| daemon roster shape | `cc_daemon` | `roster.json`, `supervisorPid`, `proto`, `workers` | binary: read the roster's zod schema (`un({proto:…, supervisorPid:…, updatedAt:…, workers:…})`) and the path helpers `function c(){return i(be(),"daemon")}` / `Te.daemon(["roster.json"])`. Two numbers to re-check: the accepted `proto` range (`[1,1]` at 2.1.251 — `MAX_KNOWN_PROTO` must not lag it, or every roster degrades) and the quarantine size bound (8 MiB, `MAX_ROSTER_BYTES`). This row **replaces** a `claude daemon status` output-scrape row. That scraper was deleted, not repaired: on a CC predating the `daemon` subcommand its positional fell through to a billed model prompt once a minute (issue #94, and see the subcommand-floor row below). **Verified 2.1.251 (2026-08-29).** |
| subcommand version floors | `cc_capability::GatedSubcommand` | `claude --help`, changelog | **the row that exists because a green check proves nothing here.** CC's grammar is `claude [options] [command] [prompt]`, so an unrecognized *positional* becomes the prompt and is billed — silently, and only on binaries older than the reader's. Confirm each floor against the changelog entry cited in the table (`auth` 2.1.41, `daemon` 2.1.139), and add a row for any NEW subcommand Claudepot spawns. Note what a floor cannot do: it guards the lower bound only, so a subcommand CC *removes or renames* walks straight through it. Anything polled on a timer must not reach the CLI at all — `cc_daemon` reads a file for exactly this reason. **The one polled CLI spawn that remains is `cc_doctor`** (`claude doctor`, every 5 min, on a pty). It has no floor row because the changelog only proves the verb existed by 2.0.33, which is a lower bound on its age and not an introduction version — a floor citing it would look verified and would not be. It cannot take `cc_daemon`'s remedy either: `doctor` is a computed diagnostic, not stored state, so there is no file to read instead. It is guarded by a **circuit breaker** in `src-tauri/src/cc_doctor_watcher.rs`: a fall-through prompt can never print CC's `Diagnostics` header, so it lands `ParseStatus::Degraded` on every attempt, and three in a row stop the poll for the life of the process. That caps a removed or renamed verb at three billed calls instead of 288 a day — which is the bound issue #94 lacked. So what to watch for here is not billing but **silence**: a CC output change that trips the breaker legitimately costs the tray its health signal until the parser is updated and Claudepot restarted. **Verified 2.1.251 (2026-08-29).** |
| transcript JSONL schema | `session_index`, `session_live`, `corpus` | `sidechain`, `toolUseResult`, `subagents/`, `cost-state`, `atis-latch`, `bridge-session`, `file-history-delta` | parse newest transcripts; count unknown record kinds. **Four are known-unmodelled as of 2.1.250 (2026-08-28)** — measure the delta against that list rather than rediscovering them: `atis-latch`, `bridge-session`, `file-history-delta` (the companion of the `file-history-snapshot` we *do* parse — we read half a pair), and `cost-state`, which is the one that matters: it carries CC's own `totalCostUSD`, `modelUsage` and a `hasUnknownModelCost` flag, i.e. a second opinion on the number the Cost surface computes itself via `PriceBook`. Also watch the `system` subtypes — `turn_duration`, `stop_hook_summary`, `away_summary`, `local_command`, `informational` are the set seen at 2.1.250. |
| path-keyed global state | `project::move_project` P4–P10 | `installed_plugins.json`, `history.jsonl`, `projects[` | `rules/cc-state-move-blast-radius.md` invariant |
| `claude -p` flag surface | `agent::shim` | `--output-format`, `--model`, `--fallback-model`, `--allowedTools`, `--disallowed-tools`, `--system-prompt`, `--append-system-prompt`, `--add-dir`, `--mcp-config`, `--include-partial-messages`, `--bare` | grep `claude --help` per flag |
| credentials keychain item | `cli_backend/keychain` | `-credentials`, `Claude Safe Storage` | **read the keychain, not the binary.** `security find-generic-password -s 'Claude Code-credentials'` must exit 0 and report that `svce`. The full literal is **not in the binary** — CC concatenates the service name at runtime, so grepping `strings` for `Claude Code-credentials` returns zero on a build where the item is perfectly intact. That is the shape of token this file's own preamble warns about: a check that cries wolf every month trains the reader to skim. Verified 2.1.250 (2026-08-28) against the live keychain. |
| MCP config scope resolution | `config_view::effective_mcp`, `config_view::effective_io`, `mcp_snippet` | `mcpServers`, `getMcpConfigsByScope`, `is malformed` | docs `mcp.md` diff, plus binary: CC's `.mcp.json` reader is `z.object({mcpServers: z.record(...).default({})})` and throws *".mcp.json is malformed (not valid JSON, or mcpServers is not an object)"*. **A bare top-level map is not accepted** — Claudepot once had a fallback that treated one as the server map, which turned a VS Code-style `{"servers": {...}}` into an invented server named `servers`. If CC ever starts accepting another shape, `read_mcp_servers_obj` needs the arm and `McpConfigProblemKind` needs the variant. **Verified 2.1.241 (2026-08-24).** |
| global config file resolution | `paths::global_claude_json_target` | `getGlobalClaudeFile`, `.config.json` | binary strings |
| peer messaging inbox (socket, key, frames) | `peer::wire`, `peer::key`, `peer::client` | `cc-socks`, `messagingSocketPath`, `peerToken`, `peerProtocol`, `uds-messaging`, `skipSlashCommands` | binary: grep `uds-messaging` log lines for the frame dispatch and the line limit; confirm `peerProtocol` is still `1` in a live `~/.claude/sessions/<pid>.json`; re-derive a key filename and check the file exists. Gated on the `agents_cross_session_inbox` flag, so absence is not removal. **Re-verified 2.1.241 (2026-08-23):** `skipSlashCommands:!0` still on the inbox dispatch, control actions still exactly `rename` / `peer_message_status` / `notify_when_idle` / `peer_idle_notice`, and still zero permission frames on the socket — `permission_response` exists but belongs to CC's remote-device WebSocket and the SDK control protocol, neither reachable from here |
| `crossSessionInbound` gate semantics | `peer::inbound::settings`, `peer::inbound::eval` | `crossSessionInbound`, `cross-session-inbound`, `accept`, `hold`, `refuse` | binary: grep `cross-session-inbound` for the mode strings and the user-scope precedence rule; confirm a running session still re-reads the setting **live** (write `accept`, send, remove it, send again). Claudepot's time-boxed grant is only meaningful while that live re-read holds — if CC starts caching the value at startup, expiry becomes advisory and the grant stops closing the door. **2.1.248 changed two things (verified 2.1.250, 2026-08-28):** an unrecognised value is no longer ignored — it now warns and *holds* under user settings, *refuses* under managed settings, so garbage in this key has an active meaning it did not have when `eval::decide`'s supersession check was written; and a refused send now reports back to the sender. The user-scope precedence string is unchanged. |
| held peer-message transcript record | `peer::outcome` | `Held peer message`, `verified pid`, `preview:`, `not delivered to Claude` | binary: grep `Held peer message` for the notice template. `classify` correlates a held notice to *this* send by `[verified pid N]` or by the `preview: «...»` prefix, because the record carries **no uuid**. If either field is dropped or renamed, held notices stop being attributable and the outcome degrades to `Undetermined` rather than lying — but the check should catch it. **Verified 2.1.241 (2026-08-24):** template is `Held peer message — from ${address}${[verified pid N]}${(peer claims name: X)}${; preview: «...»} — not delivered to Claude (N held).` **Re-verified 2.1.250 (2026-08-28), template unchanged.** Also watch the `peer_message_status` receipt, whose status set is now `held \ denied \ expired \ delivered \ refused \ dropped` (`refused` travels as `{status:"expired", status_detail:"refused"}`). Claudepot cannot read it and that is a decision, not a gap — see `peer::wire::encode_line`: the receipt goes to a `from` reply address on a NEW connection, and CC drops it unless that address resolves inside its own socket namespace, so consuming it means binding an inbox in a CC-owned directory. The transcript notice stays the only signal we take. |
| reference placeholders in a prompt (`@file`, `$N`) | `session::title`, `sessions/format.ts` | `parseReferences`, `preExpansionValue`, `$ARGUMENTS` | binary: grep for the placeholder parser. Both the Rust and the TS copy are pinned by `testdata/session-title-vectors.json`; a CC change means new vectors on **both** sides. The old comment cited the 2.1.88 source mirror, which `.claude/rules/cc-upstream-watch.md` forbids as a verification source |
| `PermissionRequest` hook (the remote-approval path) | `remote::approval`, `remote::approval::install` | `PermissionRequest`, `hookSpecificOutput`, `permissionDecision`, `hide`, `BashCommandHookSchema` | binary: confirm `PermissionRequest` is still in the hook-event list, and that its **output** union is still `{behavior:"allow"}` / `{behavior:"deny",message?}` — note `permissionDecision` is documented "PreToolUse only" and is NOT this event's shape, so a copy-paste between the two silently produces a hook whose decision is ignored. Also re-check the timeout clamp (`UQ_`, 300000 ms at 2.1.241) — `approval::HOOK_TIMEOUT_SECS` must stay under it, and `approval::WAIT` under that. **The failure is quiet in the safe direction**: a decision CC stops honouring means the prompt is drawn at the machine as it always was, so nothing breaks loudly. **Verified 2.1.241 (2026-08-23).** |
| slash-command files + plugin resolution | `cc_commands` | `skipSlashCommands`, `preExpansionValue`, `argument-hint`, `allowed-tools`, `installed_plugins.json` | binary: confirm the command predicate is still `startsWith("/") && !skipSlashCommands` and that CC still expands at the input layer (`preExpansionValue` is the tell). On disk: confirm `installed_plugins.json` is still `{version: 2, plugins: {"<name>@<market>": [{scope, projectPath?, installPath}]}}` — the version field is the thing to watch, since a v3 with a different shape reads here as *no plugins at all* and the picker silently loses 147 of its 168 entries. Also re-check the frontmatter keys that cannot travel in an expansion (`allowed-tools`, `model`); a new one that changes execution needs surfacing the same way. **Verified 2.1.241 (2026-08-23).** |
| pending `AskUserQuestion` shape | `remote::panel::ask` | `AskUserQuestion`, `multiSelect`, `questions`, `Your questions have been answered` | parse a transcript that used the tool: the input must still be `{questions:[{question, header, multiSelect, options:[{label, description}]}]}` and the answer must still arrive as a `tool_result` on the same `tool_use_id`. A rename makes `ASK_TOOL` stop matching and the chips silently disappear; a reshape makes `parse_input` return `None`, which is also silent. **Unestablished and worth settling on the same pass:** whether a peer message arriving while a session is blocked on this tool can resolve it at all, or is merely queued. The UI says "handed off" because nobody has measured it |
| PID record status fields | `remote::panel::PanelStatus`, `session_live::types::PidRecord` | `waitingFor`, `statusUpdatedAt`, `peerFeatures`, `nameSource` | read a live `~/.claude/sessions/<pid>.json`: `status` must still be one of `busy` / `idle` / `waiting`. An unknown value falls back to `idle`, so a rename shows every live session as sitting still rather than erroring |
| Desktop OAuth token-cache key | `desktop_backend::token_cache::TOKEN_CACHE_KEYS`, `desktop_identity`, `services::desktop_service` | `oauth:tokenCache`, `oauth:tokenCacheV2`, `tokenCacheV`, `lastKnownAccountUuid` | **Claude Desktop, not Claude Code** — the only row here that watches the other binary, and it drifts on the same terms. Read the key names in `~/Library/Application Support/Claude/config.json` (names only, never values): a `…V3` means a new most-preferred entry at the FRONT of `TOKEN_CACHE_KEYS`. **The trigger is a sign-in, not a version bump**, so an install that has not re-authenticated shows the old shape and the check reads clean on a stale machine — re-authenticate before concluding nothing moved. Failure is silent and wears two faces: a legacy key still holding a token sends it and gets `403`, and a migrated one decrypts to `{}` and reads as "no bundle entries". Also re-check that the decrypted payload is still the keyed scope-bundle map `token_cache` parses; V2's was identical to V1's. **Verified Desktop 1.34493.1 (2026-08-25) by capturing `config.json` either side of a sign-in: V1 1776 b → 28 b (`{}`), V2 absent → 708 b (live token).** github#93 |
| system-record subtypes in transcripts | `remote::panel::transcript::fold` | `stop_hook_summary`, `system`, `level` | the thread view drops `type: "system"` records because `SessionEvent::System` carries only CC's `level`, never the notice body. If a **held peer message** needs surfacing, `parse_line_into` has to carry the body first — check whether the held-message record still has that shape |

## Version pins that go stale silently

These artifacts **disable themselves** on a version mismatch and say
nothing. That is correct behaviour and a reporting gap, so
`cargo xtask cc-drift` reports every one of them against the installed
CC — that command is how you run this whole file:

```bash
cargo xtask cc-drift                       # since the parity pin, via gh
cargo xtask cc-drift --since 2.1.220       # since your last check
cargo xtask cc-drift --changelog /tmp/CHANGELOG.md   # offline
```

| Artifact | Pin field | Failure mode when stale |
|---|---|---|
| `parity-harness/PINNED_CC_VERSION` | the file | fixtures lock historical parity only |
| `crates/claudepot-core/data/cc-env-evidence.json` | `binary_crosscheck_version` | env pane hides `present_in_build` entirely |
| same | `docs_fetched_at` / `docs_sha256` | docs rows drift from the live page |
| same | `cc_source_read_at` | **the one pin with no gate.** `binary_crosscheck_version` disables `present_in_build` on a mismatch (`CrosscheckValidity`); this field is a plain `String` with no consumer, so the `SAFE_ENV_VARS` / `PROVIDER_MANAGED` safety flags — read by `build-cc-env-spec.py` from `~/github/claude_code_src/.../managedEnvConstants.ts`, i.e. the **2.1.88 mirror this file forbids as evidence** — render unconditionally with no staleness signal. Those attributes are load-bearing: `ANTHROPIC_CUSTOM_HEADERS` is pre-trust-safe *and* can carry `Authorization: Bearer …`. The lists are minified out of the binary, so closing this needs a new extraction or an explicit validity gate. |

## Who runs it

A Claudepot **agent** drafted as `cc-upstream-watch`, cron `0 9 1 * *`
(09:00 on the 1st), `permission_mode: dontAsk` with tools narrowed to
`Bash, Read, Write, Glob, Grep`, and the Claudepot memory server
attached so it can read prior evidence and record its own.

It lives in `~/.claudepot/agents.json`, which is outside the repo, so
**this section is the only committed record of it.** Recreate with
`claudepot agent draft --from-json <spec> --attach-memory --drafted-by <id>`;
`cwd` is machine-specific, which is why the spec itself is not committed.

A draft is inert — no scheduler artifact exists until a human arms it in
Claudepot → Agents → "Review & install". That review is the security
gate and no CLI verb bypasses it.

The agent is an **auditor, not a fixer**. Its prompt forbids changing
source, bumping the parity pin, and running `--rebuild-evidence`: a pin
bump invalidates fixtures and rebuilding evidence rewrites a committed
artifact, and both are human calls. The only file it writes is its own
report.

## What a green run looks like

**Record the negative result.** "Checked, unchanged" is what makes a
later red meaningful — a watchlist with no history of passing runs is
indistinguishable from one nobody has run. The monthly report in
`dev-docs/reports/` is where that goes.

## Precedent

`cleanupPeriodDays` is the worked example, found 2026-08-16 by running
this list by hand for twenty minutes. CC had started rejecting `0`; the
control Claudepot shipped for it kept writing `0` behind a
type-to-confirm gate, which inverted its effect from "delete everything"
to "never clean up, and keep writing". It had been wrong for some
unknown part of 145 releases. See `dev-docs/cc-upstream-watch.md` §2.1.

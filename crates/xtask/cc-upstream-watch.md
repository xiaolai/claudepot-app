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
| `cleanupPeriodDays` sweep scope | `cc_sweep::SWEPT` | `shell-snapshots`, `dump-prompts`, `file-history`, `retention sweep` | binary: grep the cleanup module's sweep fns, then reconcile against the `SWEPT` table. A new directory is a new row; a changed sweep unit (files vs subdirs) silently makes a count read zero |
| settings-validation → cleanup suppression | `cc_retention::RetentionMode::Invalid` / `LegacyZero` | `Skipping cleanup`, `validation errors` | binary strings |
| env var catalog + `SAFE_ENV_VARS` | `cc_env`, `data/cc-env-spec.json` | `CLAUDE_*`, `ANTHROPIC_*`, `SAFE_ENV_VARS`, `PROVIDER_MANAGED` | `scripts/build-cc-env-spec.py --rebuild-evidence` then `--check` |
| settings merge precedence | `config_view::effective_settings`, `parity-harness/` | `settings.local.json`, `managed-settings`, `--setting-sources`, `settingSources` | `cargo xtask verify-cc-parity` + re-pin |
| `permissions.defaultMode` | `permission::settings` | `defaultMode`, `bypassPermissions`, `acceptEdits`, `permissionMode` | binary strings for the mode wire strings |
| `availableModels` / `enforceAvailableModels` | `available_models` | `availableModels`, `enforceAvailableModels` | binary strings |
| model ids and rates | `pricing`, `session_live::pricing`, `src/costs.ts` | `claude-opus`, `claude-sonnet`, `claude-fable`, `claude-haiku`, `mythos` | changelog grep; add a `RatePeriod` **and** a vector to `rate-resolution-vectors.json` |
| fast-mode billing (known gap) | `fast_mode_toggle`, `pricing` | `fastMode` | changelog grep |
| auto-update channel | `updates::settings_bridge` | `autoUpdatesChannel`, `minimumVersion`, `autoUpdates` | binary strings |
| auto-memory + consolidation | `settings_writer`, `memory_view`, `auto_dream` | `autoMemoryDirectory`, `autoDreamEnabled` | docs `memory.md` diff |
| artifact toggle | `artifact_toggle` | `enableArtifact`, `disableArtifact` | binary strings |
| commit/PR attribution | `attribution_settings` | `Co-Authored-By`, `includeCoAuthoredBy` | binary strings |
| tips ledger shape | `cc_tips::catalog`, `cc_tips::history` | `tipsHistory`, `numStartups`, `spinnerTipsEnabled` | re-run catalog extraction |
| `claude doctor` output | `cc_doctor` | — | run it, diff against the parser |
| `claude daemon status` output | `cc_daemon` | — | run it, diff against the parser |
| transcript JSONL schema | `session_index`, `session_live`, `corpus` | `sidechain`, `toolUseResult`, `subagents/` | parse newest transcripts; count unknown record kinds |
| path-keyed global state | `project::move_project` P4–P10 | `installed_plugins.json`, `history.jsonl`, `projects[` | `rules/cc-state-move-blast-radius.md` invariant |
| `claude -p` flag surface | `agent::shim` | `--output-format`, `--model`, `--fallback-model`, `--allowedTools`, `--disallowed-tools`, `--system-prompt`, `--append-system-prompt`, `--add-dir`, `--mcp-config`, `--include-partial-messages`, `--bare` | grep `claude --help` per flag |
| credentials keychain item | `cli_backend/keychain` | `Claude Code-credentials` | binary strings |
| MCP config scope resolution | `config_view::effective_mcp`, `mcp_snippet` | `mcpServers`, `getMcpConfigsByScope` | docs `mcp.md` diff |
| global config file resolution | `paths::global_claude_json_target` | `getGlobalClaudeFile`, `.config.json` | binary strings |
| peer messaging inbox (socket, key, frames) | `peer::wire`, `peer::key`, `peer::client` | `cc-socks`, `messagingSocketPath`, `peerToken`, `peerProtocol`, `uds-messaging`, `skipSlashCommands` | binary: grep `uds-messaging` log lines for the frame dispatch and the line limit; confirm `peerProtocol` is still `1` in a live `~/.claude/sessions/<pid>.json`; re-derive a key filename and check the file exists. Gated on the `agents_cross_session_inbox` flag, so absence is not removal. **Re-verified 2.1.241 (2026-08-23):** `skipSlashCommands:!0` still on the inbox dispatch, control actions still exactly `rename` / `peer_message_status` / `notify_when_idle` / `peer_idle_notice`, and still zero permission frames on the socket — `permission_response` exists but belongs to CC's remote-device WebSocket and the SDK control protocol, neither reachable from here |
| `PermissionRequest` hook (the remote-approval path) | `remote::approval`, `remote::approval::install` | `PermissionRequest`, `hookSpecificOutput`, `permissionDecision`, `hide`, `BashCommandHookSchema` | binary: confirm `PermissionRequest` is still in the hook-event list, and that its **output** union is still `{behavior:"allow"}` / `{behavior:"deny",message?}` — note `permissionDecision` is documented "PreToolUse only" and is NOT this event's shape, so a copy-paste between the two silently produces a hook whose decision is ignored. Also re-check the timeout clamp (`UQ_`, 300000 ms at 2.1.241) — `approval::HOOK_TIMEOUT_SECS` must stay under it, and `approval::WAIT` under that. **The failure is quiet in the safe direction**: a decision CC stops honouring means the prompt is drawn at the machine as it always was, so nothing breaks loudly. **Verified 2.1.241 (2026-08-23).** |
| pending `AskUserQuestion` shape | `remote::panel::ask` | `AskUserQuestion`, `multiSelect`, `questions`, `Your questions have been answered` | parse a transcript that used the tool: the input must still be `{questions:[{question, header, multiSelect, options:[{label, description}]}]}` and the answer must still arrive as a `tool_result` on the same `tool_use_id`. A rename makes `ASK_TOOL` stop matching and the chips silently disappear; a reshape makes `parse_input` return `None`, which is also silent. **Unestablished and worth settling on the same pass:** whether a peer message arriving while a session is blocked on this tool can resolve it at all, or is merely queued. The UI says "handed off" because nobody has measured it |
| PID record status fields | `remote::panel::PanelStatus`, `session_live::types::PidRecord` | `waitingFor`, `statusUpdatedAt`, `peerFeatures`, `nameSource` | read a live `~/.claude/sessions/<pid>.json`: `status` must still be one of `busy` / `idle` / `waiting`. An unknown value falls back to `idle`, so a rename shows every live session as sitting still rather than erroring |
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
| same | `cc_source_read_at` | dated against the abandoned mirror |

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

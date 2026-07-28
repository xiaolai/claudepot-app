# Claudepot

Control center for Claude Code and Claude Desktop. Tauri 2 + Rust + React.

The seed was multi-account credential switching. The shipped surface
is broader: accounts, projects, sessions, API keys, third-party
integrations, agents, memory (CLAUDE.md files), usage/cost
tracking, updates, service status, and notifications — all under one
Tauri shell with tray + menubar integration.

The domain model in `claudepot-core` is five nouns: account, cli,
desktop, project, plus **agent** (scheduled headless `claude -p`
runs — the one noun added since the seed; see
`claudepot-core::agent`). Other surfaces are presentation layers
over those nouns and over CC's filesystem, not new domain types. See
`.claude/rules/architecture.md` for the noun-vs-surface distinction.
Scope discipline applies to the *domain model* (don't add nouns
casually); it does not cap what the UI can usefully expose.

## Shared memory (dogfooding)

Claudepot indexes this repo's own Claude + Codex transcripts and
exposes them over MCP. The snippet below tells you which tools exist
and when to call them. It is generated — refresh with
`claudepot mcp install-snippet --out .claude/claudepot-mcp-instructions.md`;
never hand-edit it, and never duplicate it inline.

@.claude/claudepot-mcp-instructions.md

## Build

```bash
cargo check --workspace              # Rust
cargo build -p claudepot-cli         # CLI binary
pnpm build                           # Frontend bundle
pnpm tauri dev                       # GUI in dev mode (hot reload)
pnpm tauri build --no-bundle         # GUI release binary (no .dmg)
```

## Test

```bash
cargo test --workspace               # Rust
cargo xtask verify-cc-parity         # CC settings-merge parity goldens (see parity-harness/README.md)
pnpm test                            # React (Vitest + RTL, jsdom)
pnpm test:coverage                   # React with coverage report
```

CI runs the core + cli tests on a Linux/macOS/Windows matrix and the
`claudepot-tauri` crate's tests on macOS + Windows (Linux needs
webkit2gtk; release.yml's Linux build job is that crate's Linux
compile gate). The lint job fmt/clippy-gates `xtask` itself and runs
`cargo xtask verify-cc-parity`. Release builds preflight a five-site
version lock-step check (tag vs `Cargo.toml`, `package.json`,
`tauri.conf.json`, README status banner, web install-page banner).

## GUI (Tauri)

- `src-tauri/src/commands/` — async Tauri commands wrapping `claudepot-core`,
  sliced by domain (`mod.rs` + one file per surface). NO business logic.
- `src-tauri/src/dto.rs` — serde DTOs crossing to JS. Credentials never cross.
- `src/App.tsx` + `src/api/` (sliced by domain — `account`, `project`,
  `notification`, `activity`, etc., merged in `index.ts`) + `src/types/`
  (sliced by domain, merged in `index.ts`) — React UI, plain CSS.
- `AccountStore.db` is `Mutex<Connection>` so stores can cross `await` points in Tauri commands.
- Seven SQLite files live in `~/.claudepot/` (override with
  `CLAUDEPOT_DATA_DIR`; the authoritative list is whatever joins onto
  `claudepot_core::paths::claudepot_data_dir()`, and
  `cargo xtask verify-docs` fails when this list drifts from it):
  - `accounts.db` — authoritative account + verification state, linked to Keychain.
  - `sessions.db` — persistent cache for the Sessions tab. One row per
    `.jsonl` transcript, keyed by file_path; `(size, mtime_ns)` is the
    re-parse guard. Owned by `claudepot-core::session_index`. Rebuild
    via Settings → Cleanup or `claudepot session rebuild-index`.
  - `env-vault.db` — the local named-secret vault (`env_secrets`
    table, secret in a 0600 column). Owned by
    `claudepot-core::env_vault::store`. Mirrors `keys.db`'s at-rest
    pattern — no OS Keychain. See "## Env secret vault" below.
  - `keys.db` — the Keys tab's API-key inventory. Owned by
    `claudepot-core::keys::store`.
  - `memory_changes.db` — append-only log of detected CLAUDE.md /
    memory-file writes. Owned by `claudepot-core::memory_log`.
  - `activity_metrics.db` — one row per session per tick for the
    Activity Trends view. Owned by
    `claudepot-core::session_live::metrics_store`.
  - `corpus.db` — the **analysis corpus**: every transcript from every
    machine, deduped. Owned by `claudepot-core::corpus`. Built by
    `claudepot corpus index`, which walks the live `~/.claude/projects`
    plus each `~/claude-corpus-archive/<host>/projects/`.

    **Why this is not in `sessions.db`, which is the whole point.**
    `sessions.db` is a *cache of one machine's live `~/.claude`*:
    `SessionIndex::refresh` diffs every row against one `config_dir`
    and deletes the remainder (`codec::delete_row`, cascading turns
    and — via the v4 FK — exchanges / tool_calls / FTS). Correct for a
    cache, fatal for an archive. Point `refresh` at an imported corpus
    and it deletes the live rows; run it again on the live directory
    and it deletes the imported ones. A separate file sits outside that
    loop, so `host_id` costs a column rather than a migration, and the
    file is rebuildable by definition.

    Tables: `corpus_sessions` (deduped by CC session UUID, most
    complete copy wins), `corpus_files` (every physical copy, per
    host), `corpus_exchanges` + `corpus_tool_calls` (turn-level; the
    substrate the detectors read). Derived data — safe to delete, one
    ~5-minute pass to rebuild. Reference machine: 8,249 sessions /
    173,572 tool calls / 848 MB.

    **Outputs do not live here.** Distilled claims go to `memories` in
    `sessions.db`, which carries no foreign key to `sessions` and is
    never cascaded — that asymmetry is what makes the split work.
- A dozen-plus JSON state files also live in `~/.claudepot/`
  (`agents.json`, `routes.json`, `routing-rules.json`, `updates.json`,
  `preferences.json`, `usage-snapshot.json`, `usage_alert_state.json`,
  `agent-events.json`, … — again, the data-dir joins in source are
  authoritative). Stores backed by `claudepot-core::json_store` (the
  six below plus `agent-events.json`) move a corrupt file aside to a
  timestamped `<name>.corrupt.<unix-ts>` and start empty — never
  fatal at boot. Six carry behavior worth documenting here:
  - `notifications.json` — ≤ 500 dispatched toast + OS-banner entries
    surfaced by the WindowChrome bell-icon popover. Owned by
    `claudepot-core::notification_log`. Capture sites: `pushToast` in
    `src/hooks/useToasts.ts` and `dispatchOsNotification` in
    `src/lib/notify.ts`.
  - `rotation-rules.json` — user-authored auto-rotation rules.
    Hand-edit-friendly JSON with `{schema_version, rules: [...]}`.
    Owned by `claudepot-core::rotation::store`. Settings → Rotation
    is the editor; the orchestrator loads the file each
    `usage_snapshot::run_tick`. Empty file or no rules = feature off.
  - `rotation-audit.json` — ≤ 500 rotation outcomes (applied,
    suggested, skipped_*, failed, quarantined) with rule_id +
    from/to + reason. Owned by `claudepot-core::rotation::audit`.
    Rendered in the Settings → Rotation pane's "Recent activity"
    table.
  - `rotation-breaker.json` — per-rule consecutive-failure ledgers
    for the auto-rotation circuit breaker. `{schema_version,
    ledgers: {rule_id: {...}}}`. Owned by
    `claudepot-core::rotation::breaker_store`; the breaker logic is
    pure `claudepot-core::breaker`. A rule that fails to swap 3
    times running is quarantined (skipped before `evaluate`) until
    a 6-hour cooldown probe. Stale rule_ids are pruned each tick.
    Empty file = no failures recorded.
  - `permission-grants.json` — active time-boxed permission grants.
    `{schema_version, grants: [...]}`, one grant per project_path.
    Owned by `claudepot-core::permission::store`. The orchestrator
    reverts expired grants each `usage_snapshot::run_tick`. Empty
    file or no grants = feature off. See "## Permission grants".
  - `pricing-history.json` — observed model-rate changes.
    `{schema_version, observations: [...]}`, appended (never
    overwritten) when a live pricing scrape reports a rate that
    differs from what we already believe. Owned by
    `claudepot-core::pricing::history`. Empty file = no change ever
    observed, and the bundled rate history stands alone. See
    "## Pricing".

## Pricing (Activities → Cost, and every "on API" figure)

Cost figures answer "what would pay-per-call have cost me". Rates are
**dated**, because a rate change must not silently re-score the past.

- `claudepot-core::session_live::pricing` — `RATE_TIERS` is the single
  source of truth: each model carries a list of `RatePeriod`s
  (`starts: Option<Ymd>` + rates), oldest first. `FAMILY_CURRENT` maps
  `claude-<family>-` to the model an unlisted member falls back to;
  it is explicit because a family can span tiers (current Opus at
  $5/$25 vs retired Opus 4.1 at $15/$75).
- `claudepot-core::pricing::PriceBook` — **the** resolution surface.
  `resolve(model, day)` returns rates plus a `RateConfidence`
  (`Exact` | `FamilyEstimate`). Nothing else should resolve rates.
- `claudepot-core::pricing::history` — observed rate changes
  (`pricing-history.json`), merged over the bundled periods. An
  observation dated `D` means "first seen on `D`", an upper bound on
  when the change landed, so observations never override a bundled
  period that already covers that day.
- `src/costs.ts` mirrors `PriceBook::resolve` for client-side
  aggregation. **The two are locked together by
  `crates/claudepot-core/testdata/rate-resolution-vectors.json`** —
  both run those vectors. Change one, change the other, add a vector.
- Family estimates are always marked in the UI (a leading `≈` plus a
  `title`), never presented as a quote. `ProjectUsageRow` carries
  `estimated_sessions` for the same reason.

Known gap: fast mode bills Opus 5 / 4.8 at $10/$50 rather than
$5/$25, and CC's transcripts carry no fast-mode marker, so a
fast-mode session is under-reported.

## Permission grants (ProjectDetail → Permissions)

Optional feature: grant a project a time-boxed
`permissions.defaultMode` (almost always `bypassPermissions`) that
Claudepot auto-reverts on expiry — the elevated state is never
left to memory.

- Pure logic in `claudepot-core::permission`: `mode` (PermissionMode
  over CC's wire strings), `settings` (resolve/read/write the nested
  `permissions.defaultMode` key, format-preserving, refuses the
  committed Project layer), `grants` + `store` (the JSON file),
  `eval` (expiration, clock injected).
- Orchestrator at `src-tauri/src/permission_orchestrator.rs` —
  `tick()` reverts expired grants (skips if the user hand-changed
  the setting since the grant) and emits `permission-reverted`.
  Hooked into `usage_snapshot::run_tick` ahead of the account-state
  early returns. Zero overhead when no grants exist.
- Grants always land in `.claude/settings.local.json`. A project
  elevated by hand-editing settings shows as elevated but *not*
  Claudepot-managed — the UI won't revert someone's own choice.
- CC schema (`permissions.defaultMode`) verified against
  `~/github/claude_code_src/src`.

## Env secret vault (Keys → Secret vault, ProjectDetail → Environment files)

Optional feature: a fully-local named-secret vault plus
format-preserving per-project `.env*` editing — copy a secret out,
inject it into a project's `.env`, comment/uncomment/delete keys.
Movement layer only, not a text editor.

- Pure logic in `claudepot-core::env_vault`: `env_file` (line-
  oriented `.env` editor — every mutation touches only the target
  key's line; `parse` exposes the active/commented/absent
  tri-state), `store` (the SQLite vault).
- Tauri commands in `src-tauri/src/commands/env_secret.rs` —
  `env_vault_*` (vault) and `env_file_*` (per-project). Inbound
  secret args zeroized on every exit path; outbound values cross
  only via the Rust-side clipboard write + `KeyCopyReceiptDto`,
  never rendered. Renderer-supplied `.env` file names are validated
  as safe bare dotenv filenames (no separators / `..` / NUL).

## Auto-rotation (Settings → Rotation)

Optional feature: when the active CLI account's Anthropic
utilization on a configured window crosses a user-set threshold,
swap to a chosen alternate.

- Pure rule logic in `claudepot-core::rotation::eval` —
  `evaluate(rules, snapshot, active, audit, now) -> Vec<RuleDecision>`,
  no I/O. Tests inject the clock.
- Orchestrator at `src-tauri/src/rotation_orchestrator.rs` bridges
  to the Tauri runtime: confirm-mode emits `rotation-suggested`
  events for the toast, auto-mode calls
  `cli_backend::swap::switch_force` directly.
- Hooks into `usage_snapshot::run_tick` (the existing 5-min
  multi-account fetch). Zero overhead when no rules exist.
- Confirm is the default mode; promote to auto after watching the
  rule fire correctly. See `dev-docs/auto-rotation.md` for the
  full design including the policy framing.

## Transcript retention (Settings → Retention)

Reads and writes CC's `cleanupPeriodDays` — **the only Claude Code
setting that destroys user data**, and one CC's own UI never mentions.
Pure logic in `claudepot-core::cc_retention`; commands in
`src-tauri/src/commands/cc_retention.rs`; pane at
`src/sections/settings/RetentionPane.tsx`.

Not named `retention` in core — `claudepot-core::retention` already owns
an unrelated concept (Claudepot's own `activity_cards` / `metrics_tick`
pruning horizon). `cc_` matches `cc_daemon` / `cc_doctor` / `cc_tips`.

Four properties drive the whole design; changing any of them invalidates
the pane:

- **Sliding, not one-shot.** `getCutoffDate()` recomputes
  `now - cleanupPeriodDays` on *every* run, so loss is continuous.
- **`0` is the most destructive value, not "off".** It means *write no
  transcripts and delete the existing ones at startup*. It is therefore
  **not on the duration scale** — `set_retention_days` rejects it and
  `disable_persistence()` is a separately-named call behind a
  type-to-confirm gate. Do not "simplify" these into one setter.
- **A negative value suppresses cleanup entirely.**
  `cleanupOldMessageFilesInBackground` bails when settings fail
  validation *and* the raw key is present, so an invalid value
  accidentally **protects** transcripts (`RetentionMode::Invalid` /
  `cleanup_suppressed`). The UI must say "fix the value", never
  "restore the default" — restoring clears the error and re-arms
  deletion.
- **Invisible on disk.** Cleanup unlinks top-level session transcripts
  and never walks `subagents/`, so the folder grows while history is
  destroyed. `TranscriptRisk::nested_immortal` exists to say so.

`TranscriptRisk::scan_incomplete` is load-bearing: a scan that failed
must never render as "nothing is scheduled for deletion". Boot check at
`src-tauri/src/retention_boot_check.rs` emits one bell entry via
`Category::TranscriptsExpiring`; there is deliberately **no dismissal
flag** — gating on the condition means raising retention silences it,
while dismissing without fixing does not.

## Corpus + detectors (`claudepot corpus`)

`claudepot-core::corpus` builds `corpus.db` (see the data-dir list
above for why it is a separate file). `corpus::normalize` is Tier 0,
`corpus::detect` is Tiers 1–3. No model calls anywhere in this path —
`claudepot corpus detect` is the free preview before any distillation.

Precision, not scale, is the problem: the naive "error then any later
success of the same tool" join yields ~358k useless pairs. The
constraints that make it usable are same-file, same **command family**,
first success only, and a bounded turn gap.

Two normalizers on purpose — `normalize_prompt` flattens numbers
wholesale; `error_signature` keeps small integers, because merging
`Exit code 1` with `Exit code 143` merges "failed" with "timed out".
Signatures take the **first line only**, redacted and capped: tool
output is arbitrary stdout and on a real machine contains financial
records.

Two filters exist because the real corpus demanded them, and removing
either re-floods the output:

- `is_harness_synthetic` — CC injects `<local-command-caveat>`,
  `<command-name>`, `<bash-stdout>`, `[Request interrupted by user]`.
  Before filtering, the largest "repeated request" in the corpus was
  harness plumbing at 1,258 occurrences.
- `command_family` skips segment-consuming shell words (`cd`, `source`)
  and comments. Real commands open `cd "/path"; …`, so a naive first
  token returns `cd`'s *argument*; and `#` produced a phantom `bash:#`
  family with 266 "verified recoveries".

**Vocabulary.** Nothing here is a *recurrence* — that word has a
precise, human-confirmed meaning in `shared_memory::recurrence` and
diluting it breaks the one honest signal the knowledge base has.
Repetition is a *repetition cluster*; a failure with no observed
success is `unresolved`, never "abandoned".

## Proactive token refresh (no UI)

Always-on behavior, not a feature: keeps **inactive** accounts' access
tokens alive so every surface that needs a live token (usage windows,
Activity strip, tray report) doesn't read "Expired" for every account
except the one in use. Access tokens last about an hour; before this,
nothing refreshed a parked slot between an explicit "Verify all" and
the next account switch.

- Pure selection logic in `claudepot-core::token_refresh` —
  `is_eligible(facts, now_ms)` and
  `select_next(candidates, now, min_retry_gap) -> Option<Uuid>`, no
  I/O, clock injected.
- Orchestrator at `src-tauri/src/token_refresh_orchestrator.rs`, hooked
  into `usage_snapshot::run_tick` *before* the usage fetch so an
  account healed this tick reports live numbers in the same tick.
- **Does not implement a refresh.** It picks an account and calls
  `services::identity::verify_account_identity`, whose existing
  401 → refresh → CAS-write path already refuses to persist a rotated
  blob when the profile email drifts from the label. Reimplementing
  the exchange here would fork that protection.
- **Never the active account** — that token belongs to Claude Code,
  which rotates it on its own schedule; refreshing it from a
  background tick is the 0.2.10 sign-out bug. Also skips
  `drift`/`rejected` slots and any token that has not actually
  expired (a live token makes `/profile` return 200, so the refresh
  branch is never reached).
- **One account per tick**, ordered round-robin by last attempt rather
  than by staleness — staleness alone starves, because an account that
  fails every time stays the most stale forever. `reference.md`
  §III.4.1 records the token endpoint refusing three refreshes from one
  IP in ten minutes, so the 5-min cadence is the rate limit; there is
  deliberately no backoff state machine.

## Test on test-host

> Real `<user>`, `<host>`, and `<password>` values live in
> `CLAUDE.local.md` (gitignored). The placeholder shape below is
> the public form.

```bash
cargo build -p claudepot-cli
scp target/debug/claudepot <user>@<host>:/tmp/claudepot
ssh <user>@<host> "security unlock-keychain -p <password> ~/Library/Keychains/login.keychain-db; /tmp/claudepot <command>"
```

Automated login for setting up CC state on test-host:
```bash
ssh <user>@<host> "security unlock-keychain -p <password>; bash /tmp/claude-login-local.sh <email>"
```

## Release validation (Linux + Windows)

CI's clippy + Windows-test gates run on Linux/Windows runners that
local macOS can't reproduce. A four-round cascade of "fix-and-pray"
clippy commits in v0.0.18 prompted this setup:

- **`<runner-a>`** (internal validator network, Ubuntu aarch64) —
  runs the same command as CI's `Format / Clippy (Linux)` job:
  ```bash
  cargo clippy --all-targets -p claudepot-core -p claudepot-cli -- -D warnings
  ```
  Catches new-clippy-version lints (1.95 added `io_other_error`,
  `manual_pattern_char_comparison`; 1.92 added `useless_format`,
  `cloned_ref_to_slice_refs`, `iter_nth_zero`) and
  `cfg(target_os = "macos")`-only items that the macOS-local clippy
  never sees. `--all-targets` covers test-code lints too — without
  it, test-only drift accumulated silently between 1.92 and 1.95
  and surfaced as a 7-lint backlog on 2026-05-13.

- **`<runner-b>`** (internal validator network, Win 11 MSVC x86_64) —
  runs the same compile-step as CI's `Tests (windows-latest)` job:
  ```bash
  cargo test -p claudepot-core -p claudepot-cli --no-run
  ```
  Catches Windows-only compile errors (e.g. types referenced in
  `cfg(target_os = "windows")` arms but cfg-gated to macOS only).

Real host names and the network they sit on live in `CLAUDE.local.md`
(gitignored).

The hook source is committed at `scripts/pre-push`. Install it
per clone with `scripts/install-hooks.sh`. The hook auto-runs both
validators against the pushed SHA when — and only when — the push
contains a `refs/tags/v*` release tag. Branch pushes skip
validation. Failure aborts the push and prints the recovery recipe
(delete tag, fix locally, re-tag, re-push).

**Never hand-symlink the hook into `.git/hooks/`.** A global
`core.hooksPath` — set by the git-lfs installer and most dotfile
setups — makes git ignore `.git/hooks` entirely, so a symlinked hook
reports "Installed" and then never runs. The v0.2.7 … v0.2.10 tags
were all pushed with the validators silently inert for exactly this
reason. `install-hooks.sh` instead points `core.hooksPath` at a
generated, gitignored `.githooks/` (a `--local` setting, so no other
repo is affected) whose hooks call `scripts/<hook>` and then chain to
whatever the clone previously inherited — the global `commit-msg`
and git-lfs hooks keep working. Re-running is safe; the inherited
path is recorded once in `claudepot.inheritedHooksPath`.

Verify an install rather than trusting it: `git config
core.hooksPath` should print a `.githooks` path, and a dry-run push
of a throwaway `v*` tag should print the validator banner.

**When a validator host is unreachable, the hook defers to CI** rather
than failing. CI runs the same two gates on the same commit, so the
hook asks `gh` whether the `ci.yml` run for that commit is green and
accepts it in place of the missing host. Note the asymmetry: a host
that is *reachable and fails* still aborts. Only absence of evidence
falls back, never contrary evidence.

The lookup dereferences `^{commit}` first — an annotated tag's own
object sha differs from the commit's, and CI indexes runs by commit,
so looking up the tag sha would silently never match. It also needs
an authenticated `gh`; an unauthenticated one reads as "no run" and
aborts, which is the safe direction.

This exists because `--no-verify` was becoming the reflex — v0.2.10,
v0.2.11 and v0.2.12 all shipped that way while the validator boxes
were offline. A bypass used routinely is indistinguishable from no
gate at all, which is how these validators sat inert for four
releases. The workflow that keeps the gate real: push the branch,
let CI finish, then push the tag.

Validator hosts are never committed: the hook reads them from the
gitignored `.validator-hosts` file at the repo root (shape documented
in the `scripts/pre-push` header) or from
`CLAUDEPOT_VALIDATOR_LINUX_SSH` / `CLAUDEPOT_VALIDATOR_WINDOWS_SSH`
in the environment. Real host names live in `CLAUDE.local.md`.
Bypass with `git push --no-verify` if a host is unreachable, but
note CI is unforgiving about red main.

## Architecture

See `dev-docs/implementation-plan.md` for the full plan.

- Five nouns: **account**, **cli**, **desktop**, **project**, **agent**
  (see `.claude/rules/architecture.md` for each noun's scope)
- `claudepot-core` = pure Rust library, no Tauri dependency
- `claudepot-cli` = thin clap wrapper over core
- `src-tauri` = Tauri app consuming same core
- `crates/xtask` = workspace automation, currently the CC-parity
  verifier (`cargo xtask verify-cc-parity` over `parity-harness/`)
- Separate keychain surfaces on macOS — CC's item vs Claudepot's own
  slots, `keyring` vs `/usr/bin/security` (see rules/architecture.md)
- Account identity = email, resolved by prefix matching
- GUI is paper-mono shell: custom 38px `WindowChrome` at top
  (breadcrumb + ⌘K palette hint + bell + theme toggle), 240px `Sidebar`
  on the left (swap targets + primary nav + live Activity strip
  + synced strip), content column, 24px `StatusBar` at bottom.
  Primitives live in `src/components/primitives/`. Sections live
  under `src/sections/`; the registry (`src/sections/registry.tsx`)
  is the single source of truth for primary nav. Sections in order:
  Accounts, Activities (id `events` for localStorage compatibility,
  label "Activities" — live + today/month dashboard + cards stream),
  Projects (hosts per-project sessions in ProjectDetail's
  master-detail pane), Knowledge (id `shared-memory` — dashboard,
  curated base, review queue, and recall over indexed Claude + Codex
  transcripts, memories,
  decisions), Keys, Providers (id `third-party`, localStorage
  compatibility), Agents (id `automations`, ditto), Global,
  Settings. Nine top-level tabs total.
  Cleanup (session prune + trash) lives at Settings → Cleanup.
- Long-running ops (project rename, repair resume/rollback) flow
  through a single op-progress pipeline:
  `Tauri *_start` cmd → spawns task → emits events on
  `op-progress::<op_id>` channels → the op-progress modal subscribes
  by op_id. The `RunningOps` map on the backend is the polling
  backstop; see `src-tauri/src/ops.rs`.

## Web (claudepot.com)

`web/` is a self-contained Next.js 15 app that ships
`https://claudepot.com`. Independent install (its own
`package.json` + `pnpm-lock.yaml`); not a workspace member of the
root Tauri app. Two surfaces in one app:

- `/` — **reader**: resource aggregator for one-man companies
  building with AI.
- `/app/*` — **product docs**: 15 routes (landing + why + install
  + 9 features + features index + changelog + download), MDX
  under `web/src/app/(reader)/app/`.

Stack: Next.js 15 + Drizzle/Neon + Auth.js v5 (GitHub + Google +
Resend magic-link) + Resend + boring-avatars. `editorial/` carries
the editorial spec read at runtime by the bot office (a separate
private repo).

Deploy: Vercel project `<vercel-org>/claudepot-com`, Root Directory
`web/`. CF DNS for the `claudepot.com` zone is unproxied A
records to `76.76.21.21`. Phase-1 plan and full migration log in
`dev-docs/archive/domain-realignment.md`.

CI: `.github/workflows/ci-web.yml` runs typecheck + tests on
`web/**` changes (no build — Vercel handles the build per push).

The `web/.tokenize/` config currently runs the hook in
`{"mode": "maintainer", "strictness": "advisory"}` — it flags
hardcoded values but does not block. Promote to strict only after
the residual hardcoded values in the imported codebase are absorbed,
and diff-scan TS/TSX after any `/ui-tokenize:fix` run (the hook has
corrupted non-CSS files before).

## Reference

`dev-docs/kannon/reference.md` — 3400-line verified reference for CC/Desktop internals.
Always verify claims against CC source at `~/github/claude_code_src/src` before coding.

## Icon assets

Full post-mortem of the v0.1.13–0.1.19 Dock-blur arc is in
`dev-docs/icon-design-notes.md`. Load-bearing rules:

- **SVG must use a power-of-2-friendly grid.** Cell sizes 16, 24,
  32, 64 in a 512-px viewBox. Avoid 22, 28, 30 — they don't divide
  128/256 cleanly and rsvg AA-softens at every Dock size.
- **Generate raster icons via `scripts/regen-icons.sh`,
  not `pnpm tauri icon`.** The latter uses lossy resampling for
  some `.icns` layers and produces ~50 dead-byte files for targets
  we don't ship (iOS, Android, MSIX). Our script uses
  `rsvg-convert` + `iconutil` + a manual ICO struct-pack that
  embeds PNG-compressed layers verbatim.
- **`src-tauri/src/dock_icon.rs` calls `setApplicationIconImage`
  with `icon.png` (512×512) at startup on macOS.** This is required
  — Tauri's runtime only does this in dev mode. Without it, prod
  Dock at default size (96 px on Retina) renders the `.icns` 128
  layer downscaled bilinearly and looks visibly soft. The 512-px
  source means every Dock size is a clean Lanczos downsample.
- **`pnpm tauri icon`'s output paths are `.gitignore`'d** so a
  stray invocation can't re-stage MSIX/iOS/Android dead bytes.

## Documentation screenshots

No manual capture, no PII scrubbing:

```bash
cargo xtask screenshot-fixture                  # synthetic profile
pnpm dev &                                      # vite, REAL home
cargo build -p claudepot-tauri                  # REAL home
HOME=/tmp/claudepot-demo-home \
  ./target/debug/claudepot-tauri &              # app only
pnpm screenshots                                # capture all 9
```

The fixture is a **fake `HOME`**, not a pair of env overrides.
`CLAUDE_CONFIG_DIR` + `CLAUDEPOT_DATA_DIR` cover only two of the three
places the app reads — Claude Desktop's directory resolves through
`dirs::data_dir()` with no override and leaked a real account through
the header. `HOME` closes every home-relative path at once.

Two things that look like fussiness and are not:

- **The fake home goes to the app, not the build.** `HOME=… pnpm tauri
  dev` reads better and fails — rustup keeps its default toolchain in
  `$HOME/.rustup`, so the override takes the toolchain with it and
  `cargo metadata` dies before anything compiles.
- **The fixture lives outside the repo** (`/tmp/claudepot-demo-home`).
  The app displays the paths it reads, so an in-repo fixture put
  `/Users/<you>/…/claudepot-app/fixtures/…` on screen in Global →
  Config. No amount of synthetic *data* fixes a leaking *path*.

**Never mask real data to take a screenshot.** It was tried and it is
architecturally wrong: the vocabulary is unbounded and only visible as
you navigate (1 project name found on one surface, 79 across four), and
substring replacement corrupts legitimate UI — a harvested `claude`
turned `.claude/settings.json` into `vector-store/settings.json` and the
`CLAUDE-F…` model badge into `SEARCH-INDEX-F…`. Free text defeats it
entirely. Full reasoning in `crates/xtask/src/screenshot_fixture.rs`.

`scripts/capture-screenshots.mjs` drives the app over the MCP bridge's
WebSocket (plain JSON, no auth) and writes both `assets/screenshots/`
and `web/public/screenshots/`. Node, not xtask, so it needs no new
dependency. Each shot waits for a `settle` string rather than sleeping,
and a pane that never settles is **skipped, never captured blank**.

Adding a screenshot means two edits: a `SHOTS` row in the capture
script, and a `SCREENSHOTS` row in `crates/xtask/src/verify_docs.rs` so
`verify-docs` fails when the UI moves ahead of the image. That check
compares **commit dates, not mtimes** — `git checkout` rewrites mtimes,
which is how eight screenshots sat three months stale unnoticed.

Known limitation: `HOME` does not redirect the macOS keychain, so the
Accounts pane's live credential probe finds nothing and each card shows
"Saved login is missing or broken".

## Conventions

- Grill reports go in `dev-docs/reports/`. Never drop them at the repo root.

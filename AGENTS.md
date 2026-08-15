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
- Eight SQLite files live in `~/.claudepot/` (override with
  `CLAUDEPOT_DATA_DIR`; the authoritative list is whatever joins onto
  `claudepot_core::paths::claudepot_data_dir()`, and
  `cargo xtask verify-docs` fails when this list drifts from it).

  **Every one of them opens through
  `claudepot-core::db_pragmas::apply_standard_pragmas`**, and
  `verify-docs` fails a `Connection::open` that doesn't. Hand-rolling
  the pragma batch is the failure, not getting it wrong: `corpus.rs`
  hand-rolled a batch that *looked* deliberate and silently omitted
  `journal_size_limit` + `wal_autocheckpoint`, leaving the largest
  database in the app outside the bound that exists because
  `sessions.db-wal` once reached 6.3 GB. The helper also retries the
  `delete` → `wal` transition, because SQLite does **not** run the busy
  handler for it — `busy_timeout` does not cover that statement, and
  racing the first open of a file failed outright with "database is
  locked" (measured: 2 failures per 320 concurrent opens). Per-store
  extras like `synchronous=NORMAL` or `foreign_keys=ON` go in a second
  batch *after* the helper, never instead of it.
  - `accounts.db` — authoritative account + verification state, linked to Keychain.
  - `boards.db` — durable agent-written boards (grid spec, typed
    series, rows). Owned by `claudepot-core::board::store`. **User
    data, not a cache**: a board's contents exist nowhere else once
    the writing session ends, so migrations preserve rows and there is
    no automatic pruning. Opened *directly* by every writer — GUI,
    CLI, and the MCP server subprocess — with no IPC channel between
    them, following `sessions.db`'s access pattern. That is a
    deliberate trade whose cost is that `writer_id` is self-reported:
    every surface renders provenance as "Reported by …", never as
    verified identity. See `dev-docs/agent-boards-plan.md` §11.
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
  fatal at boot. Seven carry behavior worth documenting here:
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
  - `migrate-peers.json` — per-`(peer, project)` file fingerprints
    for delta export (`claudepot export --since-peer <id>`).
    `{schema_version, peers: {peer_id: {projects: {cwd: [...]}}}}`.
    Owned by `claudepot-core::migrate::peer`. **Transport state, not
    cache**: it must survive a `sessions.db` rebuild, because
    rebuilding a cache would silently re-send every file to every
    peer — which is why it is its own file rather than a table in
    `sessions.db`, whose documented remedy is "delete and rebuild".
    Empty file = every export is full. Fingerprints are
    `(size, mtime_ns)` rather than a high-water mark, because
    `session slim` rewrites transcripts *smaller* in place and
    retention deletes them outright; a watermark skips both.
  - `automations.json` — the **legacy v1** agents file, read only by
    the v1 → v2 migration in `AgentStore::open_at` and never written.
    v2 is `agents.json`. Kept documented because it still exists on
    any install that predates the rename, and a stray file in the data
    dir with no entry here reads as unexplained.
  - `cc_tips_snapshots.jsonl` — append-only log converting CC's
    counter-only tips state (`tipsHistory`, `numStartups` — integers,
    no timestamps) into wall-clock time. Owned by
    `claudepot-core::cc_tips::history`; see `dev-docs/cc-tips-ledger.md`
    §6. Append-only: deleting it loses the time mapping for past
    counters, which cannot be reconstructed.
  - `doctor-parse-failures.jsonl` — append-only log of inputs
    `cc doctor` could not parse, for diagnosing the user's environment.
    Owned by `claudepot-core::cc_doctor::parse_failures`. Safe to
    delete; it is diagnostic history, not state anything reads back.
  - `pricing-cache.json` — cached result of the live pricing scrape.
    Owned by `claudepot-core::pricing` (`CACHE_FILENAME`). Pure cache:
    safe to delete, refetched on next scrape. Distinct from
    `pricing-history.json`, which is an append-only record of observed
    rate *changes* and is not regenerable — see "## Pricing".
  - `cc_tips_catalog.json` — cache of the tips catalog extracted from
    the CC binary. Owned by `claudepot-core::cc_tips::catalog`. Pure
    cache: safe to delete, rebuilt on next extraction. Resolved
    through `paths::claudepot_data_dir()` rather than a hand-built
    `$HOME/.claudepot` — the hardcoded form bypassed both the
    `CLAUDEPOT_DATA_DIR` override and the test-isolation guard, which
    let a test write into the developer's live data root.

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
- **Any value CC's schema rejects suppresses cleanup entirely.**
  `cleanupOldMessageFilesInBackground` bails when settings fail
  validation *and* the raw key is present, so an invalid value
  accidentally **protects** transcripts (`RetentionMode::Invalid` /
  `cleanup_suppressed`). The UI must say "fix the value", never
  "restore the default" — restoring clears the error and re-arms
  deletion.

  That is a negative number **and** anything that is not an integer:
  `z.number().nonnegative().int()` rejects `"thirty"`, `30.5` and `true`
  alike. This is why `settings_writer::read_i64_setting` returns a
  three-state `SettingValue` rather than an `Option` — collapsing
  "absent" and "present but wrong type" reported a 30-day timer on
  history CC was in fact leaving alone, and pointed the user at the one
  button that starts it. Known limit: CC suppresses cleanup on a
  validation error **anywhere** in the file while this key is present;
  Claudepot models only this key, so a file invalid elsewhere reads as
  "cleanup armed". That errs toward warning about deletion that is not
  happening, which is the safe direction, but it is not complete.
- **Invisible on disk.** Cleanup unlinks top-level session transcripts
  and never walks `subagents/`, so the folder grows while history is
  destroyed. `TranscriptRisk::nested_immortal` exists to say so.

`TranscriptRisk::scan_incomplete` is load-bearing: a scan that failed
must never render as "nothing is scheduled for deletion". Boot check at
`src-tauri/src/retention_boot_check.rs` emits one bell entry via
`Category::TranscriptsExpiring`; there is deliberately **no dismissal
flag** — gating on the condition means raising retention silences it,
while dismissing without fixing does not.

## CC env variables (Global → Config → Env Variables)

Reads and writes the `env` block of `~/.claude/settings.json` — CC's
**officially documented** environment variables only, backed by a
generated spec. Pure logic in `claudepot-core::cc_env` (`spec` /
`settings` / `state` / `errors`); commands in
`src-tauri/src/commands/cc_env.rs`; pane at
`src/sections/config/envvars/`.

The artifact is `crates/claudepot-core/data/cc-env-spec.json`, embedded
with `include_str!` and produced by `scripts/build-cc-env-spec.py` from
committed evidence (`cc-env-evidence.json`). `cargo xtask verify-docs`
re-runs the script with `--check`, which regenerates byte-for-byte and
runs the hand-authored goldens in
`crates/claudepot-core/testdata/cc-env-vectors.json`.

Four properties drive the design; changing any of them invalidates the
pane:

- **Re-apply is additive-only.** CC re-applies `settings.env` to a
  running session with `Object.assign` and nothing else — its own
  comment on `state/onChangeAppState.ts:163` says *"additive-only: new
  vars are added, existing may be overwritten, nothing is deleted."*
  Setting a value is usually live; **clearing one never is**. Every
  clear/restore confirmation says the old value survives until relaunch.
- **Unset ≠ `0`, and neither is `""`.** CC's default for nearly every
  variable is the key being absent. Restore-default therefore *removes
  the key*; writing the documented default would pin today's number into
  settings and override whatever CC changes it to later. An explicit
  empty string is a third state again, so clearing is always its own
  action, never "blank the field".
- **Snapshot ≠ runtime.** `undocumented_in_build` and every
  `present_in_build` flag describe **one** binary and are valid only on
  an exact version match (`spec::CrosscheckValidity`). Undocumented names
  are non-monotonic — CC can rename or delete one in any release — so a
  nearest-version match would be unsound, not approximate. On a
  mismatch the section renders "unavailable for this version" and no
  documented row is hidden or tagged `not in build`.
- **Safety attributes are orthogonal, not tiers.** CC's `SAFE_ENV_VARS`
  answers "safe to apply from an untrusted source"; this pane needs
  "safe to display". `ANTHROPIC_CUSTOM_HEADERS` is both pre-trust-safe
  **and** able to carry `Authorization: Bearer …`. Collapsing the two
  axes leaks it.

`~/.claude.json` carries its own `env` block that CC applies *first*
(`utils/managedEnv.ts:136,188`), so a row with no settings entry reads
"No settings.json override", never "CC default" — the user's shell is a
source we cannot see. v1 is **user scope only**: CC applies just the
`SAFE_ENV_VARS` allowlist from project-scoped settings pre-trust, so a
project-scope editor is a different security design rather than a layer
selector. `CLAUDE_CONFIG_DIR` is read-only here (CC resolves it before
settings load, so writing it splits its own bootstrap).

All writes go through `claudepot-core::settings_mutex`, the one
serialized read-modify-write boundary for CC's settings files — see
below.

## Locating CC's global config — one resolver, two meanings

Two different questions, and picking the wrong one is a silent bug:

| You mean | Call |
|---|---|
| the file at `$HOME/.claude.json` | `paths::claude_json_path()` |
| the file **CC will actually read** | `paths::global_claude_json_target()` (or `resolved_global_claude_json()` when "it doesn't exist" is a distinct answer) |

The second mirrors CC's `getGlobalClaudeFile` (`utils/env.ts:14-26`):
legacy `<config_dir>/.config.json` wins when present, else
`$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`. **Never
hand-roll that three-way check** — it was re-implemented three times
and two copies were wrong. `config_view::effective_io` dropped the
legacy branch while claiming parity in its own comment, so Config →
Effective MCP read the wrong file and showed no user-scope servers
while the preview beside it read the right one; `cc_tips::history`
hardcoded the home sibling, so under `CLAUDE_CONFIG_DIR` the tips
ledger reported `num_startups: 0` forever — into
`cc_tips_snapshots.jsonl`, which is append-only and unreconstructable.
`claude_json_path()` is correct only where the home sibling is
genuinely the target (project-move rewriting the `projects` map).

## Command palette (⌘K)

`src/components/CommandPalette.tsx` + `usePaletteActions` +
`components/palette/rows.ts`. Three properties hold it together:

- **One ordering, not two.** `buildPaletteRows` emits a single
  `rows` array (what renders) and `selectable` (the same list minus
  headings). A row's cursor index is assigned where the row is
  created, so `selectable[i]` *is* the i-th visible row. The
  original bug was two orderings: rows rendered grouped by category
  while Enter indexed the ungrouped production order, so Enter on a
  highlighted "Open Projects" ran "Sign Desktop out". Keyboard and
  mouse now share one `activate(row)` — if you add a row kind, add
  it there, not at a second call site.
- **Deep targets are hidden until you type.** Settings panes and
  Global tabs carry `deep: true`. They are real palette entries but
  listing all 28 on an empty query buries the nine sections.
- **Pane metadata lives outside the lazy sections.**
  `sections/settings/panes.ts` and `sections/global/tabs.ts` hold
  no JSX precisely so the palette can import them without dragging
  the Settings / Global chunks into the main bundle. Deep links
  reach Settings through `triggerSettingsTab` (cold-mount
  sessionStorage hint + hot-mount event) and Global through a
  one-shot `tab:<id>` sub-route that the section consumes and
  clears.

Matching is scored (`lib/paletteScore.ts`), not boolean: tiers are
spaced 100 apart and every within-tier penalty sums to under 100, so
a scattered subsequence can never outrank a real substring hit —
which is what let "Sign Desktop out" answer a search for "set".

Shortcut gating is shared: `isShortcutContextBlocked()` in
`useGlobalShortcuts` is the one predicate for "modal open or input
focused", and `useShellShortcuts` / `useGlobalShortcuts` / the
palette all defer to it. Forking a weaker check is how ⌘K ended up
able to open over an already-open dialog.

## Internationalization (en + zh-CN)

Two locales ship: `en` and `zh-CN`. English is the source of truth
everywhere; a missing translation falls back to English rather than
failing, which is why the catalogs are gated in CI (see below).
Full design in `dev-docs/i18n-plan.md`.

**Three catalogs, three owners.** They are not interchangeable:

- `src/locales/<locale>/<ns>.json` — the React UI, 16 namespaces
  (`common`, `components`, `shell`, `errors`, plus one per section).
  Loaded statically and initialized **synchronously** in
  `src/lib/i18n.ts` (`initAsync: false`) so the first paint is already
  localized. `src/types/i18next.d.ts` types `t()` against the *English*
  catalogs — a key that doesn't exist is a compile error, not a runtime
  English leak. Adding a namespace means editing both files.
- `src-tauri/i18n/<locale>.json` — the Rust-authored surfaces: app
  menu, tray, and the four OS-banner modules. Hand-rolled lookup in
  `src-tauri/src/i18n.rs` (`tr` / `tr1` / `tr_args` / `tr_n`), embedded
  with `include_str!`. Deliberately not a crate dependency: ~120 flat
  keys and zh needs no plural rules.
- **Nothing in `claudepot-core`.** Core's `thiserror` strings stay
  canonical English — the CLI prints them verbatim and the GUI uses
  them as its fallback. Localizing core would fork the CLI's output.

**What stays English, permanently:** CLI stdout/stderr and `--json`,
logs and tracing, core error `Display` text, and technical identifiers
(paths, model ids, CC setting keys like `cleanupPeriodDays`, env var
names, commands the user copies). Localizing a value the user must
type or paste is a bug, not a feature — the retention pane's
type-to-confirm gate is the one deliberate exception, because a zh
user must be able to type the phrase they are shown.

**Load-bearing rules, each learned the hard way:**

- **Module-level label constants freeze the boot language.** A
  `const X = { label: "Foo" }` evaluated at import time never follows a
  language switch. Use a `labelKey` resolved where rendered, or a lazy
  `get label()` — `src/sections/settings/panes.ts` and
  `src/sections/global/tabs.ts` are the reference implementations, and
  they stay JSX-free so the ⌘K palette can import them without
  dragging their section chunks into the main bundle.
- **Locale preference is `Option<String>`, and `None` means follow the
  OS.** Never write a resolved locale back into `preferences.json`, or
  "follow system" stops following. `localStorage` mirrors the
  *preference* purely so first paint is correct before IPC returns;
  `preferences.json` is authoritative.
- **`sys-locale`, not `LANG`.** Dock-launched macOS apps inherit no
  env, so env-var detection silently resolves everyone to English.
- **CJK glyphs come from Sarasa Mono SC**, `unicode-range`-gated in
  `index.html` so an English-only session never downloads ~9 MB.
  JetBrains Mono has no CJK coverage — this also fixes Chinese
  *project names* in the English UI, which were falling back to a
  proportional system face.
- **Section labels live in `shell:sections.*`, keyed by registry
  `labelKey`.** Log tags and `ErrorBoundary` labels use the section
  `id` instead — machine-facing strings must not move with the UI
  language.
- **Notification category names key off the category id**
  (`src/lib/notifications/labels.ts`), not the English label core
  ships over IPC. The fixture test in
  `src/lib/notifications/types.test.ts` fails when a new core category
  lacks catalog entries — that is the moment the English fallback
  would start leaking into a zh UI.

**The gate:** `pnpm check:catalogs` (`scripts/check-catalogs.mjs`, wired
into `ci.yml`) enforces en↔zh key parity, `{{placeholder}}` parity,
`<Trans>` tag parity, no orphans, no empty values, valid JSON.

**"Orphan" there means a zh key with no en counterpart — a *cross-locale*
check.** It does not detect a key that no source file references, and a
green run is not evidence there are none: deleting the Activities live
surfaces left 30 dead keys behind and the gate stayed green. Prune them
by hand when you delete a component. A mechanical check is not offered
because ~450 keys resolve dynamically — the whole `errors` namespace is
keyed by Rust error code — so a reference scan would report hundreds of
live keys as dead, and a gate that cries wolf gets bypassed. Plural
parity is asserted on plural *bases*, since zh legitimately carries
only `_other` where en carries `_one` + `_other`. Point
`CLAUDEPOT_LOCALES_DIR` at a fixture to exercise the gate itself — a
check nobody has watched fail is indistinguishable from one that
cannot fail.

`check:envvar-layout` was the standing example of that decay: it drives
the real app over the debug-only MCP bridge, so CI cannot run it and
nobody had ever seen it go red. It now has both halves covered —
`node scripts/check-envvar-layout.mjs --self-test` forces `.envvar-list`
to 0px against the live pane and fails if the assertions *don't* fire,
and `scripts/check-envvar-layout.test.mjs` unit-tests the pure
`evaluate()` half in CI. Split the judgement out of the measurement in
any guard of this shape; the measurement may need a screen, the
judgement never does.

## Settings-file mutation boundary

`claudepot-core::settings_mutex::mutate_settings_file` is the **only**
sanctioned way to read-modify-write a CC settings JSON file. Every
same-process writer is on it: `settings_writer` (and therefore
retention, models, attribution, fast mode, artifact and memory),
`updates::settings_bridge`, `permission::settings`, and `cc_env`.

Atomic rename gives crash-safety, not concurrency-safety — two
overlapping RMWs both read the old bytes and the later rename silently
discards the earlier mutation. The boundary adds a per-path mutex plus a
re-read-and-rebase retry.

Be exact about the limit: **same-process writes are serialized; external
ones cannot be.** CC itself and a user's text editor do not honor
Claudepot's mutex, so those get change-detection and a rebase retry, not
mutual exclusion. A new writer of these files that does its own
read-modify-write is a review finding: a lock only one participant holds
is not a lock.

The same rule covers *multi-key* edits. A transition that changes two
keys together belongs in **one** closure, not two `write_*` calls —
`settings_bridge::change_channel` moves `autoUpdatesChannel` and
`minimumVersion` together for exactly this reason, since a failure
between two writes leaves a half-applied state from a function whose own
contract calls the choice atomic. Reading current state to *decide* what
to write belongs inside the closure too; deciding from a snapshot taken
outside it is a race by construction.

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
  Boards (id `boards` — durable agent-written surfaces; see
  `claudepot-core::board`), Settings.
  **Ten** top-level tabs, one of which (Boards) is **off by default**
  and toggled with ⌃⌥⌘B or Settings → General. The enabled list lives
  in `src/lib/optionalSections.ts`, and every consumer — sidebar,
  palette, ⌘ bindings, shortcuts modal, launch picker, deep-link
  bridges — derives from it. Filtering only the sidebar would leave a
  hidden section still reachable by ⌘9 and ⌘K, which is worse than
  either state.
  Boards sits ninth on purpose: `useSection`
  binds ⌘1..⌘9 to the first nine, so that position gives it ⌘9 and
  pushes Settings to tenth, which costs nothing because Settings has
  its own ⌘, in `useShellShortcuts`.
  Cleanup (session prune + trash) lives at Settings → Cleanup.
- Everything that enumerates the sections reads
  `sections` from the registry — the ⌘K palette
  (`usePaletteActions`), the ⌘1..⌘9 bindings (`useSection`), the
  shortcuts reference (`ShortcutsModal`), and Settings → General's
  "Open on launch" picker. Each of those four used to carry its own
  hand-written copy, and three of the four had drifted: the modal
  documented ⌘3 as "Sessions" and ⌘4 as "Config" (neither is a
  section), the launch picker offered a `sessions` id that
  `useSection` silently rejected back to Accounts, and the palette
  reached three of the nine sections. A new section is one registry
  entry; a new *list* of sections is a review finding.
- Long-running ops (project rename, repair resume/rollback) flow
  through a single op-progress pipeline:
  `Tauri *_start` cmd → spawns task → emits events on
  `op-progress::<op_id>` channels → the op-progress modal subscribes
  by op_id. The `RunningOps` map on the backend is the polling
  backstop; see `src-tauri/src/ops.rs`.
- **Every event channel `src-tauri/src/events.rs` declares must have a
  subscriber in `src/`**, and `cargo xtask verify-docs` fails when one
  doesn't. Seven had none: the four tray→Desktop channels
  (`tray-desktop-switched`, `tray-desktop-switch-failed`,
  `tray-desktop-launch-failed`, `desktop-reconciled`), so a tray
  Desktop swap left the account cards stale and a *failed* one produced
  no toast, no banner, nothing — while the CLI sibling had toast, OS
  banner and Undo; plus three (`desktop-adopted`, `desktop-cleared`,
  `desktop-running-changed`) that only re-announced what the invoking
  command had already returned, now deleted. `events.rs` had a test
  called "wire-contract lock" that compared each constant to its own
  literal — a tautology inside one crate cannot see the far end of a
  cross-boundary contract. Deliberate non-subscriptions go in
  `UNSUBSCRIBED_BY_DESIGN` in `verify_docs.rs` **with the reason**;
  entries there are validated in both directions, so one cannot outlive
  its rationale. A tray action emits because nothing returns to a
  caller — if nobody listens, the click silently does nothing.

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

`pnpm test` here is `node scripts/run-tests.mjs`, which **discovers**
`tests/*.test.ts` rather than listing them. It used to be 23 filenames
chained with `&&`, so adding a test file did not add it to the suite —
and three never ran anywhere: `username.test.ts` (reserved names and
self-rename cooldown, the impersonation surface of a public site),
`editorial-routing.test.ts`, and `social-format.test.ts`. All three
passed once run, which is the bad case: 45 assertions looked like
coverage while CI was green without them. A list of files is a cache of
the directory; read the directory. `tests/integration/` stays out on
purpose — it needs `--env-file=.env.local` and a live Neon connection,
so it keeps its own `test:integration` script.

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
`dev-docs/icon-design-notes.md`.

**The authored set lives in `assets/icon-set/`** — isometric block on
an anodised plate, every coordinate a multiple of 16 on a 1024 grid.
That directory is the source; `src-tauri/icons/` holds only what
`scripts/regen-icons.sh` derives from it, plus the two masters the
script reads directly (`icon.svg`, `icon-flat.svg`). There is
deliberately no second copy of the artwork anywhere: the previous
`pixel-*` masters were deleted when this landed rather than left
beside it, because two plausible masters in one directory is how the
wrong one gets regenerated from.

Load-bearing rules:

- **SVG must use a power-of-2-friendly grid.** The current set is on
  16-unit multiples in a 1024 viewBox. Avoid 22, 28, 30 — they don't
  divide 128/256 cleanly and rsvg AA-softens at every Dock size.
- **Generate raster icons via `scripts/regen-icons.sh`,
  not `pnpm tauri icon`.** The latter uses lossy resampling for
  some `.icns` layers and produces ~50 dead-byte files for targets
  we don't ship (iOS, Android, MSIX). Our script uses
  `rsvg-convert` + `iconutil` + a manual ICO struct-pack that
  embeds PNG-compressed layers verbatim.
- **Three masters, not one, and the split is not cosmetic:**
  - `icon.svg` — plated master with an `feTurbulence` grain, used at
    48 px and up.
  - `icon-flat.svg` — same artwork, solid plate, no filter. Used
    **below 48 px**. The grain is computed at render size, so it
    coarsens relative to the tile as the tile shrinks and reads as
    dirt rather than as a finish. A single-source ladder cannot
    express this.
  - `assets/icon-set/windows/icon-glyph.svg` — plateless, for
    `icon.ico`. Windows draws no enclosure and shows the icon against
    chrome of every shade, so the plate would read as a grey card
    floating behind the block.
- **Tray icons are generated too** (`tray-icon{,Alert}{Template,Mono}@2x.png`,
  44×44). Template is inverted by macOS to match the menubar; Mono is
  the same alpha filled `#808080` because Windows and Linux have no
  template concept and a pure-black glyph vanishes on a dark taskbar.
  - **`tray-icon` normalises the tile to 18 points tall, so the tile's
    pixel size is irrelevant and its padding is pure loss.** The crate
    hard-codes `let icon_height: f64 = 18.0` and derives width from the
    aspect ratio, so only the FRACTION of the tile the glyph inks decides
    how large it lands in the menubar. Measured against its neighbours:
    ChatGPT.app inks 94.4% and renders 17.0pt; Claude.app inks 70.8% and
    renders 12.8pt. Claudepot inked 77% and rendered 13.9pt — visibly
    smaller, while passing every dimension check, because those checks
    asserted a tile fraction rather than the thing the user sees. The
    tray SVGs carry a cropped `viewBox` (672 of the 1024 authoring
    canvas) and now render 16.4pt. Both variants share one viewBox
    *size* so the block does not change scale when the alert badge
    appears, and **the badge sets the floor on how tight the crop can
    go** — it moved inward to (692, 332) to buy it. Its margin is derived
    in RENDERED PIXELS and converted back: a unit here is 44/672 px, so a
    first attempt at a 12-unit margin measured 0.79px, antialiasing
    closed it, and the badge rendered touching two tile edges.
    `verify-icons.py` asserts the rendered POINT HEIGHT and that nothing
    touches the tile edge — every dimension check passed while the icon
    was too small, so the rendered-height assertion is the one that
    matters.
- **`scripts/verify-icons.py` is the structural gate** — 58 checks over
  the PNG ladder, the `.icns` layer list, ICO layer encoding, tray
  sizes, and the grain floor. It catches the failures that still look
  like valid files on disk: an ICO whose layers are raw BMP, an `.icns`
  missing the 128/256 layers the Dock reaches for, a small raster that
  kept the grain. Run it after any icon change. It needs no GUI; what
  it explicitly does **not** check is how the artwork looks, which is
  what launching the app is for.
- **The bundle path and the `setIcon` path want OPPOSITE artwork, and
  swapping them is the classic macOS icon bug:**

  | Path | Wants |
  |---|---|
  | bundle `.icns` / `bundle.icon` list | **full bleed** — macOS applies the squircle mask, inset and shadow |
  | `setApplicationIconImage` (`dock_icon.rs`) | **everything already applied** — drawn verbatim at slot size, no mask, no inset, no shadow |

  So `dock_icon.rs` embeds `icon-dock.png`, **not** `icon.png`: 1024
  canvas, artwork inset to 824/1024 = 0.805 (Apple's measured tile
  fraction), superellipse corner (`|x/a|^n + |y/a|^n = 1`, n = 5) rather
  than a circular arc, which meets the straight edge with a curvature
  discontinuity and reads boxy beside real icons. A full-bleed image on
  this path renders as a hard square measured **~22% larger** than every
  neighbouring Dock icon.

  The pre-2026-08 artwork hid the distinction by baking a squircle into
  the SVG at 416/512 = 0.813 of the canvas, so one file happened to
  serve both roles. The current set is full-bleed by design — correct
  for the bundle — which is exactly why the second asset now exists.
  Reference: `~/.claude/agents/icon-smith/specs.md`, measured against
  macOS 26.5.
- **`src-tauri/src/dock_icon.rs` calls `setApplicationIconImage`
  at startup on macOS.** This is required — Tauri's runtime only does
  this in dev mode. Without it, prod Dock at default size (96 px on
  Retina) renders the `.icns` 128 layer downscaled bilinearly and looks
  visibly soft. The 1024-px source means every Dock size is a clean
  Lanczos downsample.
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
script, and a `SCREENSHOTS` row in `crates/xtask/src/verify_docs.rs`.

Two checks read that table, and the split is deliberate:

- **`cargo xtask verify-docs`** (runs in CI) asserts each shot exists and
  that `assets/screenshots/` and `web/public/screenshots/` hold the same
  bytes. Content-based, no false positives, and the fix is a file copy —
  something a red CI run can actually ask you for.
- **`cargo xtask verify-screenshots`** (**on demand**, not a PR gate)
  reports shots whose sources have moved since capture. Run it before a
  release, or after changing a view you know is captured.

Freshness is not a gate for two reasons. It compares **commit dates, not
mtimes** — `git checkout` rewrites mtimes, which is how eight screenshots
sat three months stale unnoticed — but it compares them per *directory*,
so any edit under `src/sections/projects` reads as "the UI changed",
including edits to views no screenshot shows. And re-capturing needs a
macOS GUI session, a Vite server, a debug build carrying the MCP bridge
and a windowed app; CI has none of them, so a failure there is a wall
rather than a signal. A gate whose remedy cannot run where it fires is
the dynamic that made `--no-verify` a reflex for the release validators.

Adjacency is not staleness. When `verify-screenshots` flags a shot whose
captured view provably did not move, that is the check being coarse —
say so, rather than re-capturing to silence it.

Known limitation: `HOME` does not redirect the macOS keychain, so the
Accounts pane's live credential probe finds nothing and each card shows
"Saved login is missing or broken".

## Conventions

- Grill reports go in `dev-docs/reports/`. Never drop them at the repo root.

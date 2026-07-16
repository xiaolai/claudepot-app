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
- Six SQLite files live in `~/.claudepot/` (override with
  `CLAUDEPOT_DATA_DIR`; the authoritative list is whatever joins onto
  `claudepot_core::paths::claudepot_data_dir()`):
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
- A dozen-plus JSON state files also live in `~/.claudepot/`
  (`agents.json`, `routes.json`, `routing-rules.json`, `updates.json`,
  `preferences.json`, `usage-snapshot.json`, `usage_alert_state.json`,
  `agent-events.json`, … — again, the data-dir joins in source are
  authoritative). Stores backed by `claudepot-core::json_store` (the
  five below plus `agent-events.json`) move a corrupt file aside to a
  timestamped `<name>.corrupt.<unix-ts>` and start empty — never
  fatal at boot. Five carry behavior worth documenting here:
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
per clone with `scripts/install-hooks.sh` (which runs
`ln -sf ../../scripts/pre-push .git/hooks/pre-push`). The hook
auto-runs both validators against the pushed SHA when — and only
when — the push contains a `refs/tags/v*` release tag. Branch pushes
skip validation. Failure aborts the push and prints the recovery
recipe (delete tag, fix locally, re-tag, re-push).

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

## Conventions

- Grill reports go in `dev-docs/reports/`. Never drop them at the repo root.

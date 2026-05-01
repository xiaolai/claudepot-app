# Claudepot

Multi-account Claude Code / Claude Desktop switcher. Tauri 2 + Rust + React.

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
pnpm test                            # React (Vitest + RTL, jsdom)
pnpm test:coverage                   # React with coverage report
```

## GUI (Tauri)

- `src-tauri/src/commands.rs` — async Tauri commands wrapping `claudepot-core`. NO business logic.
- `src-tauri/src/dto.rs` — serde DTOs crossing to JS. Credentials never cross.
- `src/App.tsx` + `src/api/` (sliced by domain — `account`, `project`,
  `notification`, `activity`, etc., merged in `index.ts`) + `src/types.ts` — React UI, plain CSS.
- `AccountStore.db` is `Mutex<Connection>` so stores can cross `await` points in Tauri commands.
- Two SQLite files live in `~/.claudepot/` (override with `CLAUDEPOT_DATA_DIR`):
  - `accounts.db` — authoritative account + verification state, linked to Keychain.
  - `sessions.db` — persistent cache for the Sessions tab. One row per
    `.jsonl` transcript, keyed by file_path; `(size, mtime_ns)` is the
    re-parse guard. Owned by `claudepot-core::session_index`. Rebuild
    via Settings → Cleanup or `claudepot session rebuild-index`.
- One JSON ring buffer also lives in `~/.claudepot/`:
  - `notifications.json` — ≤ 500 dispatched toast + OS-banner entries
    surfaced by the WindowChrome bell-icon popover. Owned by
    `claudepot-core::notification_log`. Capture sites: `pushToast` in
    `src/hooks/useToasts.ts` and `dispatchOsNotification` in
    `src/lib/notify.ts`. Corrupt files are moved aside to
    `notifications.json.corrupt` and the log starts empty — never
    fatal at boot.

## Test on test-host

```bash
cargo build -p claudepot-cli
scp target/debug/claudepot joker@192.0.2.1:/tmp/claudepot
ssh joker@192.0.2.1 "security unlock-keychain -p <password> ~/Library/Keychains/login.keychain-db; /tmp/claudepot <command>"
```

Automated login for setting up CC state on test-host:
```bash
ssh joker@192.0.2.1 "security unlock-keychain -p <password>; bash /tmp/claude-login-local.sh <email>"
```

## Architecture

See `dev-docs/implementation-plan.md` for the full plan.

- Four nouns: **account**, **cli**, **desktop**, **project**
- `claudepot-core` = pure Rust library, no Tauri dependency
- `claudepot-cli` = thin clap wrapper over core
- `src-tauri` = Tauri app consuming same core
- Two separate keychain surfaces on macOS (see rules/architecture.md)
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
  master-detail pane), Keys, Third-parties, Automations, Global,
  Settings. Eight top-level tabs total.
  Cleanup (session prune + trash) lives at Settings → Cleanup.
- Long-running ops (project rename, repair resume/rollback) flow
  through a single op-progress pipeline:
  `Tauri *_start` cmd → spawns task → emits events on
  `op-progress::<op_id>` channels → the op-progress modal subscribes
  by op_id. The `RunningOps` map on the backend is the polling
  backstop; see `src-tauri/src/ops.rs`.

## Reference

`dev-docs/kannon/reference.md` — 3400-line verified reference for CC/Desktop internals.
Always verify claims against CC source at `~/github/claude_code_src/src` before coding.

## Conventions

- Grill reports go in `dev-docs/reports/`. Never drop them at the repo root.

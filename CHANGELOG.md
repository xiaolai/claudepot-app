# Changelog

Versioning scheme:

- `0.0.x` — alpha
- `0.1.x` — beta
- `1.0.0+` — stable

## 0.0.3 — alpha (unreleased)

### Added

- **`session slim --strip-images` / `--strip-documents`**: drop
  base64 image and document payloads from closed session transcripts,
  replacing each block with a `[image]` / `[document]` text stub.
  Mirrors Claude Code's own `stripImagesFromMessages` transform, so
  `claude --resume` loads cleanly minus the ~2000-token-per-image
  cost. Reuses the existing `TrashKind::Slim` pre-slim snapshot for
  reversibility. Only touches `message.content` blocks that reach
  the API — `toolUseResult` display-only sidecars are intentionally
  left intact so the transcript viewer still renders images.

### Fixed

- **Slim reversibility**: `session slim --execute` previously stored
  the throwaway snapshot temp path in the trash manifest, so
  `session trash restore` would have recreated the file at
  `<session>.pre-slim.jsonl` instead of overwriting the real
  session. `TrashPut` now carries a separate `restore_path`
  field for this case. Any slim entries produced before this fix
  restore to the wrong path — empty the trash if you have any.
- **Slim atomicity**: a second `(size, mtime_ns)` re-stat now runs
  immediately before the atomic rename, narrowing the TOCTOU window
  against a concurrent Claude Code appender. Temp files and snapshot
  files are cleaned up via RAII guards on every error path.

## 0.0.2 — alpha (unreleased)

### Added

- **Activity system** (M1–M5): live session tracking with OS
  notifications, tray submenu, durable metrics store, and a Trends
  view. Status bar surfaces the live count (⌘⇧L toggles the pane).
- **Sessions SQLite index**: `sessions.db` caches transcript metadata
  keyed by file path, guarded by `(size, mtime_ns, inode)` so CC
  compaction and `session_move` rewrites don't poison the cache.
  Rebuildable via Settings → Cleanup or `claudepot session
  rebuild-index`.
- **Paper-mono shell**: single typeface (JetBrainsMono Nerd Font),
  Lucide SVG icons, warm OKLCH palette, tokens centralised in
  `src/styles/tokens.css`. Primitives: `Button`, `IconButton`,
  `Glyph`, `Avatar`, `Tag`, `Modal`, `SidebarItem`, `SectionLabel`,
  `Toast`.
- **Sidebar LIVE strip**: presence-only indicator (dot · project ·
  model) sorted by priority tier then recency, hides long-idle
  sessions.
- **External-link handling**: `tauri-plugin-opener` wired, scope
  narrowed to `https://console.anthropic.com/settings/keys`.
- **Session-live redaction**: `sk-ant-*`, Authorization headers,
  JWTs, URL params, and cookies are stripped before events reach
  the UI or the index.

### Fixed

- Opener scope tightened (security) — was open, now limited to the
  Anthropic key-settings URL.
- Clipboard clear guarded on readback so it can't wipe an unrelated
  payload.
- Font preloads now consumed (moved `@font-face` to `index.html`).
- Activity current-transcript resolution no longer reads a stale
  PID-file `sessionId`.
- Paper-mono buttons render the design-system focus ring.
- `dialog:allow-save` granted so session export opens the native
  save picker.

### Security

- `first_user_prompt` stored in `sessions.db` is now redacted for
  `sk-ant-*` tokens before insert.
- WAL/SHM sidecars are forced to materialise then chmod'd 0600
  alongside the main SQLite file.
- `SessionIndex` recovers from mutex poison instead of panicking,
  and recovers from `SQLITE_CORRUPT` by moving the DB aside and
  rebuilding.

### Developer

- Vite dev server moves to `:1430` (HMR `:1431`) so port `:1420`
  stays free for vmark.
- React tests: 196 passing across 25 files.
- Rust workspace tests: all green, including activity E2E and
  session_index round-trip + edge cases.

## 0.0.1 — alpha seed (not published)

Initial CLI + GUI skeleton. Four nouns (`account`, `cli`, `desktop`,
`project`). `claudepot-core` / `claudepot-cli` / `claudepot-tauri`
crate split. Keychain-backed credential store with two surfaces
(keyring crate for Claudepot's own secrets, `/usr/bin/security` for
`Claude Code-credentials`). Email-prefix account resolution.

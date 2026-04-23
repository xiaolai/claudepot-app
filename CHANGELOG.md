# Changelog

Versioning scheme:

- `0.0.x` — alpha
- `0.1.x` — beta
- `1.0.0+` — stable

## 0.0.4 — alpha (unreleased)

### Added

- **Claude Desktop feature overhaul — end-to-end parity with CLI.**
  Landed in seven phases across macOS + Windows, reviewed by Codex
  MCP twice (plan review → implementation review → follow-up). See
  `dev-docs/desktop-feature-overhaul-plan.md` for the full spec.
  - **Live-identity probing**: org-UUID fast path (candidate,
    unverified) + decrypted `oauth:tokenCache` + `/profile` slow
    path (authoritative). `ProbeMethod` tags every result so
    mutating callers can enforce the Decrypted-only gate at
    compile time via the private `VerifiedIdentity` constructor.
  - **Mutators**: `desktop adopt`, `desktop clear`, `desktop
    reconcile`, plus Tauri commands for each. All gated on a
    cross-process advisory flock (`~/.claudepot/desktop.lock`)
    AND a process-wide async Mutex (`DesktopOpState`) so CLI +
    GUI + tray can't race.
  - **macOS crypto**: `security find-generic-password -s "Claude
    Safe Storage" -a "Claude Key" -w` keychain subprocess →
    PBKDF2-HMAC-SHA1 (1003 iters, "saltysalt") → AES-128-CBC
    with fixed `b' ' * 16` IV.
  - **Windows crypto**: `Local State → os_crypt.encrypted_key` →
    DPAPI `CryptUnprotectData` → 32-byte AES-256-GCM key →
    decrypt the `v10`-prefixed ciphertext.
  - **Windows DPAPI invalidation detection**: pre-restore probe
    attempts real ciphertext decryption of the stored snapshot's
    `oauth:tokenCache`. Catches the post-machine-migration case
    where Local State unwraps cleanly but stored blobs are bound
    to the old DPAPI master key. Surfaces as `DpapiInvalidated`
    with a "re-sign in, then re-bind" message.
  - **Windows package discovery**: runtime `Get-AppxPackage
    Claude_*` probe with `once_cell` cache. Drops the hard-coded
    `Claude_pzs8sxrjxfjjc` from four production sites.
  - **UI**: `DesktopConfirmDialog` for destructive actions (sign
    out, overwrite-profile); `DesktopImportCard` in the
    Add-account modal; sync-driven Desktop banners
    (adoption-available, stranger, candidate-only) in the shell's
    status-issues strip; Bind/Sign-out items across context menu
    + command palette + tray.
  - **Sidecar metadata**: `claudepot.profile.json` written inside
    every snapshot dir with `captured_at`, `captured_from_email`,
    `captured_verified`, version, platform, session-items list.
    Survives dir `mtime` drift.
  - **Tray parity**: active-display row falls back to Desktop
    when CLI is unbound; Set-Desktop submenu gains Bind / Sign
    out / Launch / Reconcile header items; per-account rows gate
    on disk truth (`desktop_profile_on_disk`).
  - **Window-focus sync**: startup probe runs unthrottled,
    subsequent window-focus probes respect a 5-minute cooldown.

### Changed

- `AccountSummary` gains `desktop_profile_on_disk` (disk truth)
  alongside `has_desktop_profile` (DB cache). UI must prefer the
  disk-truth field when gating Desktop affordances. `account_list`
  opportunistically reconciles the DB flag against disk on every
  call; `desktop reconcile` surfaces the outcome explicitly.
- `app_status.desktop_installed` now uses the authoritative
  `is_installed()` (macOS: `/Applications/Claude.app`; Windows:
  MSIX package dir) instead of collapsing "installed" and "has
  a data_dir" into one disk check.
- Context-menu "sign in via Desktop app first" disabled reason
  (misleading — signing in via Desktop didn't help Claudepot)
  replaced with "bind current Desktop session first".
- Desktop switch (`desktop_use` + CLI + tray) now acquires the
  operation lock. Previously bypassed it, letting adopt/clear/
  switch race across surfaces.

### Fixed

- `trash.rs::inode_of` on Windows used `MetadataExt::file_index`
  (nightly-only, rust-lang/rust#63010). Broke every Windows build.
  Now returns 0 (graceful degradation: no hardlink dedupe on
  Windows until the stable API lands).
- `account_list` pre-PR only reconciled `has_cli_credentials`,
  leaving `has_desktop_profile` to drift indefinitely once a
  snapshot went missing out-of-band. Now also flips the Desktop
  flag to match disk.
- Adopt with `overwrite=true` previously deleted the old profile
  BEFORE attempting the new snapshot; a snapshot failure left the
  user with NO profile. Now stages the old profile to a tempdir
  and rolls back on snapshot failure.

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

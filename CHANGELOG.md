# Changelog

Versioning scheme:

- `0.0.x` — alpha
- `0.1.x` — beta
- `1.0.0+` — stable

## 0.1.37 — beta (unreleased)

Recovery release. v0.1.36's release.yml failed silently on
`windows-latest` (nightly-only `file_index()` from the v0.1.35
shared-memory work — E0658), so no `.msi`/`.exe` installers shipped
for either v0.1.35 or v0.1.36 even though both tags exist. v0.1.37
bundles the Windows-compat fix plus everything that was supposed to
ship in 0.1.35–0.1.36: the section-navigation flash fix, the
shell-wide render-storm fix that surfaced as a "shaking" Sessions
section in Projects detail, and two web/post styling touch-ups.

### Fixed

- **Windows release build no longer fails on `file_index()`.** The
  v0.1.35 shared-memory work introduced
  `std::os::windows::fs::MetadataExt::file_index()` in two places to
  populate the `(size, mtime_ns, inode)` staleness triple in
  `sessions.db`. That method sits behind
  `#![feature(windows_by_handle)]` (rust-lang/rust#63010) and breaks
  stable rustc with E0658 — which silently killed the Windows job
  in the v0.1.36 release matrix. Both call sites now use stable
  `creation_time()` instead, which has strictly safer semantics for
  the equality-only triple: it changes when a file is created or
  replaced, stays constant across in-place modifications. macOS and
  Linux paths are unchanged.
- **Section navigation no longer flashes a blank panel.** The earlier
  cold-start fix (boot skeleton in `index.html`, preloaded saved
  section) only addressed the first paint — clicking from Accounts
  to any other top-level section still entered `<Suspense
  fallback={null}>` while the lazy chunk was being fetched, painting
  the content column blank for one frame. Two coordinated changes
  close the gap: `useSection` now wraps section state updates in
  `startTransition`, so React keeps the previous section's tree on
  screen until the next chunk resolves; `App.tsx` adds
  `preloadAllSections()` that trickles every remaining lazy chunk
  into the module cache during browser idle slices after first
  paint. Verified live through the Tauri MCP bridge: a
  `MutationObserver` watching `<main>` recorded zero empty-content
  events across four rapid sidebar clicks.
- **Projects detail no longer shakes every 3 seconds.** Two polling
  hooks (`useRunningOps`, 3 s; `usePendingJournals`, 30 s) committed
  their Tauri IPC response unconditionally — every poll, the
  freshly-deserialized object/array failed `Object.is` against the
  prior state, even when the contents were structurally identical
  (most commonly an empty list). `AppShell` re-rendered on every
  tick; the cascade reached `ProjectsSection` whose inline
  `onError={(msg) => pushToast("error", msg)}` arrows passed a fresh
  function identity to `ProjectDetail`'s panels; `ProjectEnvPanel`'s
  data-loading `useEffect` (deps include a `useCallback` over
  `onError`) re-fired every 3 s, set `loading=true`, replaced its
  171 px loaded body with a 64 px `Loading…` placeholder for ~2 ms.
  The Sessions section below was being displaced ±107.8 px every
  3 s — the "shake" reported via the Tauri MCP bridge inspector.
  Four coordinated fixes: equality-guard the two polling hooks
  (`opsEqual()` over op_id/kind/old_path/new_path; field-equality on
  the journals summary); lift the pure-delegate `onError` arrows
  into a single `useCallback`; stale-while-revalidate in both
  `ProjectEnvPanel` and `PermissionPanel` so refetches don't replace
  rendered content with `Loading…`. Verified live: zero shifts and
  zero loading flashes across a 15-second observation window.

### Changed

- **`web/post` title card breathes against its sibling embed.**
  UrlAutoEmbed mounts the YouTube / Spotify / Apple Podcasts
  wrapper outside `.proto-text` as a direct sibling of `.proto-row`,
  so the editorial block-media rhythm never applied and the iframe
  sat flush against the bare title card. Each of the three
  `.proto-row + .proto-*-embed` selectors now carries a Stride-tier
  top margin (`--sp-24`), matching the in-body stride used for
  `pre` / `.proto-code` / `.proto-mermaid` / `.proto-table-wrap` /
  `figure`.
- **Inline `<code>` is tinted and scales with headings.** Replaces
  the prior "stays unstyled" policy that font-mono-only.css made
  ambiguous — a font-family swap can't distinguish code from prose
  anymore. Inline `<code>` now gets a barely-tinted pill via
  `--code-bg` / `--code-text` plus em-relative padding
  (`--code-inline-py` / `--code-inline-px`) so it rides the
  surrounding text scale, readable inside h1–h6 as well as in body
  copy. Scoped via `:not(pre) > code` so fenced blocks inside
  `.proto-code` are unaffected.

## 0.1.36 — beta (unreleased)

Post-0.1.35 audit-fix round plus the user/project scope toggle for the
MCP installer pane. Surfaced by driving the running dev app through
the Tauri MCP bridge (`webview_screenshot` + `webview_execute_js`) and
by a Codex mini-audit over the whole branch.

### Added

- **MCP installer scope toggle.** Settings → MCP now offers explicit
  `User` vs `Project` scope. User scope writes
  `~/.claude/claudepot-mcp-instructions.md` and names the three
  home-scope agent configs (`~/.claude/CLAUDE.md`,
  `~/.codex/AGENTS.md`, `~/.gemini/GEMINI.md`) as paste targets.
  Project scope writes `<project>/.claude/claudepot-mcp-instructions.md`
  and emits a relative `@.claude/...` include line for the project's
  `AGENTS.md` — aligned with the `/init-workspace` convention
  (AGENTS.md is the canonical source; CLAUDE.md and GEMINI.md just
  `@AGENTS.md`).
- **Validation errors on MCP filter args.** `claudepot_list_memories`
  and `claudepot_list_decisions` now reject unknown `scope` / `kind` /
  `status` values with `invalid_scope` / `invalid_kind` /
  `invalid_status` error envelopes instead of silently coercing them
  to "no filter".

### Changed

- **`exchanges.id` namespaced by source_kind.** The PK format moved
  from `<session_id>:<turn_index>` to
  `<source_kind>:<session_id>:<turn_index>` so an unlikely Claude /
  Codex UUID collision can't reject the second transcript.
  SCHEMA_VERSION bumped 4 → 5; the existing v4 cache-invalidation path
  cascade-clears `exchanges` via FK on first open.
- **Canonical MCP snippet centralized.** The agent-instruction snippet
  now has a single source of truth at
  `claudepot_core::mcp_snippet`. CLI and Tauri pane both re-export /
  alias it. Audit caught the CLI and GUI bodies had already drifted.
- **Tauri Shared Memory commands share one `SessionIndex`.** Opened
  once at startup and `.manage()`d via Tauri state instead of
  per-command `SessionIndex::open()`. Mirrors the existing
  `AccountStore` / cards-index pattern.
- **Activities heading.** Now reads "Activities" (was a leftover
  "Events" from before the registry rename).

### Fixed

- **`database is locked` in the Memories tab.** Each
  `shared_memory_*` Tauri command was opening its own `SessionIndex`
  per call; under live contention with the running app's other
  handles, the first write inside `apply_schema` (a `BEGIN IMMEDIATE`
  `_touch` round) raced and lost. The shared-state refactor above
  eliminates the contention.
- **MCP health probe stuck at `tool_visible · failed`.** Probe was
  spawning `current_exe()` — the GUI binary, which has no
  `mcp memory-server` subcommand. Resolver now finds the CLI binary
  next to the GUI (`claudepot` in dev, `claudepot-cli` in release,
  target-triple fallbacks for pre-bundle externalBin layouts). On
  failure we drain 1 KiB of stderr so the UI shows *why* instead of
  a bare "failed" badge.
- **MCP health 8s deadline ineffective.** Blocking
  `BufReader::read_line` only honored the deadline between reads, so
  a child that stayed alive but emitted no newline hung the request
  thread. Rewritten with `tokio::process::Command` +
  `tokio::time::timeout` around the read loop and
  `kill_on_drop(true)` on the child.
- **Runtime `waiting_for` change never emitted.** `set_pid_status`
  was updating the state machine *before* the diff check, so the
  comparison against `snapshot_waiting_for(s)` always read equal.
  `SessionState` now carries a `last_waiting_for` field and compares
  against the previously-emitted value.
- **`environment_trail` collected and discarded.** The Codex parser
  populated this field but the indexer never read it. Removed; the
  synthetic-seed branch still extends `line_end` so physical line
  ranges stay accurate.
- **CSS token-literal artifacts.** Five files contained invalid CSS
  strings like `"tokens.sp.px solid var(--line)"` and
  `"tokens.sidebar.width"` that the browser silently dropped —
  borders, paddings, and grid columns never rendered. Rewritten to
  `var(--sp-px)` / `var(--sp-N)` / `var(--sidebar-width)`. fontSize
  fallbacks of the form `var(--fs-2xs, tokens.sp[12])` simplified to
  `var(--fs-2xs)`.

## 0.1.35 — beta (released 2026-05-15)

Cross-harness shared memory: a v4 schema migration over `sessions.db`,
a Codex rollout parser, a backfill indexer, FTS5 search, durable
memories/decisions/evidence, and an MCP server (`claudepot mcp
memory-server`) that exposes search/read/remember/log-decision/
archive-decision/submit-evidence/list-memories/list-decisions to
Claude Code and Codex over stdio.

### Added

- **Cross-harness shared memory.** Claude Code and Codex transcripts
  index into a unified `exchanges` table with FTS5 search. New CLI:
  `claudepot codex index` walks `$CODEX_HOME/sessions/` and populates
  the index incrementally.
- **MCP memory server** (`claudepot mcp memory-server`) — 8 tools on
  rmcp 1.7 stdio transport. Categorized error envelopes
  (`invalid_scope`, `locator_not_indexed`, `decision_not_found`, ...)
  and a stricter emission redaction policy than the at-rest store.
- **Durable memories, decisions, and evidence.** New tables hold
  user/agent-authored facts (`memories`), `decisions` with atomic
  supersession + explicit archive, and audit-fix `evidence_records`,
  cross-linked via `memory_links` with DB-level CHECK constraints
  enforcing exactly-one parent + target.

### Changed

- **`sessions.db` migrated v3 → v4.** Additive: new `exchanges`,
  `tool_calls`, FTS5 virtual table + maintenance triggers, plus the
  four durable-CRUD tables. Migration is fully atomic (DDL +
  cache-invalidate + version bump in a single `BEGIN IMMEDIATE`...
  `COMMIT`) with strict post-write validation across the named
  tables, the three FTS triggers, and FTS internals. Crash-safe
  via `_pending_rescan` and forward-downgrade-guarded via
  `_min_compatible_version`.
- **`PRAGMA foreign_keys=ON`** now set on every `sessions.db`
  connection. Pre-v4 FK declarations finally enforce.
- **Tracing pinned to stderr** for the `mcp memory-server`
  subcommand regardless of `--verbose`. MCP framing on stdout is
  no longer at risk from a stray `tracing::warn!`.
- **DB created atomically with mode 0600 on Unix** via
  `OpenOptions::mode(0o600).create_new(...)` before rusqlite opens
  the file. Closes the umask race window.

### Fixed

- Codex parser bounded to 1 MiB per line; oversized lines drained
  via `BufRead::fill_buf` + `consume` so memory stays O(1)
  regardless of input size.
- Indexer skips symlinks to prevent arbitrary-file disclosure via a
  poisoned `$CODEX_HOME/sessions/` entry pointing outside the
  sessions root.
- FTS5 user input phrase-escaped and ASCII control chars stripped
  before reaching `MATCH`; LIKE filters wildcards escaped with an
  explicit `ESCAPE` clause.
- `read_locator`'s `exchange_id` lookup constrained by `file_path` —
  a mismatched id returns `NotIndexed` instead of silently widening
  the read to a file-level scan.
- MCP `max_bytes` clamped server-side to 1 MiB.
- Multi-byte-safe truncation in `read_lines`: walks down to the
  nearest UTF-8 char boundary, never produces an invalid string.
- Indexer's `truncate_chars` is grapheme-aware (ZWJ emoji families,
  regional-indicator flags, skin-tone modifiers no longer split).
- Slug uniqueness for non-UTF-8 paths via SHA-256-prefix hash
  fallback when `to_string_lossy()` produced `U+FFFD` characters.

## 0.1.34 — beta (2026-05-15)

Two new control surfaces layered over the existing project model:
time-boxed permission grants and a fully-local `.env` secret vault.

### Added

- **Time-boxed permission grants (ProjectDetail → Permissions).**
  Putting a Claude Code project into `bypassPermissions` is easy to
  switch on and easy to forget — the elevated state is invisible and
  relies on memory to undo. Claudepot now grants `bypassPermissions`
  for a fixed window (30 minutes, 2 hours, 8 hours) and auto-reverts
  it when the timer ends, restoring the exact mode that was there
  before. A live countdown keeps the elevated state visible while it
  lasts, and Extend / Revert-now controls let you adjust it. A
  project elevated by hand-editing its own settings is surfaced as
  elevated but left untouched — Claudepot only manages grants it
  created. Grants are written to `.claude/settings.local.json` and
  reverted on the existing five-minute background tick; if the
  settings layer was changed by hand since the grant, the revert
  steps aside rather than clobbering that change.
- **Local `.env` secret vault + per-project `.env` editing (Keys →
  Secret vault, ProjectDetail → Environment files).** A fully-local
  named-secret store — no cloud, no sync, stored at rest on this
  machine only — plus a movement layer for a project's `.env*`
  files. Copy a value out (written to the OS clipboard from Rust,
  never rendered into the UI, self-clearing after 30 seconds),
  inject a vault secret into a project's `.env`, or comment /
  uncomment / delete individual keys. Edits are line-oriented and
  format-preserving: only the target key's line changes, so blank
  lines, ordering, and unrelated comments survive byte-for-byte. It
  is deliberately a movement layer, not a text editor — arbitrary
  file editing stays with your editor of choice.

## 0.1.33 — beta (2026-05-15)

A new Third-party capability — shell-PATH setup for CLI wrappers —
plus the route base-URL and edit-modal fixes that surfaced while
building it.

### Added

- **Shell-PATH integration for third-party CLI wrappers.** "Use in
  CLI" writes a wrapper to `~/.claudepot/bin/`, but nothing put that
  directory on the shell's PATH — so the command the wrapper installs
  couldn't actually be run, and the Third-party pane claimed
  "✓ on PATH" from a flag that only meant "file written," never a
  real check. Claudepot now probes the login shell to verify whether
  `~/.claudepot/bin` is genuinely on PATH (timeout-guarded, so a slow
  shell rc can't hang the app), shows a verified three-state wrapper
  indicator, and offers an "Add to PATH" banner that appends the
  `export` line to `.zshrc` / `.bash_profile` idempotently — scoped
  to real PATH-assignment lines, with shell-unsafe paths refused
  rather than written into a file the shell sources.

### Fixed

- **Gateway routes whose base URL ends in `/v1` no longer fail with
  "model not found".** Ollama and other OpenAI-compatible docs tell
  you to point at the `…/v1` URL, so that's what people pasted into
  the route's base-URL field — but Claude Code's SDK appends
  `/v1/messages` itself, producing `…/v1/v1/messages`: a 404 the CLI
  surfaced as a misleading "the selected model may not exist." The
  base URL is now normalized on save — a trailing `/v1` is stripped,
  and query strings / fragments are rejected outright.
- **The route and automation edit modals no longer reset while
  you're typing.** Their fetch effects depended on callbacks the
  parent section recreated on every render — and those sections
  re-render on a timer (the live Activity strip) — so every few
  seconds the modal refetched and remounted its form, throwing away
  in-progress edits and scroll position. The fetch now keys only on
  the route/automation being edited, not on render-churn.

## 0.1.32 — beta (2026-05-14)

Two follow-ups to 0.1.31's chrome fix:

### Fixed

- **Window-chrome row exactly centered.** 0.1.31's runtime intro
  pinned the breadcrumb to wherever AppKit placed the lights, but
  the lights themselves were at y=20 in a 38-px chrome (1 px below
  the geometric center y=19) because `trafficLightPosition.y` in
  `tauri.conf.json` was 22 — Tauri 2.11 maps that to a close-button
  center of 20 (config-y minus 2). Drop the config to `y: 21` so
  AppKit places the close button at y=19 exactly. The runtime intro
  picks up the new center, the chrome's `translateY(calc(...))`
  collapses to 0 px, and both the lights and the breadcrumb land on
  the chrome midline. Verified live: close-button center y=19,
  breadcrumb center y=19, inner transform `matrix(1, 0, 0, 1, 0, 0)`.
- **Notification bell popover renders opaquely over main content
  again** (regression from 0.1.31). The chrome's inner wrapper
  (added in 0.1.31 for the traffic-light translateY) carries a
  `transform` value, which creates a new CSS stacking context —
  even an identity matrix does. The popover's `z-index: 200` was
  scoped to the chrome's local context instead of the document, so
  main-content painting ended up on top of the popover and you'd
  see Settings / Health pane content showing through. Render the
  popover via `createPortal` to `document.body` so it escapes the
  chrome's stacking context entirely. Bell button stays in-place;
  only the popover panel portals out. Verified live:
  `elementsFromPoint` at the popover-overlap point now returns the
  popover dialog as topmost (was returning the cc_doctor `<p>`).

## 0.1.31 — beta (2026-05-14)

Bundles the unreleased 0.1.30 payload (UI fixes + release-workflow
fixes) with a follow-up batch: principled fix for the long-standing
chrome / traffic-light alignment drift, full stderr surfaced when
`claude auth login` fails (issue #16), and a rustfmt cleanup that
returns CI to green.

### Fixed

- **Settings → Cleanup is now usable at narrow widths.** The
  `CleanupPane | TrashDrawer` row had a fixed 2:1 split with no
  reflow, so a ~1000-px window squeezed the trash drawer's header
  buttons and entry rows into a ~150-px column. Stacked the sections
  vertically with hair-line top borders (same treatment the
  artifact-lifecycle pane below already used). Every section now
  gets the full content width.
- **Error and diagnostic text is selectable.** Plugin errors, scan
  failures, preview failures, and cc-doctor entry rows could not be
  selected for copy/paste — the global `body { user-select: none }`
  rule was silently winning over inline `userSelect: "text"` in
  WKWebView (React doesn't emit the `-webkit-` prefix). Opted ARIA
  `role="alert"` / `role="status"` surfaces back into
  text-selectable globally, plus `<code>` / `<pre>`; this catches
  every toast and banner across the app. Three bare error divs
  (ConfigSection's scan and preview failures, AutomationsSection's
  load error) got the proper `role="alert"` added.
- **Window-chrome breadcrumb now pins to the OS-reported
  traffic-light centerline** — not the chrome's geometric center.
  AppKit doesn't place the standard window buttons at chrome
  midpoint; the actual visible center depends on macOS version,
  button height, the configured `trafficLightPosition.y`, and
  AppKit's autoresizing during first paint. Hardcoded `translateY`
  compensation drifts on every Tauri / macOS bump (this repo's
  `trafficLightPosition.y` went 14 → 21 → 22 across three months
  chasing the moving target, then drifted again under Tauri 2.11).
  New `src-tauri/src/traffic_light.rs` reads the live
  `NSWindow.standardWindowButton(.closeButton).frame` via objc2-app-kit
  and emits a `traffic-light-metrics` event with the center y +
  cluster right edge. The frontend writes those as CSS custom
  properties (`--traffic-light-center-y`, `--traffic-light-right`,
  and overrides the static `--chrome-inset-left`); `WindowChrome`'s
  inner wrapper carries a single `transform: translateY(calc(...))`
  pinned to the OS line. Re-emits on Resize / Move / Focus /
  ScaleFactorChanged so multi-monitor + DPR changes track. The
  `calc()` collapses to 0 on non-macOS / pre-mount, falling back to
  flex centering.
- **`claude auth login` failures now surface the actual stderr** in
  the dialog (issue #16). The error variant
  (`OnboardError::AuthLoginFailed`) carries the last 12 redacted
  stderr lines from the subprocess. The dialog used to show only
  `exited with code 1`; users can now see the actual reason —
  network error, OAuth state mismatch, keychain perm denied, etc.
  Combined with the `role="alert"` selectability fix above, the
  text is copy-pasteable so users can share it in a bug report.
  The timeout sentinel (`-2`) keeps its existing recovery message;
  blank/whitespace tails suppress the label.

### Infrastructure

- **Release workflow now ships correctly via `workflow_dispatch`.**
  `softprops/action-gh-release` was defaulting `tag_name` to
  `github.ref`, which is `refs/heads/main` (a branch ref, not a
  tag) when invoked manually. The action hit HTTP 400 and the job
  failed with "Too many retries." Pass
  `${{ github.event.inputs.tag || github.ref_name }}` explicitly so
  manual re-dispatches land at the right tag. Tag-triggered runs
  (the normal path) are unaffected.
- **`-Force` on PowerShell `Compress-Archive` in the Windows CLI
  packaging step.** A re-dispatch picks up the previous run's
  `target/release` from the cargo cache, including
  `claudepot-x86_64-windows.zip` — `Compress-Archive` refuses to
  overwrite by default. Caught when re-dispatching 0.1.29 to test
  the `tag_name` fix above.
- **rustfmt drift absorbed.** `cargo fmt --all` against modern
  rustfmt (Rust 1.95+) reformatted 27 files (mostly width-wrapping
  changes accumulated since 1.92). The `Format / Clippy (Linux)`
  job has been red on every release commit since v0.1.27 because of
  this; main is back to green from this commit forward. Behaviour-
  only — no logic edits.

### Notes

`v0.1.30` was tagged but never released. The tag was deleted from
origin so the timeline jumps cleanly v0.1.29 → v0.1.31. Everything
that would have been in 0.1.30 ships here.

## 0.1.29 — beta (2026-05-13)

Mini audit-fix sweep across both the desktop app and `claudepot.com`.
26 medium-severity findings; 25 fixed in this release, 1 deferred
(`migrate_export.sign_password` zeroize gap — needs a core API change).

### Fixed

- **Secret zeroization.** Both CC and private-storage keychain writers
  now wrap the hex-encoded credential and the `security -i` command
  string in `SecretString`, so the bytes are scrubbed on every exit
  path. Route validation paths (Gateway / Bedrock / Foundry) likewise
  zeroize their secret-bearing inputs on every early-return error
  branch (empty region, missing auth, invalid base URLs); the secret
  no longer appears in the user-facing error message and no partial
  route lands in the store.
- **Template path scoping.** `within_home` validation now walks up to
  the nearest existing ancestor, canonicalizes through the
  Windows-aware helper, and rejoins the tail — closing a
  symlink-escape gap on missing leaf paths. `canonicalize_glob_prefix`
  preserves UNC prefixes and routes through `canonicalize_simplified`
  instead of bare `std::fs::canonicalize`.
- **CLI `--json` invariants.** `memory view`, `desktop launch`,
  `desktop quit`, and `session export` previously printed
  human-readable lines regardless of `--json`. They now emit
  structured JSON when the flag is set (one shape per destination for
  `session export`: `file`, `clipboard`, `gist`).
- **xtask error propagation.** `verify-cc-parity` silently dropped
  `read_dir` entry errors via `.flatten()` and could exit 0 when
  `--only` matched nothing. Errors now propagate, zero-match `--only`
  fails loudly, and `policy` entries are validated for shape so broken
  fixtures don't pass silently.
- **`UsageView` / `UpdatesPanel` listen rejections.** Tauri
  `listen()` promises now `.catch()` so a non-Tauri / failed-listen
  environment doesn't surface as an unhandled promise rejection;
  `UpdatesPanel` cleanup optional-chains `fn?.()` for the same
  reason.
- **`CopyButton` clipboard handling.** Guards `navigator.clipboard?
  .writeText` typeof so the fallback path runs on platforms missing
  the API, and clears the reset timer on unmount.
- **Web — duplicate per-request DB reads.** `/post/[id]` and
  `/links/c/[slug]` each loaded the same row twice (once in
  `generateMetadata`, once in the page body). Wrapped the loaders in
  React `cache()` and parallelized independent fetches.
- **Web — visibility filter gap.** `getSavedForUser` and
  `getUpvotedByUser` filtered `deleted_at` but not `unlisted_at`, so
  staff-unlisted submissions surfaced in those personal feeds. Added
  the missing filter.
- **Web — OG image score guard.** The OG image route guarded score
  with a falsy check on `post?.upvotes`, so a post with 0 upvotes and
  a negative net score rendered `0 points`. Now guards on post
  existence only.
- **Web — MCP auth fail-closed.** `verifyClaudepotToken` wraps the
  lookup chain in try/catch so a transient DB failure returns
  `undefined` and the auth adapter fails closed instead of throwing.

### Changed

- **Notifications drain order when polling.**
  `listNotificationsForUser` returns newest-first for one-shot fetches
  and oldest-first when polling with `since`, so callers can advance
  the cursor to the newest returned item and deterministically drain
  an overflowed window. The MCP tool description is updated; the REST
  shape (`/api/v1/notifications`) is unchanged but bots that
  hard-coded "newest-first while polling" will see the new order.
- **Usage scans off the IPC worker.** `local_usage_aggregate` and
  `top_costly_prompts` now run their filesystem walks + session-index
  refresh + pricing-table work in
  `tauri::async_runtime::spawn_blocking` so the Tokio IPC worker
  isn't pinned during a large scan.
- **Focus rings on custom buttons.** Several shell and Settings
  buttons (palette pill, health pill, service-status dot, sidebar
  target switcher rows, Settings nav / theme / threshold-chip /
  toggle) now carry the `pm-focus` class so the documented
  `:focus-visible` ring renders for keyboard users.

### Infrastructure

- **CI clippy: `--all-targets`.** The Linux clippy gate now includes
  `--all-targets`, catching test-code lints that drifted silently
  between Rust 1.92 and 1.95 (`useless_format`,
  `cloned_ref_to_slice_refs`, `iter_nth_zero`, …).

## 0.1.28 — beta (2026-05-13)

### Added

- **Category/Priority/Surface schema for notifications.** Every event
  routed through `emit()` now declares a Category (job, threshold,
  status, rotation, …), a Priority (P0–P3), and a Surface (toast,
  banner, bell, os). The schema is enforced at the type system, so a
  new emit site must classify itself — opaque toast strings are no
  longer accepted.
- **Per-category Settings → Notifications pane.** Each category can
  be muted independently with a per-surface toggle (e.g. silence the
  OS banner for rotation events but keep the bell entry). Persisted
  under `CategoryPrefs` and migrated forward on first launch.
- **Bell popover hides P3 ambient by default.** The notification
  history dropdown filters Priority 3 (ambient/informational) entries
  out of the default view, with a "Show all" toggle. P0–P2 always
  visible; the rule is intentional — the bell is the "what should I
  know about right now?" surface, not the firehose.

### Changed

- **Single `emit()` facade replaces scattered notification sites.**
  `useUsageThresholdNotifications`, `useActivityNotifications`,
  `useOpDoneNotifications`, `useRotationEvents`,
  `useStatusBannerEmits`, and `useBackgroundChangeEmits` all funnel
  through one dispatch path. Toast wrapping (`pushToast`), bell-log
  append, and OS notification dispatch are decided in one place from
  the event's Category/Priority/Surface — not by each hook.
- **`pushToast` is wrapped, not bypassed.** The `_suppressLog` shim
  that let hooks opt out of bell logging is gone; emit() decides
  whether a toast also lands in the bell based on category rules.
  New surfaces plug into the same dispatch.

### Fixed

- Four rounds of Codex audit closed across the refactor: 1 Critical,
  6 High, 6 Medium, 4 Low findings — plus follow-up rounds for the
  remaining partials and a checked-in cross-language fixture for the
  final Low item.

## 0.1.27 — beta (2026-05-13)

### Added

- **`DoctorSeverity::Unknown`** — new "we couldn't measure" state in
  the Health pane, distinct from healthy/warning/error. The
  window-chrome pill goes grey (never green or yellow) when the
  `claude doctor` parser can't read CC's TUI output, so a parser
  hiccup no longer masquerades as a real health alert. The pane
  separately shows a parse-failure banner with a link to the
  forensic log.
- **Direct probes back up the pty scrape.** A new
  `cc_doctor::probes::probe_version()` runs `claude --version`
  (~50 ms, no pty) and resolves the install path via
  `fs::canonicalize`, so the Health pane still shows
  `claude X.Y.Z · native` and the install path even when the TUI
  scraper trips on an Ink redraw. Brew-cask `claude-code`,
  npm-global, and Volta layouts are all covered.

### Changed

- **Health snapshot invalidates on CC version drift.** Before
  honoring its 60 s cache, the snapshot command now compares the
  cached `cc_version` to a fresh `claude --version` probe. If CC
  self-updated mid-cache, the stale snapshot is discarded and a
  fresh scrape runs — eliminating the "claude version unknown" +
  "PATH not set" panel that lingered for up to a minute after a
  self-update.
- **`aggregate_severity` no longer forces a Warning floor on
  partial parses.** A scrape that returns sections + a degraded
  parse status reports the worst observed section severity, not
  Warning by default. Empty sections + failed parse → Unknown
  (grey dot, not yellow). The metrology signal (parse_status)
  drives the banner; the health signal (severity) drives the dot.
  Two distinct surfaces, two distinct meanings.
- Health-pill tooltip and Health-pane header surface three distinct
  states cleanly: fully measured (colored dot + sections), identity-
  only (version header + grey dot + parse-failure banner), and
  fully unmeasured ("Couldn't read claude doctor — refresh to
  retry" copy in grey).

### Fixed

- Health-pill tooltip no longer claims "No issues reported." when
  the scrape parser failed — it now says "(health check incomplete
  — refresh to retry)" so a parser failure can't read as a clean
  bill of health.
- `classify_install_path` normalizes `\` → `/` before matching, so
  the Homebrew + `node_modules` rules fire on Windows-canonicalized
  paths too (per `.claude/rules/paths.md`).
- Two pre-existing Rust 1.92 clippy lints
  (`collapsible_str_replace` in `dev_alert.rs`,
  `duplicated_attributes` in `dock_icon.rs`) — small one-line
  fixes that were blocking the CI Linux clippy gate.

## 0.1.25 — beta (2026-05-12)

### Added

- **Copy buttons on every session-detail bubble + tool viewer.** Each
  user/assistant/thinking/tool-call/tool-result/summary bubble grows
  a small copy icon in its header. Tool viewers carry their own copy
  buttons too: Bash copies `$ cmd\nstdout\n[stderr]…`, Edit copies a
  unified-diff-shaped text, Read copies the file content with CC's
  line-number prefix stripped, Write copies what was written, the
  generic fallback copies `[input]…\n\n[result]…`. Payloads are
  always run through `redactSecrets` — the clipboard never carries a
  secret the screen wouldn't show.
- **Collapsible left sidebar (⌘\\).** A chevron at the bottom of the
  sidebar (and a `PanelLeft` toggle at the far-left of the status bar)
  flips the sidebar between full width and a 52px icon-only rail.
  Persisted under `cp-sidebar-collapsed` in localStorage; ⌘\\ is the
  global keyboard shortcut. In collapsed mode swap-targets and the
  activity strip hide, primary nav becomes glyph-only with a presence
  dot in the corner for badged sections, and tooltips carry the
  hidden labels for keyboard/AT users.

## 0.1.24 — beta (2026-05-11)

### Added

- **Per-project + per-session cost in the Projects rail.** The left
  rail's project rows now carry a hypothetical Anthropic API-rate
  cost ("$12.34") computed from the same `local_usage_aggregate`
  pipeline that powers the Cost tab. Inside each project, the
  Sessions list shows a per-session cost in the meta line and a
  project total in the section heading ("Sessions · 7 · $4.21 at
  API rates"). Unpriceable sessions (model not in the price table)
  render no `$` rather than `$0.00`. The section subtitle gains an
  install-wide total ("47 projects · all healthy · $128.50 at API
  rates"). The label "at API rates" disambiguates from actual
  Max-plan billing.

## 0.1.23 — beta (2026-05-11)

### Added

- **CC self-diagnostic surface.** New `claude doctor` scrape
  pipeline surfaces CC's own self-check (installation type,
  version, plugin errors, version locks, settings validation,
  context-budget warnings, unreachable permission rules) in three
  layered surfaces without ever needing to run the CLI command:
  - **HealthPill** — 10px severity dot next to the bell in
    WindowChrome. Polls every 60s + on focus. Click → opens
    Settings → Health.
  - **Settings → Health pane** — full snapshot render with
    severity-tinted section cards, a Refresh button, and a
    parse-status banner when the parser produced incomplete
    data. Distinct from Settings → Diagnostics, which still
    covers Claudepot's own self-check (accounts / API reach /
    proxy).
  - **Tray menu Health row** — live label ("Health: 4 issues" /
    "checking…" / "ok"). Click → opens the window on the Health
    pane. Backed by a 5-min background scrape so the tray stays
    current when the window is closed.
- **Defensive trigger for the scraper.** `claude doctor` is a
  TUI; we capture it via a pty, replay through a minimal terminal
  emulator, and parse sections. CC's layout drift could break our
  parser. Any non-clean parse records the raw bytes (redacted —
  `sk-ant-*` and `cdp_pat_*` tokens are masked before persistence)
  along with diagnosed reason, CC version, and Claudepot version
  to `~/.claudepot/doctor-parse-failures.jsonl` (100-entry ring
  buffer, `0o600` perms enforced at file creation and on existing
  files). In debug builds or with `CLAUDEPOT_DEV=1`, an OS
  notification fires, de-duped per `(cc_version, reason)` per
  process. End-user release builds get the silent fallback:
  pill keeps the last-known-good snapshot visible.
- **Web: citizen-bot fleet.** Humans can now create their own
  bots from Settings → Bots. Backed by a `bot_kind` discriminator
  + per-axis gating; complements the existing op-bot and
  reader-bot fleets.
- **Web: client-side avatar cropper.** New `/settings` avatar
  panel uploads a chosen image, crops square in-browser, and
  posts the result to `avatar:set` / `avatar:clear` API. Reader
  bots get pixel-invader avatars at 32×32 / 512×512.
- **Web: Spotify + Apple Podcasts auto-embed** on post-detail
  pages. YouTube embed surface from 0.1.22 widened to a tighter
  media-embed pipeline.

### Changed

- **Web: unified button + input shape across the site.**
  Finished the radius-sm migration started in commit `303ddf8`
  — form-scoped submits, OAuth provider buttons, docs hero
  CTAs, and `.suggest-form-actions` were still on radius-full /
  radius-md. All buttons now share radius-sm + hairline accent
  border. Text inputs share a single border token, surface
  background, and paper-mono focus halo (was a solid accent
  ring on most surfaces).
- **Web: `commentCount` split into human + bot** so feed/post
  surfaces can render the right denominator per context.
  Submission `score` defaults to human-only.

### Fixed

- **Web: `/stats` page polish.** Big metric cards center their
  numbers; trailing note paragraph has breathing room above the
  preceding list (previously stacked flush).
- **Web: comment tree** plumbs avatars + AI chip through nested
  replies, hides 0-points lines.
- **Web: settings reveal-cookie flow** split into `peek` + `delete`
  so the Server Component path doesn't fail with the
  digest-4117502548 cookie-write-in-RSC incident.
- **Web: paper-mono focus halo + unified button radius** site-wide
  (from `303ddf8` — landed pre-cc-doctor in this release window).
- **Web: vote button** receives viewer's initial vote so the SSR
  render matches the post-mount state.

## 0.1.22 — beta (2026-05-08)

### Added

- **Auto-rotation of the active CLI account.** New
  Settings → Rotation pane lets you author rules that swap the
  active CLI account when an Anthropic-reported usage window
  (5h / 7d / 7d-Opus / 7d-Sonnet) crosses a threshold you set.
  Triggers fire from the existing `/api/oauth/usage` poll (no new
  network cost). Two modes: **Confirm** (default) raises a toast
  with a Switch action; **Auto** swaps immediately. Selectors:
  `least-used` (lowest utilization in candidate list),
  `round-robin`, `explicit`. Guards: `min_interval_secs` (default
  60s), `max_swaps_per_window` (default 3 per cycle, evaluated
  against the actual `resets_at` cycle marker — not a fixed
  lookback), `skip_when_cc_running` (defers cleanly without
  re-logging every tick). Every swap attempt — applied, suggested,
  skipped, failed — lands in a 500-entry ring-buffer audit log
  (`~/.claudepot/rotation-audit.json`) visible in the same pane
  with an outcome filter. The rule file
  (`~/.claudepot/rotation-rules.json`) is hand-edit-friendly JSON
  with stable schema-version field. Pure-Rust evaluator in
  `claudepot-core::rotation::eval` is fully unit-tested
  (44 tests, including cycle-boundary regressions). See
  Settings → Rotation to add a rule; the empty state explains the
  policy framing.
- **Confirm-mode toast hydration on mount.** Rotation suggestions
  raised while the renderer was disconnected (between reloads,
  before mount) are re-surfaced via `rotationPendingList()` on the
  same path live events take, so a Switch action is never
  silently lost.
- **Web: YouTube auto-embed in submission bodies.** Bare YouTube
  URLs in a post's markdown body render as inline players. Pipeline
  hardened against malformed URLs and parameter injection
  (`web/src/lib/youtube-embed.ts` + tests). Production-safe.
- **Web: `/app/features` screenshots.** Six previously-404'ing
  screenshot assets are now tracked in
  `web/public/screenshots/`.

### Changed

- **Web: re-enabled Vercel `<Analytics />` + added
  `<SpeedInsights />`** in the root layout, now that the Vercel
  Web Analytics dashboard is on for the project.
- **Web: office layout refactor.** Newsroom + ada moderator card
  introduced; AI chip on team cards replaced with the bare bot
  glyph; Admin link moved into the `@user` dropdown.

### Fixed

- **Rotation cycle-boundary accounting.** The
  `max_swaps_per_window` guard now compares each prior swap's
  `resets_at` against the current cycle's marker. A swap from the
  previous cycle that still falls inside the raw 5h/7d lookback no
  longer counts toward the new cycle's cap.
- **Rotation rules I/O distinction.** A real read failure
  (permission denied, transient FS error) on
  `~/.claudepot/rotation-rules.json` now propagates instead of
  silently looking like "no rules" — protecting against a
  follow-up save that would clobber the user's real config. Missing
  + corrupt files still recover to empty as before.
- **Rotation pending-swap lifecycle.** TTL evicts before every
  accessor (`pending_list`, `take_pending`, `peek_pending`); the
  apply path peeks then removes only on success, so a transient
  swap failure leaves the entry available for retry; dedupe key
  now includes `from_uuid` so a fresh suggestion to the same
  target after the active account changed is no longer suppressed.
- **Rotation dry-run accuracy.** The "Test now" button in the rule
  form now reads the audit log when evaluating, so guards
  (min-interval, max-swaps) participate. Previously it could
  promise "would fire" when the next real tick would be blocked.

## 0.1.21 — beta (2026-05-08)

### Added

- **Per-account usage snapshot file for non-GUI consumers.** The
  GUI now writes `~/.claudepot/usage-snapshot.json` (mode 0600)
  every 5 minutes, carrying per-account 5h / 7d / 7d-Opus /
  7d-Sonnet utilization plus reset timestamps and an
  `ok | no_credentials | expired | rate_limited | error` status
  per entry. Cron jobs, Claude Code Bash-tool subprocesses, and
  third-party automations can now consult per-account usage —
  e.g. pick the least-loaded account at startup, or refuse to run
  when every account is saturated — without going through the
  keychain. The previous wall (CC-spawned subprocesses and cron
  jobs hitting `errSecAuthFailed` from `/usr/bin/security`) is
  bypassed entirely: consumers read JSON, no auth, no IPC, no
  daemon. `written_at` is the consumer's tripwire — older than
  5 minutes means Claudepot isn't running, treat as historical.
  Owned by `claudepot-core::services::usage_snapshot` (pure,
  Tauri-free) and the new periodic task in
  `src-tauri/src/usage_snapshot.rs`.

- **`claudepot account list` now shows the 7-day window.** The
  CLI's table grew a `7d` column between `5h` and `CLI`,
  rendering the same `seven_day` utilization the GUI's usage
  modal already showed. Useful for quickly spotting which
  account is approaching the 7-day cap when picking where to
  start a session. JSON output gained a parallel
  `seven_day_pct` field next to the existing `five_hour_pct` —
  additive, non-breaking for existing consumers.

## 0.1.20 — beta (2026-05-07)

### Added

- **Global → Tips: a searchable ledger of CC's spinner tips.** New
  sub-tab in Global that extracts the tip registry directly from
  the user's installed CC binary (Bun-compiled binaries embed the
  JS source as ASCII; no network call required), joins against
  `~/.claude.json::tipsHistory` to mark seen / never-seen rows,
  and time-resolves "last seen" via a Claudepot-side snapshot log
  that converts CC's count-based ledger into wall-clock deltas.
  Surfaces categories (12 buckets), plain-English trigger
  summaries, A/B variant disclosure with the GrowthBook flag name,
  platform-conditional branches (Apple Terminal vs Shift+Enter),
  shortcut interpolation against the user's keybindings, and the
  raw `isRelevant` source under a "Show advanced trigger logic"
  disclosure. 47 of 53 known tips extracted from CC 2.1.132 on
  first launch; format drift is caught by an integration test
  against the real binary. Lives at `claudepot-core::cc_tips` —
  pure Rust, no JS parser dependency.

### Changed

- **Global tab order**: Config → Memory → Tips → Updates. Tips
  ships before Updates so the page where the user looks for "what
  did I miss?" sits above the maintenance surface.

### Fixed

- _…_

## 0.1.19 — beta (2026-05-07)

### Fixed

- **Dock icon now matches dev-mode crispness in prod.** v0.1.17–18
  shipped pixel-perfect `.icns` layers, but the Dock still rendered
  soft because macOS displays Dock icons at 48pt = 96 raster pixels
  on Retina by default, and `.icns` doesn't have a 96-pixel layer
  slot. macOS picks the 128 layer and downscales 128→96 with
  bilinear filtering, softening pixel-art edges. Routed the icon
  through Cocoa's NSImage pipeline at runtime via
  `NSApplication.setApplicationIconImage` with our 512×512 source —
  Cocoa picks Lanczos for any Dock size's downsample, preserving
  crispness. Same trick Tauri's runtime uses in dev mode.

### Changed

- _…_

### Added

- _…_

## 0.1.18 — beta (2026-05-07)

### Fixed

- **Pixel-art house now fills ~80% of the squircle.** v0.1.17's
  16-px grid kept rendering crisp, but the house was visibly too
  small inside the orange surround. Bumped the cell size to 24 px
  while keeping the same 14×12-cell design — the icon now matches
  the proportions of standard pixel-art app icons (e.g. Space
  Invaders). Dock-relevant sizes (64/128/256/512/1024) still
  render pixel-perfect; only the micro-icon layers (16/24/32/48
  raster) pick up sub-pixel antialiasing, which is invisible at
  those sizes.

### Changed

- _…_

### Added

- _…_

## 0.1.17 — beta (2026-05-07)

### Fixed

- **Dock icon now renders pixel-perfect at every standard size.**
  The root cause of the v0.1.13–0.1.16 blur was the SVG's 22-px
  pixel-art grid, which doesn't divide the Apple/Microsoft icon-
  size ladder (16/32/64/128/256/512/1024) evenly — at 128 raster,
  each "art pixel" covered 5.5 raster pixels and rsvg's default
  antialiasing softened the edges. Redesigned the SVG on a 16-px
  grid (which divides every standard size cleanly). The squircle
  is on the same grid (48,48 / 416×416) so its edges are crisp too.
- **Reverted the v0.1.16 objc2 runtime override.** That patch
  loaded the 32×32 PNG and let Cocoa upscale to 128, producing
  visibly blocky output — strictly worse than the now-clean .icns
  rendering. With the 16-grid SVG, no runtime trick is needed.

### Changed

- _…_

### Added

- _…_

## 0.1.16 — beta (2026-05-07)

### Fixed

- **macOS Dock icon now matches dev-mode crispness in prod.**
  v0.1.15 shipped pixel-perfect `.icns` layers, but the actual
  blur was in macOS's render pipeline, not the file content:
  Tauri's runtime calls `NSApplication.setApplicationIconImage`
  only in dev mode, leaving prod to render via the legacy
  IconServices `.icns` path which softens pixel-art edges at
  default Dock sizes. Replicated dev's `setApplicationIconImage`
  call in our own `setup()` so prod uses the same Cocoa NSImage
  pipeline.

### Changed

- **Repo cleanup: removed 47 unused icon files.** `Square*Logo.png`
  (MSIX/UWP), `iOS/`, `Android/`, and the 1× tray-icon variants
  that were never `include_bytes!`-d. Tauri's `pnpm tauri icon`
  command produces them by default, but we don't ship for those
  targets — they were dead bytes. Added `scripts/regen-icons.sh`
  that produces only the assets we actually consume, using
  `rsvg-convert` + `iconutil` directly to avoid the bilinear blur
  that `pnpm tauri icon` introduced in earlier releases.

### Added

- _…_

## 0.1.15 — beta (2026-05-07)

### Fixed

- **macOS Dock icon now renders crisply at every size.** The `.icns`
  layers were bilinearly resampled by tauri's icon generator,
  visibly softening the 128×128 and 256×256 layers — exactly the
  ones macOS picks for default Dock display on Retina. Regenerated
  every layer fresh from `icon.svg` via `rsvg-convert` plus
  `iconutil`; pixel-identical to a clean SVG render now.
- After upgrading from a previous beta, run `killall Dock; killall
  Finder` once if the Dock still shows the cached softer icon —
  macOS's icon-services cache holds onto the old version until
  explicitly flushed.

### Added

- _…_

### Changed

- _…_

## 0.1.14 — beta (2026-05-07)

### Fixed

- **Linux app icon now installs at every standard hicolor size.**
  Previous `.deb` / `.rpm` packages shipped only 32×32 and 128×128;
  the freedesktop spec's required 48×48, GNOME's 256×256 app-grid
  default, and 64×64 / 512×512 launcher sizes were missing or filed
  under a non-standard `hicolor/256x256@2/` directory that no Linux
  desktop environment looks at. Packages now install the full
  32 / 48 / 64 / 128 / 256 / 512 ladder.

- **macOS Dock icon no longer renders larger than well-behaved
  apps.** The orange squircle was drawn full-bleed (0,0 / 512×512);
  Apple's Big Sur+ HIG specifies a ~10% transparent margin per
  side. Inset to (51, 51) / 410×410 so Claudepot now sits at the
  same visual weight as system apps.

### Added

- _…_

### Changed

- _…_

## 0.1.13 — beta (2026-05-07)

### Added

- **Network detection panel on first run.** When
  `api.anthropic.com` isn't reachable from your network — DNS
  poisoning, TCP block, TLS interception, timeout, 5xx, or an
  unclassified failure — Claudepot now surfaces a remediation
  panel above the active section. Diagnosis-specific copy plus
  four buttons: *Use a third-party LLM* (deep-links into
  Third-parties → Add Route, with China-reachable presets
  highlighted), *Configure proxy* (Settings → Network),
  *Network help* (opens claudepot.com/help/network), and *Retry*.
  Dismissible per session; the rest of the app stays usable
  (Sessions, Memory, Cleanup don't need network).

- **Quick-start gateway presets in Add Route.** Six curated
  OpenAI-compatible providers — DeepSeek, Kimi (Moonshot), Qwen
  (DashScope), GLM (Zhipu), OpenRouter, Ollama-local — each
  pre-fills base URL + a sensible default model. The
  China-reachable subset is emphasized when arriving from the
  network panel.

- **claudepot.com/help/network help page.** Endpoints Claudepot
  needs, how to diagnose unreachability with `curl`, how to
  configure a proxy, npm-mirror install for the Claude CLI, and
  third-party-LLM routing. Explicit boundary on what Claudepot
  doesn't ship (no VPN setup).

### Changed

- _…_

### Fixed

- _…_

## 0.1.12 — beta (2026-05-07)

### Fixed

- **Project detail no longer flashes a "consider cleaning" hint
  that goes nowhere.** Alive projects with leftover bytes from
  retired third-party CC plugins (e.g. `.cccmemory.db-shm`,
  `.db-wal` files whose `.db` was removed) used to show a "Go to
  Maintenance" call-to-action that landed on *"Nothing to clean."*
  Maintenance only handles orphan projects whose source folder is
  gone, and an alive project's source by definition still exists.
  The misleading hint is gone; the project header's *Finder*
  button is the right way to inspect leftover files and decide
  manually.

- **Updater progress bar no longer jumps backwards.** The download
  bar previously rendered at a fake 33% before the first chunk
  arrived, dropped to 0% once the Tauri updater's `Started` event
  reported the content length, then climbed to 100%. The "33%"
  was a static placeholder for the brief unknown-total window —
  not progress. The bar now starts at 0% and grows monotonically;
  activity during the pre-`Started` window is conveyed by the
  byte counter that's already there.

- **Updates panel speaks truthfully.** The CLI/Desktop secondary
  action button was labeled *Update anyway* even when already on
  the latest version, where pressing it ran a no-op `claude
  update` — relabeled to *Reinstall*, which is what the action
  actually does. The post-install confirmation banner said
  *"Active install: vX.Y.Z"* / *"Installed Claude.app vX.Y.Z via
  brew"* regardless of whether the version actually changed; it
  now compares before-vs-after and reports one of *reinstalled
  (vX.Y.Z)*, *updated from vA.B.C to vX.Y.Z*, or *installed at
  vX.Y.Z*. The CLI card's subtitle and comparison badge mirror
  the Desktop card's "not installed" wording when no `claude`
  binary is on PATH (was: *"installed: unknown"* with a useless
  *unknown* badge).

## 0.1.11 — beta (2026-05-06)

### Added

- **Settings → About now names the publisher.** Adds a Publisher
  row (HANDO K.K.) and a short note explaining why macOS surfaces
  this name in *System Settings → General → Login Items & Extensions
  → App Background Activity* and in occasional "ran in the
  background" notifications when Launch-at-login is enabled — it's
  the Apple Developer–registered entity behind the Developer ID
  certificate, not a third-party process.

### Changed

- **Auto-memory toggle relocated to Global → Memory.** Auto-memory
  governs Claude Code's `~/.claude/.../memory/` writes — a CC
  behavior, not a Claudepot app preference — so the toggle now lives
  next to the rest of the CC global state instead of in Settings →
  General. The new panel-card sits above the file-health grid; ⌘K
  still finds it.

## 0.1.10 — beta (2026-05-05)

### Fixed

- **Traffic-light buttons sat ~1px above the breadcrumb on macOS.**
  At `trafficLightPosition.y = 21`, the OS centered the buttons at
  visual y=18.75 logical while the home glyph next to "keys" landed
  at y=19.50 — the 0.75px gap was visible to a careful eye. Bumped
  to y=22 so the OS-drawn dots line up with the chrome's flex-
  centered content. Light- and dark-mode both verified.

## 0.1.9 — beta (2026-05-05)

Internal-only release. No user-visible changes.

### Internal

- **CI green again on `windows-latest`.** A native-install detection
  test (`detect_cli_installs_at_promotes_native_when_path_lookup_misses`)
  set up `~/.local/bin/claude.exe` via `cli_filename()` but the
  detection's `NATIVE_BIN_REL` constant is a literal `.local/bin/claude`
  with no extension — the XDG layout is Unix-only; Windows uses
  WinGet to `%LOCALAPPDATA%\Programs\Claude\`. The test is now
  `cfg(not(target_os = "windows"))`-gated. Pre-existing on main
  since v0.1.7's path-lookup-miss promotion fix; v0.1.7 + v0.1.8
  binaries shipped fine (the Release workflow runs `cargo build`,
  not `cargo test`), but CI was red.

## 0.1.8 — beta (2026-05-05)

Internal-only release. No user-visible changes.

### Internal

- **Tauri command modules reorganized.** The 30 `src-tauri/src/commands_*.rs`
  files moved into `src-tauri/src/commands/<noun>.rs`, mirroring the
  CLI's `cli/commands/<noun>.rs` layout (per `.claude/rules/commands.md`).
  `lib.rs` lost 28 top-level `mod` lines; new handlers go in the
  noun directory. No runtime behavior change.
- **Linux clippy gate restored.** Clippy 1.95 strengthened
  `needless_return`; four `cfg(not(target_os = "macos"))` branches
  in `claudepot-core` (`automations/scheduler`, `desktop_identity`,
  `routes/desktop`, `routes/helper`) now end with the bare expression
  instead of `return …;`, so the Linux + Windows CI gates compile
  warning-free again.

## 0.1.7 — beta (2026-05-04)

### Fixed

- **Updates panel showed "installed: unknown" and skipped CC CLI
  auto-updates with "no active CC install detected"** even when CC
  was installed and running. Root cause: `resolve_active_cli_binary`
  walked the Tauri process's `PATH`, but Dock-launched apps on macOS
  inherit the launchd default (no `~/.local/bin`, no
  `/opt/homebrew/bin`). The native install was enumerated by the
  explicit-path probe but never flagged active. Detection now
  promotes the highest-precedence detected install (native > brew >
  npm > linux pkg) when `PATH` lookup misses — same behavior covers
  users whose `claude` is a shell function or alias.

### Changed

- **Settings → About → Website** now points at
  `https://claudepot.com/app` (the product page) instead of the
  reader landing.
- **Global section tab order** is now `Config | Memory | Updates`
  — `Memory` sits next to the static reference tabs; `Updates` is
  the action surface and reads last.

## 0.1.6 — beta (2026-05-04)

### Fixed

- **Browser-login flow had no Cancel button after the RunningOps
  rewrite.** Once the login op moved through the shared
  `OperationProgressModal`, the modal only rendered "Run in
  background" / "Close" — there was no way to abort the in-flight
  `claude auth login` from the UI. The modal now carries a primary
  Cancel button while the op is in flight, on both the **Add
  account → Log in with a new account** browser-register path and
  the per-account **Re-login** path. The same dead end b61adda
  fixed for the inline modal had silently returned during the
  services rewrite.
- **WindowChrome home glyph misaligned** with the macOS traffic
  lights by a couple of pixels in the 38 px chrome strip.

### Changed

- **Operation progress modal renders cancellation as "Cancelled."**
  (info tone) instead of red "Error." when the terminal detail
  says the user cancelled — and suppresses the now-irrelevant
  "Open Repair" button on cancellation.
- **"Opening browser…" toast during re-login** is now an info
  toast (not error) and no longer carries a redundant Cancel undo
  affordance — the modal owns that action now.

## 0.1.5 — beta (2026-05-04)

### Fixed

- **Automations: "first-party `claude` binary not found on PATH" when
  registering a scheduled run.** The GUI walked the GUI process's
  `PATH` to resolve `claude`, but Dock-launched Tauri apps on macOS
  inherit only `path_helper`'s defaults (no `~/.local/bin`), so users
  on the Anthropic native installer (canonical layout since Sept
  2025) couldn't save an automation. The shim now invokes `claude`
  by name and resolves it against its own controlled `PATH` at run
  time. Bonus: automations stay correct across `claude doctor`
  upgrades — the version-specific symlink target rotates, but the
  symlink stays put.
- **Automations: shim runtime `PATH` now covers bun, npm-global, and
  Volta installs** alongside the existing Homebrew + system + native
  paths. The Anthropic native installer (`~/.local/bin`) is ranked
  first so a stale Homebrew copy can't shadow it.
- **Memory pane: Windows test failure for `anchor_uses_git_root_when_present`.**
  The test compared a path against `canonicalize`'s output without
  stripping the verbatim `\\?\` prefix; production code already
  routes through `simplify_windows_path`, so the test was the only
  thing wrong. `paths.md` rule now applies in the test too.

### Changed

- **Repo housekeeping.** rustfmt drift on `main` from the 0.1.4
  release commit reformatted across `claudepot-cli/src/commands`,
  `claudepot-core/src/{memory_log,memory_view,settings_writer}.rs`,
  and `main.rs`. No behavioral change — pure formatting.

## 0.1.4 — beta (2026-05-04)

### Added

- **Projects → Memory pane.** A third sub-tab next to Sessions and
  Config. For the project you have open: lists every memory artifact
  CC loads (project `CLAUDE.md`, `.claude/CLAUDE.md`, the auto-memory
  index + topic files, KAIROS daily logs, and the global
  `~/.claude/CLAUDE.md`); renders markdown content with a
  rendered/raw toggle; opens any file in your editor of choice via
  the existing "Open with…" detector; surfaces a per-file change-log
  timeline with collapsible unified diffs; toggles auto-memory at
  per-project scope (writes `.claude/settings.local.json`).
- **Settings → General · Auto-memory toggle.** Global on/off for
  CC's `autoMemoryEnabled` setting (writes `~/.claude/settings.json`),
  with read-only display when an env var is overriding.
- **CLI verbs.** `claudepot memory list|view|log [--show-diff]` and
  `claudepot settings auto-memory status|enable|disable|clear`.
- **`.claude/rules/icon-buttons.md` and `.claude/rules/path-display.md`.**
  Two project rules that codify when to use icon-only buttons (3-tier
  system) and how truncatable paths must be disclosed (tooltip
  mandatory; copy default unless covered by a canonical detail
  surface).

### Changed

- Long-running fs-watcher backs the change-log: 250 ms debounced,
  recursive on `~/.claude/`, plus per-file watches on every
  registered project's CLAUDE.md candidates. Picks up newly-added
  projects on a 30 s rescan; recovers original paths for lossy
  (>= 200-char) slugs via session.jsonl `cwd`.
- `ArtifactTrashList`, `DisabledArtifactList`, `ProtectedPathsPane`
  per-row Restore/Forget/Re-enable/Trash/Remove actions converted
  to `IconButton` with tooltips per the icon-button rule.
- `ProjectsTable` row + `WindowChrome` breadcrumb now disclose full
  paths via tooltip per the path-display rule.
- Cross-platform basename helper in `projects/format.ts` — Windows
  paths in the Projects table and rename progress now display
  correctly.
- `Toggle` primitive in `SettingsSection` honors `disabled`
  (regression: previously accepted the prop but ignored clicks).

### Fixed

- **Symlink escape in `read_memory_content`.** The IPC now
  canonicalizes the target path before the allowlist check, so a
  symlink inside the auto-memory dir whose target is outside the
  scope is rejected before `std::fs::read` follows the link.
- **Memory IPC dies when `MemoryLog::open` fails.** Two-step
  fallback (canonical → temp dir) so the change-log state is always
  managed; the pane no longer goes dark on a transient open error.
- **Global toggle conflated user and project settings.** Settings →
  General now uses a dedicated global-only resolver instead of
  routing through the per-project chain (which would read
  `~/.claude/settings.json` twice as both user- and
  project-settings).
- **First post-restart edit logged with no diff.** Bootstrap now
  re-reads current bytes so the watcher has a real baseline for
  the first event after a Claudepot restart.

## 0.1.3 — beta (2026-05-03)

Patch release adding a network-status indicator. One small dot in the
StatusBar that answers two questions Claudepot users were asking
indirectly: "is Claude up?" (status.claude.com poll) and "is my path
to Claude fast right now?" (HEAD probe to the hosts CC actually hits
at startup). On-demand for the latency probe — no continuous
background polling, by design.

### Added

- **Service status dot in the StatusBar.** Color-coded (green / amber
  / red / grey) showing the worst-of two signals: the
  `status.claude.com/api/v2/summary.json` page tier × per-host
  latency to the hosts Claude Code actually pings at startup. Hover
  for the per-host latency table, active incidents, and last-poll
  age; click to re-probe. Hidden when both Network toggles are off.
- **Settings → Network.** New tab (core group, globe glyph) with
  toggles for status-page polling, poll interval (2–60 min),
  on-focus latency probing, and OS-notification on status
  transitions. OS notification is off by default — false-positive
  Anthropic blips would train users to ignore real signals.
- **Status-page transitions land in the bell-icon notification log.**
  Background poller (5 min default, gated by the user setting)
  detects OK ↔ Degraded ↔ Down transitions and writes a
  `Notice`-kind entry to the existing `notification_log` ring
  buffer, so the bell popover is the persistent record. OS banner
  is the opt-in surface on top of that.

## 0.1.2 — beta (2026-05-03)

Patch release on top of the first beta. Mostly UI polish — error
boundaries per section so a thrown render in Sessions doesn't take
the whole window with it, modal accessibility cleanup, and a sweep
of token literals that were rendering as raw CSS strings instead of
pulling from the design-token catalog. A handful of real fixes
underneath: a toast leak that piled up dispatched entries, a
`.expect()` in the restore-from-trash path that could panic on a
race, and `--quiet` finally suppresses progress lines on the
activity / cli-ops CLI handlers.

### Added

- **About → Website row.** New link to <https://claudepot.com>
  alongside the existing GitHub / Anthropic links. Brand-mark
  exception applies — uses the GitHub mark inline-SVG for the
  trademarked logo (see `.claude/rules/design.md`).

### Changed

- **Run history is a table now.** `RunHistoryPanel` switched from
  cards to the lifted `Table` primitive — the section was already a
  scan-and-drill surface (one verb per row, likely > 20 rows) so
  cards were the wrong container per the paper-mono design rules.
- **Modal width API is tighter.** Modals now declare a single
  width-cap token instead of overriding `max-width` inline; the
  command palette and the per-account modals re-use the same caps.

### Fixed

- **Per-section error boundaries.** A render panic in one section
  no longer wipes the rest of the window — each top-level section
  now ships its own `ErrorBoundary` and the command palette
  recovers when its backdrop is clicked through.
- **Modal a11y auto-wire.** `role="dialog"`, `aria-modal`, and
  `aria-labelledby` are now wired automatically when a `Modal` has
  a heading; manual props are no longer required.
- **Toast leak in the bell-icon popover.** Dispatched toasts were
  being added to the notification log but the in-memory ledger
  never expired completed entries; bell-icon counts could climb
  unbounded across a long session.
- **Restore-from-trash race.** A collision between the restore
  target path and a same-name file created concurrently could
  trigger a `.expect()` panic; the restore now resolves collisions
  atomically.
- **CLI `--quiet` honored on activity / cli-ops verbs.** Progress
  `eprintln!` lines that should have been suppressed under
  `--quiet` were leaking through; both handler families now gate
  progress output on the flag.
- **Seven broken design-token literals.** Inline styles were
  referencing tokens that didn't resolve (mostly mis-cased custom
  property names); they now go through `tokenize` and render the
  intended values in both light and dark themes.

## 0.1.1 — beta (2026-05-02)

First beta release. The version scheme tier crosses from `0.0.x`
(alpha) to `0.1.x` (beta) — same daily-driven build as 0.0.20, with a
substantially expanded Activities → Cost surface and a new memory-
health surface on Global. Everyone gets a one-time re-scan of
`~/.claude/projects/` on first launch (the schema_version bump from 2
to 3) so historical transcripts populate the new per-turn data store;
the cold scan of ~6 k JSONL files takes ~10 s and never blocks the
UI.

### Added

- **Activities → Cost gets a per-row Cache hit % column and a per-row
  Models column.** Cache-hit is computed client-side from the existing
  token totals (`cache_read / (input + cache_creation + cache_read)`)
  and surfaced both as a sortable column and as a sub-line on the
  install-wide "Tokens in" tile. Models is a new badge group on each
  project row showing how many sessions used each model (Opus, Sonnet,
  Haiku) — sessions that mixed models contribute to every bucket they
  touched. Both fields cost zero new disk space; they read from data
  the session index already had.

- **Pricing tier picker (Anthropic API / Vertex Global / Vertex
  Regional / AWS Bedrock).** Choose the platform you're billed
  through; the active tier renders alongside the source freshness in
  the pill ("Anthropic API · bundled · verified 2026-01-15") and is
  persisted to `~/.claudepot/preferences.json`. Every published
  Claude model is currently at parity across the four tiers (verified
  against Anthropic, Bedrock, and Vertex rate cards on 2026-01-15);
  the multipliers will diverge in code only when a specific premium
  is verified on a primary source. The picker is a transparency
  surface today, not a different number.

- **Per-turn token usage in `sessions.db`.** Every assistant message
  in every transcript now persists as one row in a new `session_turns`
  table: turn ordinal, timestamp, model, the four token fields, and a
  redacted preview of the user prompt that drove the turn. Replace-all
  semantics on every re-scan keep the cache consistent with growing or
  shrunk transcripts. `delete_row` cascades — when a transcript
  vanishes from disk, its turn rows go with it. The schema bump
  (v2 → v3) triggers a one-time re-scan on first launch so historical
  transcripts populate this table immediately rather than waiting on
  natural mtime changes.

- **Top costly prompts panel** on Activities → Cost. Below the per-
  project table, a compact ranked list of the install's five
  costliest prompts in the active window, each row showing the
  truncated prompt, the project, the turn ordinal, the model badge,
  and the computed dollar cost. The ranking is two-stage: SQLite
  pulls a coarse top-N×50 candidates by total token count (a fast
  cost-proxy), then Rust re-ranks against the active price table
  because Opus tokens cost ~20× Haiku tokens and the proxy can
  reorder across model families. Unresolved-model rows are dropped
  rather than surfaced with null costs.

- **Global → Memory tab with CLAUDE.md / MEMORY.md health cards.**
  Static analysis on `~/.claude/CLAUDE.md` and
  `~/.claude/memory/MEMORY.md`: line count, char count, lines past
  CC's truncation cutoff (200 lines for global memory), and a rough
  token estimate (`char_count / 4`). The "past line N" tile turns
  warning-coloured and the card's left border picks up the warning
  accent when any content sits past the cutoff — a glanceable cue
  that you've shipped instructions Claude Code can't actually see.
  Pure read; no edit affordances.

### Changed

- **Activities → Cost summary "Tokens in" tile shows install-wide
  cache hit rate** in its sub-line ("cache hit 83%") instead of just
  raw cache-read tokens. Cache-hit is the single number that
  describes how cheaply the prompt cache is doing its job.

- **Activities → Cost "Sessions" tile renders `—` for empty windows**
  instead of a literal `0`, matching the project's render-if-nonzero
  rule. The empty-state notice in the table below already conveys
  "no sessions"; a numeric tile competing for attention was visual
  noise.

- **Activities → Cost ascending sort puts unpriced rows last.**
  Sorting cost / last-active / cache-hit ascending used to surface
  unpriced rows above every priced row (the comparator put nulls
  first; the descending reverse hid the bug for the most common
  view). Sort now partitions nulls explicitly so they always land at
  the end regardless of direction.

### Fixed

- **`last_user_prompt` carry-over no longer leaks into the wrong
  assistant turn.** A user line without extractable text (image-only
  message, tool-result-only, caveat-stripped CLI command) used to
  leave the per-turn carry pointing at the previous text prompt, so
  the next assistant turn would render someone else's prompt in the
  Top costly prompts panel. The carry now clears on every user line
  regardless of payload.

- **`pricing_tier_set` no longer leaves disk and memory out of sync
  on a failed save.** The setter previously mutated the in-memory
  preference *before* attempting the disk write; an out-of-space or
  permission-revoked error left the running app on the new tier
  while `preferences.json` stayed on the old one. Saves a clone
  first, only commits to in-memory on success.

- **`memory_health` no longer overcounts past-cutoff bytes by 1 for
  files that lack a trailing newline.** A `~/.claude/CLAUDE.md` of
  exactly 250 lines whose last line had no `\n` reported one phantom
  newline in `chars_past_cutoff`. Edge case, but the fix is one
  branch and a regression test.

- **Activities → Cost dashboard no longer doubles its filesystem
  walk per tab tick.** The aggregate query and the top-prompts query
  both refresh `sessions.db` against `~/.claude/projects/`; running
  them as `Promise.all` walked the directory twice. Frontend now
  serializes them and the second call passes `refreshIndex: false`,
  collapsing the per-tick work to one walk.

- **Stale data is cleared on fetch error.** A failed
  `local_usage_aggregate` call used to leave the previous report's
  summary tiles and project rows in place beside a fresh error
  banner. The pane now blanks on error, which is more honest than
  showing yesterday's numbers under today's failure.

- **Tier picker hydrates from persisted preference on cold start.**
  Previously the picker fell through to `anthropic_api` until the
  first aggregate fetch landed, which flickered for users on
  Bedrock or Vertex. The picker now hits `pricing_tier_get` on
  mount.

- **Inline-style CSS values use real `var(--*)` references.** The
  `TopPromptsPanel` and `MemoryHealthPanel` initially shipped with
  invalid token strings (`"tokens.sp[28]"`, `"tokens.settings.nav.width"`)
  because bare pixel literals were silently rewritten by an editor-
  side tooling hook. Replaced with the canonical `var(--sp-28)`,
  `var(--sp-60)`, `var(--settings-nav-width)` form so the layout
  actually applies.

## 0.0.20 — alpha (2026-05-01)

### Fixed

- **Global → Updates toggles persist clicks again.** Every toggle on
  the Updates panel (CLI auto-update, CLI/Desktop tray-badge notify,
  CLI/Desktop OS-banner notify, Desktop auto-install-when-quit)
  appeared frozen: clicks fired no error and the UI reverted to the
  prior state on the next refresh. The renderer was sending
  snake_case keys (`cli_notify_on_available`, …) to the
  `updates_settings_set` Tauri command, but Tauri 2's IPC layer
  expects camelCase from JS and auto-converts to snake_case for the
  Rust args — so every `Option<bool>` arg deserialized to `None`,
  the handler wrote nothing, and `~/.claudepot/updates.json` stayed
  at its prior values. Renamed the `UpdatesSettingsPatch` interface
  fields and the six call sites in `UpdatesPanel.tsx` to camelCase,
  matching every other API file in the codebase. Existing settings
  on disk are preserved.

## 0.0.19 — alpha (2026-05-01)

### Fixed

- **Collapsed the duplicate label on Settings → Cleanup → Rebuild
  session index.** The card heading already established the noun
  ("Rebuild session index"); the trigger button below repeated the
  full phrase verbatim, so the card read its own name twice. The
  button now reads just **Rebuild** — the heading is the noun, the
  button is the verb. The confirm dialog already used the same
  shorter `Rebuild` label, so the click-through path is now
  consistent end-to-end.

## 0.0.18 — alpha (2026-05-01)

### Added

- **In-app notification history (bell icon).** Toasts and OS desktop
  notifications used to be fire-and-forget — only the last dismissed
  toast echoed in the status bar; OS banners lived only in the OS
  Notification Center, where macOS aggressively expires them and
  Linux libnotify often doesn't surface them at all. A new bell icon
  in the top chrome (between the ⌘K palette hint and the theme
  toggle) shows an unread badge and opens a popover with the
  persisted history: filter by kind (info / notice / error), source
  (in-app / OS / both), time window (1h / 24h / 7d / all), and
  free-text search; sort newest- or oldest-first; mark-all-read on
  open; clear via a confirm dialog. Logged entries with click
  targets route through the same `NotificationTarget` discriminator
  as fresh banner clicks, so a click in the popover lands the user
  wherever the live banner would have. Persisted as a 500-entry
  ring buffer at `~/.claudepot/notifications.json`; corrupt files
  move aside to `.corrupt` and the log starts empty rather than
  wedging the bell.

- **Focus-gated notifications now surface in the bell.** Pre-fix,
  `dispatchOsNotification` dropped the entire dispatch when the
  window had focus — for usage-threshold and other no-toast surfaces,
  focused users got nothing (no banner, no toast, no log entry). The
  dispatcher now writes to the notification log BEFORE the focus /
  permission / rate-limit gates, so the bell catches every intent
  regardless of OS delivery. Permission-denied and rate-limited
  dispatches log too — the user can scroll back through what
  Claudepot wanted to tell them even when the OS center suppressed
  the banner.

### Changed

- **Usage-threshold notifications cut from ~10/day to ~1/day per
  active account.** Four targeted changes: default threshold list
  trimmed `[80, 90]` → `[90]` (one near-cap nudge per cycle instead
  of two — add `80` back via Settings if you want the early warning);
  within-poll coalescing in `apply_crossings` so a 50→95 utilization
  jump emits one "at 90%" Crossing instead of "at 80%" + "at 90%"
  back-to-back; per-model 7-day sub-windows (Opus, Sonnet) now
  opt-in via a new `notify_on_sub_windows` preference (default off —
  the umbrella `seven_day` window is always checked, and the
  sub-quotas typically track it for users near cap, so leaving them
  on tripled the 7-day toast volume for what users perceive as one
  cap); and focus-suppressed dispatches now log to the bell so
  focused users don't silently miss crossings. Settings →
  Notifications gains the `Include 7-day Opus / Sonnet sub-windows`
  toggle.

### Fixed

- **Tokenized the usage-threshold chip group in Settings.** The
  chip styles referenced `var(--radius-sm)`, which doesn't exist in
  `tokens.css` — chips rendered with sharp corners due to the CSS
  fallback to `0`. Also fixed a raw `gap: 6` literal and a misuse of
  the `--sp-px` spacing token as a border-width. Chips now render
  with the documented rounded corners using `var(--r-1)` and
  `var(--bw-hair)`.

## 0.0.17 — alpha (2026-05-01)

### Added

- **Auto-update manager for Claude Code CLI and Claude Desktop**
  (Global → Updates tab). Detects every install on the box (native
  curl, npm global, Homebrew Cask `claude-code` / `claude-code@latest`,
  apt/dnf/apk, WinGet) and the active Desktop install (Homebrew Cask,
  direct DMG, Setapp, Mac App Store, user-local). Surfaces installed
  vs upstream-latest with a colour-coded delta, a one-click "Update
  now" that routes through the right channel for each kind (`claude
  update` / `brew upgrade --cask` / direct .zip with `codesign`
  verification + `ditto` install + quarantine strip), a "Check now"
  refresh, and a `Channel` toggle that mirrors CC's `autoUpdatesChannel`
  with `latest → stable` minimum-version pinning (`--allow-downgrade`
  CLI flag, opt-in clear). All of this is also driven from
  `claudepot update {check,cli,desktop,config}` — same code path,
  same single-flight gate.

- **Background updates poller** that probes the version endpoints on
  a cadence (default 4 h, 30 min – 24 h via `poll_interval_minutes`),
  updates the tray badge, and runs the auto-install pass when the
  toggles allow. Single-flights against manual clicks via a shared
  `Arc<PollerGate>` so a user clicking "Update now" while the poller
  is mid-cycle gets a clean refusal instead of two `claude update`
  invocations racing.

- **OS-notification toggle (per surface)** alongside the existing
  tray-badge toggle, so tray-only users can opt into a system toast
  when an update is detected without making the tray noisy. Deduped
  per-version so the toast fires once per new release, not on every
  poll. Uses `tauri-plugin-notification` Rust-direct so it works when
  Claudepot's main window is closed.

- **`claudepot update` CLI verbs**: `check` (probe + table), `cli`
  (force `claude update`), `desktop` (force install via brew or
  direct DMG), `config` (show / set channel + toggles). All support
  `--json` for scripting.

### Changed

- **Global section now hosts two tabs**: Config (the existing
  user-wide CC config tree, unchanged) and Updates (new). Tab
  selection persists in localStorage; sub-routes inside Config
  continue to work as before.

## 0.0.16 — alpha (2026-05-01)

### Fixed

- **Tray-initiated CLI switch is now one-click with OS-notification
  feedback.** Picking an account from the menubar's "Switch CLI"
  submenu used to route the live-session conflict back through the
  webview to raise `SplitBrainConfirm` — invisible when the window
  was hidden, so the click looked dead and the user had no way to
  know what happened or how to proceed. The tray now force-switches
  in one click, shows a 10-second Undo toast in-window, and
  dispatches an OS notification when the window isn't focused. The
  notification body surfaces the "restart Claude Code to apply"
  caveat when CC was running at the time of the switch, since CC's
  next token refresh can otherwise revert the swap silently. Tauri
  2's desktop notification plugin can't render action buttons, so
  the notification deep-links to the Accounts section where the
  Undo toast (still alive within its 10s window) carries the
  actual click. Backed by a new `CliOpState` async mutex that
  serializes tray clicks so two rapid swaps can't both record a
  stale "from" account in the undo target.
- **macOS proxy detection now handles SOCKS-only and PAC setups.**
  The Finder/Dock launch path previously read only `HTTPSEnable`
  from `SystemConfiguration` — SOCKS-only setups (Surge, Clash,
  ssh -D) silently went direct, and PAC setups failed with no
  diagnostic. Detection now classifies HTTPS → SOCKS → PAC: SOCKS
  uses `socks5h://` so DNS resolves through the proxy (which
  matches the typical local-rules-engine setup), and PAC is
  surfaced as a typed `MacosPacUnsupported(url)` warning in
  `claudepot doctor` (Claudepot doesn't ship a JS engine, so
  evaluating `FindProxyForURL` is out of scope — the URL is
  redacted of any embedded credentials before display). Set-but-
  malformed proxy env vars (`HTTPS_PROXY=garbage`) used to silently
  disable the proxy entirely; they now fall through to the next
  source with a `tracing::warn!` so the user still gets system
  proxy as a fallback.
- **OAuth HTTP client picks up proxy changes mid-session.** The
  shared client was a `OnceLock<reqwest::Client>` built on first
  use and never rebuilt. Toggling the system proxy or setting
  `HTTPS_PROXY` after the app launched had no effect until the
  next restart. The client now re-detects on every call with a
  `(url, no_proxy)` cache key — same key, cached `Arc<Client>` is
  cloned (cheap); changed key, the client is rebuilt. `apply()`
  no longer falls back to env `NO_PROXY` so the cache key
  faithfully reflects every input that affects the built client.

## 0.0.15 — alpha (2026-04-30)

### Fixed

- **Destructive-confirm dialogs now show the project's distinguishing
  tail.** "Clean project data", recovery snapshot lists, and the
  Settings → Cleanup → Protected paths list truncate paths with
  `text-overflow: ellipsis`. Because every path begins
  `/Users/<user>/…`, the visible portion (`/Users/…`) was the
  shared prefix and the project basename — the only thing that
  differs between rows — got hidden behind the ellipsis. Two rows
  in a "Remove 2 projects" confirmation rendered as identical
  `/Users/…` lines, leaving no way to tell what was about to be
  deleted. Truncation now flips to the head so the basename stays
  visible (`…/myprojects/claudepot-app`); the full path stays
  selectable and on hover.

## 0.0.14 — alpha (2026-04-30)

### Added

- **Skeleton placeholders for list surfaces.** Bare "Loading…" text on
  list/grid panes flashed a single word where structure was about to
  appear. New `Skeleton` / `SkeletonList` / `SkeletonRows` primitives
  (`src/components/primitives/Skeleton.tsx`) wrap the existing
  `.skeleton` CSS classes with `role="status"` + `aria-live="polite"`
  + a visually-hidden label, so the loading state announces itself
  to screen readers while the sighted UI shows shimmer blocks.
  Applied at Accounts (initial load), Keys (both tables), Activities
  card stream, Automations, Third-parties, Config preview, Settings
  notifications + diagnostics panes, and Protected paths.
- **Inline reasons on disabled buttons.** Settings → Cleanup
  "Break lock" now shows "Enter a lock file path" / "Breaking lock…"
  beside the disabled button. Settings → Updates "Frequency" row
  hints "Enable auto-check to set" when disabled. The auto-update
  "Check now" button switches its label by status (Checking… /
  Downloading… / Update ready) instead of staying as a flat
  "Check now". Closes a long-standing `.claude/rules/design.md`
  violation.
- **Empty-state CTAs in Keys.** Both API keys and OAuth tokens now
  render a ghost "Add …" button in their empty state, alongside
  the existing console link / `claude setup-token` instructions.
  Header solid-Add stays the single primary action per
  one-primary-per-view rule.
- **Inline path validation in the recovery dialog.** Settings →
  Cleanup → Recover now surfaces "Path must be absolute (starts with
  `/` or `C:\`)" beneath the input when the path is non-empty but
  relative, instead of silently disabling the Recover button.

### Changed

- **Notifications carry the project name in the body.** macOS stacks
  banners by `threadId` and shows only the body line in the
  Notification Center summary, so a stand-alone "task finished (3m)"
  lost its project attribution the moment two finished near each
  other. Body now appends `@ <project>` for all four kinds (error,
  stuck, idle-done, waiting). Title still carries the project for
  the un-stacked banner case.
- **Download progress visible on the button.** The auto-update card
  already had a progress bar in the same card; the disabled
  "Downloading…" button itself now shows the percentage too
  (`Downloading… 42%`), so the user gets progress at a glance even
  when the bar scrolls out of view.
- **Terminology: "transcript" → "session" in user-facing strings.**
  Project detail context menu items ("Reveal session in Finder",
  "Copy session file path") and adopt-orphans dialog copy now match
  the section's "Sessions" tab label. Internal variable names
  (`transcriptPath`, etc.) stay — only user-visible strings changed.
- **Specific failure copy when accounts can't load.** The startup
  load-error screen now says "Couldn't load accounts" instead of
  the misleading "Couldn't load Claudepot", with one actionable
  sentence and the raw error tucked behind `<details>`.

### Fixed

- **OAuth token rows no longer show truncated UUIDs when the linked
  account is removed.** Previously the row rendered
  `account_uuid.slice(0, 8)` as a fallback (e.g. `a3f8c2d1`),
  violating the "no internal identifiers in primary UI" rule. Now
  shows a `warn`-tone "account removed" tag, and the tag stays
  clickable for cached-usage inspection (cache is keyed by token
  UUID, not account, so the lookup still resolves). Same fix lands
  in the OAuth usage modal title.
- **Skeleton primitive carries a screen-reader label.** First pass
  introduced shimmer blocks with no `role="status"` or aria text,
  which would have regressed the a11y floor for every section that
  swapped "Loading…" for the new primitive. Container now declares
  `role="status"` / `aria-live="polite"` / `aria-busy="true"` and
  carries a visually hidden "Loading…" label.

## 0.0.13 — alpha (2026-04-30)

### Fixed

- **macOS menubar tray icon visible again on dark menubars.** The
  pixel-art ghost was rasterized with binary alpha (every silhouette
  pixel at full opacity) and white-fill eyes, then submitted to
  `tray.set_icon` on every menu rebuild. `tray-icon`'s macOS impl
  hard-codes `setTemplate(false)` inside `set_icon`, so the template
  flag we set at startup was stripped on the first rebuild and AppKit
  rendered the raw bitmap — a pure-black silhouette on a near-black
  menubar = invisible. Now we re-apply `set_icon_as_template(true)`
  after every swap, and the SVG sources encode the eyes as
  transparent gaps (so the body tints to the menubar foreground while
  the eyes punch through) — matching the original ghost design.
- **Dropdown menu icons readable on Light-mode menus.** The Lucide
  glyph stroke was painted `#888888` (~53% luminance), which sat
  almost on top of the macOS NSMenu Vibrant Light material
  (~rgb(160,160,160)) and read as invisible. Re-rasterized at
  `#3a3a3a` (~22% luminance) so the icons read as dark strokes
  against the light dropdown bg, alongside the menu's black text.
  muda 0.17 doesn't expose a template-tint hook for custom bitmaps,
  so this single value targets Light dropdowns; paired Dark assets
  will land if Dark-appearance users report low contrast.

## 0.0.12 — alpha (2026-04-30)

### Fixed

- **Proxy env vars work for rustls-tls clients.** (#2, thanks
  @XIYINGDU.) Two trapped bugs prevented `https_proxy` from being
  honoured when launched from a shell with the variable set:
  `reqwest` built with `default-features = false` + `rustls-tls`
  doesn't auto-discover system proxy, and the previous `or_else`
  chain short-circuited on `HTTPS_PROXY=""` (env-set-but-empty),
  so the lower-precedence variants were never reached even when
  they held the actual value. The fix iterates
  `[HTTPS_PROXY, https_proxy, ALL_PROXY, all_proxy]`,
  filter-maps to non-empty, and explicitly hands the first match
  to `reqwest::ClientBuilder::proxy(...)` plus
  `NoProxy::from_env()` so existing `NO_PROXY` exclusions are
  preserved. Applied to both the shared OAuth client
  (`oauth/mod.rs`) and the one-off doctor client
  (`services/doctor_service.rs`). Known follow-up: launching from
  Finder/Dock doesn't inherit shell env, so the proxy still
  isn't found in that path — tracked in #4 for a macOS
  `SystemConfiguration` lookup.
- **Release build is warning-clean on Windows and Linux.** The
  0.0.11 release CI emitted 13 dead-code warnings on Windows
  (`render_script` and 9 helpers in `routes/wrapper.rs`,
  plus two unused imports — `write_wrapper` early-returns on
  non-Unix because Windows `.cmd` wrappers are out of scope, so
  the lib never reaches them) and an `unused_variable: hide_dock`
  warning on both Linux and Windows (consumed only inside
  `#[cfg(target_os = "macos")]`). Annotated each item with
  `#[cfg_attr(not(unix), allow(dead_code))]` /
  `#[cfg_attr(not(unix), allow(unused_imports))]` and gated the
  `hide_dock` bind to macOS. Tests still reach the wrapper
  helpers on every platform (they're pure string checks, no
  fs/perms), so cfg-gating the `fn` definitions themselves
  would have forced a parallel cfg-gate sweep across the test
  suite — the per-item `cfg_attr` is precise without that cost.

## 0.0.11 — alpha (2026-04-30)

### Added

- **OS notification when a CLI session is waiting for you.** The
  highest-leverage trigger in the set: when CC pauses pending a
  permission, plan-mode approval, or clarifying answer, a toast fires
  with the project name as title and the `waiting_for` reason as
  body. Click the toast to bring the host terminal forward (same
  routing as the existing error/stuck toasts). Detection rides the
  existing `Status::Waiting` field already populated from CC's PID
  file `waitingFor` and the `permission-mode` transcript fallback —
  no new polling. Re-fires only when the session leaves Waiting and
  re-enters with a *different* reason, so a multi-turn approval flow
  doesn't spam. Defaults **on** because it's the alert the product
  exists for; the activity feature itself is already opt-in
  (`activity_enabled`), so a fresh-install user doesn't see surprise
  toasts before consenting to live tracking. New
  `notify_on_waiting` preference under Settings → Notifications.
- **OS notification when an Anthropic usage window crosses a
  threshold.** New `Alert at usage thresholds` chip group under
  Settings → Notifications (50 / 70 / 80 / 90 / 95) — defaults
  to `[80, 90]`. A Rust-side watcher polls `/usage` every 5 min
  for the CLI-active account and emits one toast per (window ×
  threshold) per reset cycle. The fired-set persists to
  `~/.claudepot/usage_alert_state.json` so a restart doesn't dupe;
  cycle resets clear the set so the next cycle re-arms. Click the
  toast to open the Accounts view for that email. Independent of
  `activity_enabled` — usage polling has no dependency on the
  transcript runtime.
- **Tray icon shows a dot when sessions need attention.** The
  menubar icon switches to an alert variant (same teapot glyph plus
  a 2 × 2 black square in the top-right corner) whenever any session
  is errored, stuck, or waiting. Both variants ship as template PNGs
  derived from `assets/pixel-claudepot-menubar.svg` /
  `pixel-claudepot-menubar-alert.svg` via `rsvg-convert`, so AppKit
  re-tints the dot to match the menubar foreground in light + dark
  modes. Replaces the previous `• N` text-title approach, which was
  macOS-only (GNOME hides title text; Windows ignores it) — the
  icon swap is the visible signal on every platform Tauri targets.

### Changed

- **⌘Q, ⌘W, and the red ✕ now hide the window instead of quitting.**
  Claudepot is meant to live in the menubar; background watchers
  (live activity runtime, usage poller) and OS notifications keep
  firing only while the process is alive, and the previous
  behaviour ended the process whenever the main window was the
  only one. The single Quit that actually exits the process is the
  `Quit` row in the tray dropdown — it routes through `attempt_quit`
  and the existing RunningOps gate so in-flight project renames /
  prunes / verifies surface a confirm modal before being abandoned.
  ⌘Q's old in-app accelerator is rebound to the same hide handler;
  Window menu's `Close Window ⌘W` is a custom item (not the Tauri
  predefined, which would tear the window down); red ✕ intercepts
  `CloseRequested` and calls `prevent_close` + `hide()`.
- **Settings tab "Activity" → "Notifications".** The pane was
  renamed and given the Bell icon; the body still hosts the live-
  activity master switch alongside the per-trigger toggles, since
  the trigger detection rides the live runtime. Pane copy now states
  the actual defaults (waiting and usage thresholds default on; the
  rest are opt-in) instead of the previous "all default off"
  promise that was no longer true.
- **"Alert when work completes" relabelled "Alert when task
  finished".** Same preference (`notify_on_idle_done`), same 2-min
  busy gate (the gate filters out drive-by edits so every successful
  turn doesn't fire a toast). Notification body now reads
  `task finished (Nm)` instead of `done (Nm)`.
- **Tray dot is just a dot — no count.** The previous title text
  rendered `• N` next to the menubar icon; the user's next action is
  binary regardless (open the app to see what), so the count was
  cognitive load. The hover tooltip still surfaces the count
  (`⚠ N alerting sessions`) for callers who want it on demand.
- **Waiting sessions count toward the tray dot.** The "alerting"
  count was `errored + stuck` only — the dot would dark while a
  session was paused for a permission, exactly the case the
  product exists for. Now `errored + stuck + waiting` lights it up.

### Fixed

- **Cold-install users get the documented defaults.** `Preferences`
  derived `Default`, which set every field to its type default —
  so the documented `notify_on_waiting = true` and
  `notify_on_usage_thresholds = [80, 90]` came up `false` and `[]`
  whenever `preferences.json` was missing on disk. Manual `Default`
  impl now reuses the same helpers serde uses for partial-read
  defaults, so cold start agrees with field-level fallback.
- **Usage threshold polling decoupled from the live activity
  feature.** A user who disabled the live Activity feature for
  privacy reasons would also lose their quota alerts, even though
  `/usage` polling has no dependency on transcript watching. The
  watcher now keys only off `notify_on_usage_thresholds`.
- **First-tick race for usage notifications mitigated.** The Rust
  watcher used to run its first tick immediately on app start; if
  the first tick crossed a threshold before the renderer's listener
  was wired up, the fired-set persisted but no OS toast fired.
  A 5 s `FIRST_TICK_DELAY` now gives the webview time to mount
  before the watcher's first emit.
- **Usage save errors no longer cause dupe-on-restart.** When the
  alert-state save to `~/.claudepot/usage_alert_state.json` failed
  (disk full, permissions), the previous code logged only the
  outer `JoinError`, dropped the inner `io::Error`, and emitted the
  toast anyway — guaranteeing the same threshold re-fired on the
  next launch. Both error layers now surface in the journal, and
  emit is suppressed when persistence fails. Trade: a rare missed
  alert vs. a rare dupe; the dupe is the more annoying outcome.
- **Settings usage-threshold chips render correctly.** The chip
  styles had been auto-rewritten to invalid token strings
  (`"tokens.sp[2] tokens.sp[8]"`, `"tokens.sp.px solid …"`); the
  browser dropped the declarations and the chips lost their
  padding/border. Replaced with valid CSS vars (`var(--sp-2)`,
  `var(--sp-8)`, `var(--sp-px)`).

## 0.0.10 — alpha (2026-04-29)

### Added

- **`claudepot --version`.** Prints `claudepot <semver>`. Wired via clap's
  built-in `version` attribute against `CARGO_PKG_VERSION`, so the CLI's
  reported version stays in lock-step with the workspace bump.
- **Notification clicks route to where the work actually lives.**
  Session notifications (errored / stuck / idle-done / card-emitted
  Warn+) now bring forward the terminal or editor that's running
  `claude`, not Claudepot. Implemented as a focus-event heuristic
  with a 10 s TTL queue (the Tauri 2 desktop notification plugin
  doesn't surface body-click events to JS, verified by reading
  tauri-plugin-notification 2.3.3's `desktop.rs`). The new
  `notification_activate_host_for_session` Tauri command walks the
  session's process tree via sysinfo, matches the topmost ancestor
  against a hardcoded macOS terminal/editor table (Terminal, iTerm2,
  Alacritty, kitty, Ghostty, WezTerm, Hyper, Tabby, Warp, VS Code,
  Cursor, Windsurf), and asks LaunchServices (`open -b`) to bring
  it forward. Falls back to deep-linking the transcript inside
  Claudepot when the host can't be resolved (SSH'd sessions,
  daemonized runs, unknown bundles). Op-completion notifications
  still focus Claudepot itself — those ARE about Claudepot's state.
- **Activities → Cost — GUI surface for the local cost report.**
  New tab inside the Activities section (alongside Stream and
  Usage) showing the same per-project token + USD totals as the
  CLI, with a window selector (7d / 30d / 90d / all), four summary
  tiles (Total cost · Tokens in · Tokens out · Sessions), and a
  sortable table. Project rows display the CWD's basename with the
  full path on hover; cost-desc is the default sort; columns
  toggle ascending/descending on re-click. A pricing-source pill
  ("bundled · verified 2026-01-15", "live · 2h ago") declares the
  trust signal on the figure, and a footer note plus a Refresh
  prices button surface when any session lacked a priced model.
- **`claudepot usage report` — local cost tracking from on-disk
  transcripts.** New CLI subcommand that rolls up token counts and
  USD cost per project, with `--window all|<n>d` for time-bounded
  views and `--json` for scripts. Mirrors CC's own `/usage` "this
  install" totals — no extra network call; cost computed against
  claudepot's bundled price table. Per-account attribution is
  intentionally omitted (CC transcripts don't carry an account id,
  and claudepot keeps no swap-event log to reconstruct one);
  building that infrastructure is reserved for a separate change.
  Sessions whose models aren't in the price table contribute their
  token totals but not their cost, with a footer note calling out
  the unpriced count so the gap is visible rather than silent.
- **OS notification when a long operation finishes.** New
  `Alert when long operations complete` toggle under
  Settings → Activity → Notifications. When the main window is
  unfocused, verify-all, project rename, session prune/slim/share/move,
  account login/register, clean projects, and automation runs all post
  a system-level notification on completion. The single `cp-op-terminal`
  channel emitted from `ops::emit_terminal` is the source — every op
  type funnels through one place, so future ops light up the toggle for
  free.
- **Tray reflects alerting sessions.** macOS shows a `• N` badge next
  to the menubar icon when sessions are errored or stuck; every
  platform receives a tooltip suffix (`⚠ N alerting sessions`). The
  count survives full menu rebuilds (account adds, syncs, etc.) so a
  tray-only user has a persistent signal instead of just transient OS
  toasts. Click the tray icon → existing menu — no extra UI.
- **Notification permission status in Settings.** The Notifications
  group now opens with the OS permission state (`Granted`, `Denied`,
  `Not requested`) and a Request button when applicable. Toggling a
  notification class against denied permission no longer fails
  silently — the row spells out the current state and points to
  System Settings when reset is needed.

### Fixed

- **Same-basename projects no longer notify under identical titles.**
  Two live sessions in `~/work/foo` and `~/personal/foo` used to
  emit notifications whose titles both read `foo`, indistinguishable
  in macOS Notification Center. The activity-notifications hook now
  computes a per-render label map: pure basename when unique,
  `parent/basename` when colliding, so the disambiguator only appears
  where it actually matters.

### Changed

- **OS notifications respect window focus.** When Claudepot is the
  foreground window, OS toasts are suppressed in favour of the in-app
  signal that already shows the same state (errored row, banner,
  running-ops chip). Blurred windows still receive every alert. Pass
  `ignoreFocus: true` from a fatal-class call site (auth-rejected,
  keychain-locked) to bypass the gate; nothing in 0.0.9 uses it yet
  but the option is reserved.
- **Unified notification coalescing.** Three different per-hook
  rate-limit policies (1-per-60s, 3-in-60s-then-summary, no
  coalescing) were folded into one shared token bucket on
  `dedupeKey` (default 3 dispatches per 60s window per key). Activity
  alerts, activity cards, and op-completion all consume it.
  Eviction sweeps run on every dispatch so single-shot keys
  (`op:<uuid>`) don't accumulate.
- **Error toasts stay until dismissed.** `kind: "error"` toasts are
  sticky by default — they carry diagnostic copy worth screenshotting
  or quoting into a bug report, and a 10 s auto-dismiss was the wrong
  default for that role. The close button + dedupeKey bound
  accumulation; transient errors can still pass an explicit
  `durationMs` to opt out of stickiness. Info toasts continue to
  auto-dismiss after 10 s.
- **OS-side notification grouping.** Each dispatch now passes a
  `group` value through to the OS — macOS reads it as `threadId` so
  related notifications stack into one expandable banner instead of
  five lookalikes. Hooks group by session (`session:<sid>`), full
  cwd (`project:<cwd>` — full path so two projects with the same
  basename don't collide), or op kind (`op:<kind>`). A
  `sound: "default"` is also forwarded so macOS plays the system
  chime. Linux libnotify ignores both fields gracefully.
- **Warning-severity banners announce as alerts.** The
  `StatusIssuesBanner` previously rendered warnings with `role="status"`
  — politely-announced to screen readers — while the visual styling
  matched errors. Severity above `info` now uses `role="alert"` so
  AT users hear the same urgency sighted users see.
- **Snoozed banners auto-clear when their condition resolves.** The
  24 h snooze used to persist even after the underlying drift / sync
  error went away, which masked re-occurrences against a stale timer.
  The shell now drops snooze entries when their issue id leaves the
  live set, including a first-mount reconciliation pass against the
  persisted store so a condition that resolves while the app is
  closed doesn't carry a silent snooze into the next launch.
- **`preferences_set_notifications` no longer blocks the IPC worker
  on disk fsync.** Mirrors the spawn_blocking pattern from
  `preferences_set_activity` — the std::sync mutex guard is dropped
  before the JSON write is handed to a blocking task. Rapid
  toggle-mashing no longer makes other prefs reads contend with the
  write.

### Removed

- **`notify_on_spend_usd` preference.** The pref was persisted, the
  Settings UI shipped an input for it, and the activity hook read it
  as a permission gate — but no detector ever fired a notification
  when a session crossed the threshold. The pricing module needed to
  expose per-session running spend before this could land
  honestly. Removed so users no longer set $5 and get nothing;
  serde-default keeps existing `preferences.json` files compatible.

### Fixed

- **`/api/oauth/usage` accepts the new `cowork` key shape.** CC 2.1.x
  renamed the team/cowork window's wire field from `seven_day_cowork`
  to a bare `cowork`. Both spellings now populate the same
  `seven_day_cowork` slot, so the Accounts/Keys usage views don't
  blank out for accounts whose budget is on the new shape. Older
  payloads keep working unchanged. The two adjacent fields that
  disappeared from CC's typed read in 2.1.123
  (`seven_day_oauth_apps`, `iguana_necktie`) stay in our type for
  now — removing them would silently drop live data if the server
  still emits them; the catch-all `unknown` HashMap covers the
  graceful-degradation path the day they're truly retired.
- **macOS Homebrew cask install.** The cask symlinks
  `/opt/homebrew/bin/claudepot` →
  `Claudepot.app/Contents/MacOS/claudepot-cli-<triple>`, but the v0.0.8
  build produced a `.app` that didn't actually contain the CLI. CI
  staged the binary at `src-tauri/binaries/` correctly, but
  `tauri.conf.json` never declared `bundle.externalBin`, so the
  Tauri bundler ignored the staged file. Adding the declaration
  makes `brew install --cask claudepot` install both the GUI and
  the CLI in one step, as the cask intends.
- **Linux / Windows bundles also include the CLI.** Same fix
  extended to .deb / .AppImage / .msi / .nsis.zip bundles. Adds ~5 MB
  per bundle; standalone CLI tarballs / zips still ship for
  CLI-only users.

## 0.0.9 — alpha (2026-04-29)

### Fixed

- **macOS Homebrew cask install.** The cask symlinks
  `/opt/homebrew/bin/claudepot` →
  `Claudepot.app/Contents/MacOS/claudepot-cli-<triple>`, but the v0.0.8
  build produced a `.app` that didn't actually contain the CLI. CI
  staged the binary at `src-tauri/binaries/` correctly, but
  `tauri.conf.json` never declared `bundle.externalBin`, so the
  Tauri bundler ignored the staged file. Adding the declaration
  makes `brew install --cask claudepot` install both the GUI and
  the CLI in one step, as the cask intends.
- **Linux / Windows bundles also include the CLI.** Same fix
  extended to .deb / .AppImage / .msi / .nsis.zip bundles. Adds ~5 MB
  per bundle; standalone CLI tarballs / zips still ship for
  CLI-only users.

## 0.0.8 — alpha (2026-04-29)

### Fixed

- **CI release pipeline.** v0.0.7's release run failed because the
  Linux/Windows GUI jobs looked for legacy Tauri 1 artifact names
  (`*.AppImage.tar.gz`, `*.nsis.zip`, `*.msi.zip`) when staging
  signatures. Tauri 2 signs each bundle file directly, so the actual
  outputs are `*.AppImage.sig`, `*-setup.exe.sig`, `*.msi.sig`. The
  staging steps and the `latest.json` generator now read those names,
  and the in-app updater for Linux/Windows points at the real
  installer URLs (`.AppImage`, `-setup.exe`).

## 0.0.7 — alpha (2026-04-28)

### Added

- **In-app auto-update.** Settings → About now checks for new
  signed releases, surfaces a Download / Skip / Restart card, and
  persists frequency preferences (every launch / daily / weekly /
  manual). Uses `tauri-plugin-updater` with a minisign-signed
  `latest.json` hosted as a GitHub release asset; signature
  verification is independent of OS code-signing. macOS, Linux
  AppImage, and Windows NSIS installs auto-update; Linux .deb,
  Windows MSI, and unconfigured-pubkey builds detect their
  unsupported state and hide the controls behind a "use the
  Releases page" hint.
- **⌃⌥⌘L** toggles developer mode globally. The visible toggle is
  gone from Settings → General; the four-modifier combo is
  unreachable by accident and matches macOS's deep-system-toggle
  convention. A toast confirms the new state.
- **Status-bar tooltips** on the live, projects, and sessions
  segments — the terse glyphy text now reveals plain English on
  hover, and screen readers get the same via `aria-label`.

### Changed

- **Settings → About redesigned.** App row renders the wordmark
  with `depot` in the accent color; author block carries two
  iconified links (GitHub mark + globe → homepage); design row
  trimmed to "paper-mono".
- **Developer mode** is no longer a Settings toggle — it's
  keyboard-only via ⌃⌥⌘L. The localStorage key (`cp-dev-mode`)
  and `<DevBadge>` consumers are unchanged.

## 0.0.6 — alpha

### Changed

- **Frontend perf overhaul.** Stabilized `useActions` /
  `useBusy` callback identities so AppStateProvider's context
  value stops churning on every render. Deferred cold-start
  `verify_all_accounts` past first paint. Replaced the 10 s
  preferences poll with an event-driven listener. Single-pass
  account match in `useStatusIssues`. Pinned the `useSection`
  ⌘1..⌘9 keydown listener via a section ref so it wires once
  for the lifetime of the hook.

### Fixed

- **Stale notification prefs after toggle.** `cp-prefs-changed`
  now carries the freshly-saved `Preferences` snapshot as its
  payload, eliminating the second-`preferencesGet()` ordering
  race that could let an older read overwrite a newer state.
- **Cross-process op discovery.** The running-ops poller no
  longer pauses when the local list goes empty — CLI-started
  ops surface in the GUI within one 3 s tick again.
- **Listener leaks under StrictMode double-mount.**
  `cp-activity-open-session`, `cp-tray-desktop-{clear,bind}`,
  the `useCardNotifications` bootstrap chain, and the
  `cp-prefs-changed` listener all use the active-flag
  late-resolve guard now.
- **Backend forwarder leak in card-notification cleanup.** Each
  frontend `unlisten` is now paired with
  `api.sessionLiveUnsubscribe(sid)` so the rust singleton
  releases its slot and remounts can re-subscribe without
  `AlreadySubscribed`.
- **Dead-state writes after fast unmount.** The deferred
  verify-all `requestIdleCallback` handle is stored on a ref
  and cancelled both on supersede and on hook teardown.

## 0.0.5 — alpha (unreleased)

### Added

- **Keyboard shortcuts modal** — `⌘/` opens a full reference grouped
  by scope (nav / global actions / modals / palette / live strip).
  Also reachable from the command palette via "Show keyboard
  shortcuts."
- **Adopt-current-session CTA** on the Accounts empty state — when
  CC is already signed in, clicking the primary button imports that
  account into Claudepot without a browser round-trip.
- **Account-to-tokens link** — each AccountCard shows a "N tokens"
  chip when the account owns stored API keys / OAuth tokens.
  Clicking jumps to Keys pre-filtered to that account.
- **Sessions section: Live filter + Trends tab** — the old Activity
  section is folded in. A "Live" chip filters the table to running
  sessions; a "Trends" tab shows bucketed active-session counts
  over 24h / 7d / 30d with an inline sparkline.
- **Trash dot + header button in Sessions** — the Cleanup tab
  renders a small accent dot when trash is non-empty, and the
  Sessions header grows a "Trash · N" button that jumps straight
  to the Cleanup tab.
- **"Updated Xm ago" label** next to the Accounts usage refresh.
- **"Send test notification" button** in Settings → Activity.
- **Sessions loading UX** — rows are cached in `sessionStorage` and
  painted immediately on mount; the header shows "Updating… Ns"
  while the cold fetch runs.
- **Per-account context-menu kebab** (`⋯`) on AccountCard, Projects
  rows, and Session rows — same items as right-click, reachable by
  keyboard users.

### Changed

- **Command palette hoisted to the shell.** `⌘K` no longer forces a
  navigation to Accounts; the palette, remove-confirm dialog, and
  shortcuts modal all mount at the AppShell level.
- **Sidebar collapses to 5 sections** (Accounts / Projects /
  Sessions / Keys / Settings). Activity is gone; `⌘4` is Keys,
  `⌘5` is Settings. The first-run Live-runtime consent modal still
  fires at shell level.
- **Cleanup surfaces consolidated.** Settings → Cleanup tab removed.
  GC (abandoned journals + snapshots) moved to Projects →
  Maintenance as a new `GcCard`. Rebuild-session-index moved to
  Sessions → Cleanup.
- **Projects filter chips** relabeled from "Source gone / Offline /
  Empty" to "Missing directory / Unreachable path / Empty project."
- **Refresh buttons** renamed per section ("Refresh projects",
  "Refresh sessions", "Refresh usage") so `⌘R` scope is obvious.
- **Desktop trust-tier copy** drops the "candidate-only / decrypt
  token" jargon in favor of "Couldn't confirm which account Claude
  Desktop is signed in as. Open Claude Desktop once, then try
  again."
- **OperationProgressModal** humanizes phase names (P3 → "Moving
  source directory", P6 → "Rewriting session transcripts", etc.).
  Raw ids remain visible in the row's title tooltip.
- **Design rule added** (`.claude/rules/design.md`): "Cards vs.
  tables — pick by primary verb (browse+act vs scan+drill), not
  count." Codifies current placements and guides future components.

### Fixed

- **`activity_hide_thinking` preference is now load-bearing.**
  SessionEventView renders thinking blocks as "Thinking · N chars
  — click to reveal" when the pref is on; Settings dispatches
  `cp-activity-prefs-changed` so open transcripts refresh without
  polling.
- **Inline reason on disabled `TargetButton`** — the CLI slot's
  disabled state now says "Session expired" / "Rejected — re-login"
  / "No credentials — re-login" under the button, honoring
  design.md's "disabled buttons state a reason inline" rule.
- **Sessions filter state persists across section hops** — a
  `sessionsFilterStore` (module-scope) keeps query / filter /
  repo / selected-path / tab / live-filter alive when the
  Sessions section unmounts and remounts.
- **Activity off-state** is no longer silent — `ActivitySection`
  (now in the Sessions "Live" filter path) renders an inline
  "Enable Activity" button when the runtime is off, instead of
  sending the user to Settings.

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
    status-issues strip; Bind items in the account context menu
    and command palette; Bind / Sign out / Launch / Reconcile in
    the tray. (Per-account Desktop sign-out is no longer in the
    context menu — use the tray's shared Sign out action
    instead.)
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

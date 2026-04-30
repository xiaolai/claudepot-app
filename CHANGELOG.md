# Changelog

Versioning scheme:

- `0.0.x` — alpha
- `0.1.x` — beta
- `1.0.0+` — stable

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

## 0.0.8 — alpha (unreleased)

### Fixed

- **CI release pipeline.** v0.0.7's release run failed because the
  Linux/Windows GUI jobs looked for legacy Tauri 1 artifact names
  (`*.AppImage.tar.gz`, `*.nsis.zip`, `*.msi.zip`) when staging
  signatures. Tauri 2 signs each bundle file directly, so the actual
  outputs are `*.AppImage.sig`, `*-setup.exe.sig`, `*.msi.sig`. The
  staging steps and the `latest.json` generator now read those names,
  and the in-app updater for Linux/Windows points at the real
  installer URLs (`.AppImage`, `-setup.exe`).

## 0.0.7 — alpha (unreleased)

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

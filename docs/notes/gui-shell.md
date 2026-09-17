# The Tauri shell — measurements and reversals

Working notes, moved out of `AGENTS.md` on 2026-09-17. `AGENTS.md` is
`@`-included into every session and states the rules in one line each;
this file carries what was measured in the renderer and the data directory, and which obvious next steps were tried and rejected.

Read it before changing anything it covers. The one-liners say *what*;
this says *why*, and why the obvious alternative is wrong — which is the
part that stops a decision being re-litigated or silently reverted.

## GUI (Tauri)

- `src-tauri/src/commands/` — async Tauri commands wrapping `claudepot-core`,
  sliced by domain (`mod.rs` + one file per surface). NO business logic.
- `src-tauri/src/dto.rs` — serde DTOs crossing to JS. Credentials never cross.
- `src/App.tsx` + `src/api/` (sliced by domain — `account`, `project`,
  `notification`, `activity`, etc., merged in `index.ts`) + `src/types/`
  (sliced by domain, merged in `index.ts`) — React UI, plain CSS.
- `AccountStore.db` is `Mutex<Connection>` so stores can cross `await` points in Tauri commands.

**Opening a transcript parses it once.** It used to parse it four
times: `read_session_detail_at_path` called `scan_session` (the row
fold) *and* `parse_events` (the event fold), and the viewer issued
`session_read_path` and `session_chunks` in a `Promise.all`, each of
which ran that same pair. The comment defending the double fetch said
"typical sessions are <1 MB"; measured on this machine, 375 transcripts
are over 1 MB, 100 over 10 MB, and the largest is **181 MB**. The two
folds now share one `serde_json::from_str` per line
(`scan_session_with_events`), and chunks — a pure function of the
events — ride back on `SessionDetailDto` instead of being their own
command. The *Tauri command* `session_chunks` was deleted rather than left unread — `claudepot_core::session_chunks` is still the module that builds them and the CLI still calls it, so a grep for the name finds nine files and none of them is a command.

`core_tests::scan_with_events_matches_the_two_separate_passes` pins the
one-pass fold against the two-pass one, over a fixture with a blank
line and a malformed line, because those are exactly where the two
loops differed.

**What is NOT worth doing here: windowing the transcript at the IPC
boundary.** It looks like the obvious next step and it is not — the
181 MB file yields **0.9 MB** of event text, because base64 image
payloads are dropped during parse (`tool_result` keeps only `text`
parts). The cost was never the payload; it was the parse. Measure
before adding a paging protocol the search box would then have to work
around.

**`project_list` caches only the nested half of its directory walk.**
The recursive size+mtime walk is 29,810 `stat` calls over 11 GB here —
1.1–1.3 s in release, paid on every mount of the Projects tab and every
⌘R — and ~90% of it is below the top level, in the per-session folders
CC writes beside each transcript. `src-tauri/src/project_size_cache.rs`
holds that share in memory (not persisted: it is a pure function of the
filesystem, so a file in the data dir would be a thing to migrate and
invalidate for a number rebuilt in under a second). The top level is
measured fresh every listing, so counts, transcript bytes and every
flag that gates behaviour are exact; what lags by at most one listing
is the nested contribution to one size column and a sort key.

Two details are load-bearing:

- **`is_empty` may not read a cached number.** It is the flag that can
  put a project in front of a delete button. It now tests
  `!has_subdirs` and the *top-level-only* byte sum, both from the
  shallow pass, so it answers identically with or without a cache. That
  is also strictly safer than the recursive test it replaces: a
  directory holding an empty subtree summed to under 4 KiB and read as
  "empty".
- **Parallelising instead was measured and is not enough** — 1.25 s →
  0.72 s, because the work is stat-bound and one slug dominates. The
  listing runs on rayon *as well*, but the cache is what makes a repeat
  listing cheap.
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

    **A refresh with nothing to apply must take no write lock.** Every
    read entry point (`list_all`, `list_by_slug`) refreshes first, and
    the steady state has an empty plan — so for most of this file's life
    a *read* of the index opened a write transaction anyway, because an
    unconditional `gc_events_older_than` sat inside it. `busy_timeout`
    is 5 s, so a reader could block for five seconds behind any other
    writer and then fail with "database is locked" while having nothing
    to write. Measured: a no-op refresh against a held `BEGIN IMMEDIATE`
    waited **5.30 s** and failed. The GC now runs on its own daily
    stamp (`meta.last_usage_gc_ms`), which is what allows the empty
    plan to return before the transaction. Note a DEFERRED transaction
    with no statements in it acquires nothing — so a test for this must
    reproduce the *write*, not just the transaction.

    **A per-project read refreshes only that project.** `list_by_slug`
    used the full refresh, stat-ing every transcript on the install
    (2,585 here) to answer a question about one directory. `refresh_slug`
    scopes **both** sides of the diff — `walk_fs_slug` and
    `load_db_tuples_for_slug`. Scoping only the walk would make every
    unwalked cached row look deleted and cascade the rest of the index
    away; there is a test on that in both directions.

    **Index-backed Tauri commands take the shared `SessionIndex`**, not
    their own. `SessionIndex::open` is not free and cannot be made free:
    `apply_schema` begins `IMMEDIATE` on **every** open, deliberately —
    it honours the `_pending_rescan` marker Settings → Cleanup writes,
    drops a redundant legacy index, and runs post-write validation, none
    of which are gated on the version changing. A "skip it when the
    version already matches" fast path was written, measured, and
    **reverted**: it silently ignored a user's rebuild request, and
    `test_schema_open_drops_redundant_turns_file_path_index` caught it.
    So the fix is not to make open cheap but to do it **once** — the GUI
    opens the index at startup and every command borrows that handle.
    A cold open (the CLI, or GUI startup) can still wait on the 5 s
    `busy_timeout` behind another writer; that wait is correct, and it
    is no longer on the interactive path.

    The one part of open that *was* safe to skip is the WAL-sidecar
    touch: it is a write whose only job is to create `-wal` / `-shm` so
    the chmod in `open()` can narrow them to 0600, and in the contended
    case they already exist (in WAL mode they live as long as any
    connection does). It now runs only when they are actually missing.
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
  nine below plus `agent-events.json`) move a corrupt file aside to a
  timestamped `<name>.corrupt.<unix-ts>` and start empty — never
  fatal at boot. Ten carry behavior worth documenting here:
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
  - `permission-grants.json` — active permission grants.
    `{schema_version: 2, grants: [{project_path, granted_at,
    expires_at}], legacy?: [...]}`, one grant per project_path. Owned
    by `claudepot-core::permission::store`. **The record is the
    capability**: `claudepot hook pre-tool-use` reads this file inside
    every Claude Code tool call while a grant is live and answers
    `allow` for sessions inside a granted project, so deleting a row
    ends the grant at the next call. The orchestrator drops lapsed
    grants each `usage_snapshot::run_tick` and keeps CC's hook entry in
    step. A schema-1 file (grants that wrote `bypassPermissions` into
    `.claude/settings.local.json`) is migrated on read, not moved
    aside: its records sit in `legacy` until the settings key has been
    put back. Empty file or no grants = feature off. See
    "## Permission grants".
  - `peer-inbound-grant.json` — the single open remote-control window.
    `{schema_version, grant: {...}|null}`. Owned by
    `claudepot-core::peer::inbound::store`. Records that Claudepot set
    CC's `crossSessionInbound` to `accept`, what the key held before
    (including "absent"), and when the window closes; the orchestrator
    reverts it each `usage_snapshot::run_tick`. **One grant, not a
    list** — the setting has a single machine-wide value, because CC
    only honors `accept` from user scope (a project-scope value can
    tighten the gate but never loosen it). That is also why the
    deadline is the whole feature: the blast radius cannot be narrowed
    spatially, so it is narrowed temporally. Like
    `permission-grants.json` this store **fails loud** on corruption —
    it is the only thing obliging anything to close the window. Empty
    file or no grant = feature off. See "## Peer messaging".
  - `remote-devices.json` — paired devices for the remote-control
    surface. `{schema_version, devices: [...], pending: {...}|null}`.
    Owned by `claudepot-core::remote::store`. Holds a SHA-256 of each
    device token and **never the token itself** — there is a test
    asserting the plaintext never reaches disk. **This is the
    revocation list**, which is why the store fails loud on corruption:
    a silent reset would not just lose the device list (that fails
    closed, safely — nothing authenticates until re-paired) but erase
    `revoked_at` for every device that was turned off — losing the
    record of what was let in and when it was turned off. (The refusal
    itself does not depend on the record: `authenticate` filters to
    `is_usable_at` and only then matches the hash, so an unknown token
    and a revoked one are the same answer. An earlier version of this
    note said the record was what kept a revoked token refused; that
    overstated it.) At most one `pending` pairing window: two live codes
    double the guessing surface for no benefit. Empty file = no paired
    devices.

    **Bounded on the way in, by `remote::prune`.** Every pairing appends
    a `Device`, and so does every password login — a session and a
    paired device are deliberately the same row — while revocation only
    *marks*. So the file grew monotonically and nothing ever removed
    anything. `DevicesFile::admit` is now the single append path and
    prunes before it pushes, so the arriving device can never be what a
    cap evicts. The policy, and the judgement in it: a **live** device
    is never pruned; a lapsed **session** goes 90 days after it expires
    (machine-issued and self-expiring, so nobody decided anything by
    letting it lapse); a **revoked** device is kept up to 200, newest
    first by `revoked_at` (revoking is a decision a human made about a
    specific device, and is worth more than a lapsed session). Both are
    equally refused by `authenticate` and are *not* equally interesting
    to a person reading the list later — that asymmetry is the whole
    policy. See "## Remote control".
  - `remote-config.json` — the remote surface's server settings and
    persisted auth state. `{schema_version, server: {enabled, bind,
    port}, password_hash, totp_secret_base32, totp_last_counter,
    failed_attempts, passkeys, passkey_user_handle}`. Owned by
    `claudepot-core::remote::config`. The passkeys are **public keys
    only** — the reason a passkey beats both of its neighbours here is
    that reading this file gives an attacker a cracking job for the
    password hash, working access for the TOTP secret, and nothing at
    all for these. They are account credentials, not device records, so
    they live here rather than on a `Device`: attaching one to a session
    would delete it when that session expired, and revoking a lost phone
    would destroy the way back in from every other one.
    **`enabled` defaults to false** — a remote surface that switches
    itself on because the app was installed is not a feature. Separate
    from `remote-devices.json` because the write rates differ by orders
    of magnitude: the throttle counter here moves on every failed
    login, the revocation list there moves when someone pairs or
    revokes, and sharing a file would rewrite the revocation list on
    every wrong password. **Fails loud on corruption** for a sharper
    reason than the other two: it holds the login throttle and the
    spent-TOTP high-water mark, so a silent reset hands an attacker
    unlimited guesses *and* reopens the replay window of every code
    that was burned to close it. Validation refuses a publicly-routable
    bind on the way to disk, not only at bind time. See
    "## Remote control".
  - `remote-read-state.json` — per-device read marks behind the panel's
    unread badges. `{schema_version, devices: {device_id: {sessions:
    {session_id: {through_count, at}}}}}`. Owned by
    `claudepot-core::remote::panel::read_state`. **Recovers silently on
    corruption**, unlike the two files above, and the asymmetry is the
    point: those hold a revocation list and a login throttle, where a
    silent reset hands something back to an attacker; this is a badge
    cache, and losing it clears every badge — exactly what tapping
    through the list would have done. The value is a **count of events
    consumed** — not a timestamp, because a phone's clock is not the
    machine's and comparing them would make a badge depend on clock
    skew; and not the *index* of the last event, because the two differ
    by one and the field was originally named for the index while every
    caller stored the count. Absent mark ≠ zero — a session this device
    never opened carries no badge at all, because a count against no
    baseline is just the event total and would put a four-digit number
    on every row of a new phone. Writes go through a process-local mutex:
    atomic rename is crash-safety, not concurrency-safety, and two marks
    landing together dropped each other (measured — there is a test that
    fails without the lock). Capped at 200 sessions per device and 32
    devices, oldest first. Empty file = no badges.
    See "## Remote control".
  - `quick-prompts.json` — the chips above the remote panel's
    composer, edited in Settings → Quick prompts. `{schema_version,
    prompts: [{id, name, text}]}`, owned by
    `claudepot-core::quick_prompt`. A short name you tap and the longer
    text it sends. **Absent and empty are different states**: no file
    means "never configured" and yields the built-in four, while a file
    that exists and is empty means "I deleted them all" and yields
    nothing — collapsing the two would make the last delete undo itself.
    Saved as a whole list because order is data; there is no add/remove
    verb. Recovers silently on corruption, unlike the two remote stores
    above: this is a list of phrases, and losing it costs retyping.
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
    `claudepot-core::cc_tips::history`. Append-only: deleting it loses the time mapping for past
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

**Launch-at-login is release-only, and a release launch re-registers it
every time.** `tauri-plugin-autostart` writes `current_exe()` verbatim
into `~/Library/LaunchAgents/Claudepot.plist`, so flipping the toggle
inside a `tauri dev` build installed `target/debug/claudepot-tauri` as
the login item; every login after that started a bare debug binary whose
`devUrl` had no Vite behind it — a blank window that looked like a
webview or proxy fault, and was blamed on one for a while (2026-08-26).
Two halves, both in `lib.rs`: the plugin is registered under
`#[cfg(not(debug_assertions))]` only, so a dev build has nothing to call
(`GeneralPane` hides the row via `AppStatus.dev_build`); and a release
build calls `enable()` again at setup whenever `is_enabled()`, which is a
plain plist rewrite — no `launchctl` — so a moved bundle or a stale dev
registration heals itself on the next launch instead of persisting until
someone toggles the switch twice.

**The status bar's pin is `window_always_on_top` in `preferences.json`,
and a launch re-applies it.** `set_always_on_top` is window state, not
a setting the window reads, so a fresh process starts at the normal
level whatever the file says; `setup()` re-applies it beside the
show-on-startup check. `preferences_set_window_always_on_top` changes
the level *before* it persists and puts both the level and the
in-memory field back if the save fails — file, window and button must
never disagree, because a pressed pin over a window that sinks behind
the next click is the one state the control exists to prevent. The
button sits at the bar's right end, just before the service-status
dot, which keeps the corner. It is the same kind of control as the
sidebar toggle at the far left — it acts on the window itself rather
than reporting state, so design.md's "a surface that only reports
state is not a control" does not apply to it — and it lives among the
ambient chips only because that is where its owner wanted it, not
because it is one. `useWindowPin` is the renderer half —
optimistic, reverting on a rejected call, and following
`cp-prefs-changed` like every other reader of that file.

**A packaged build has no reload affordance, and the guard is in the
renderer because the ErrorBoundary's Reload must keep working.**
`useWebviewChromeGuard` cancels the keys a webview reads as reload — F5
and its Ctrl/Shift hard-reload variants, ⌘R / ⌃R, ⌘⇧R / ⌃⇧R — and
suppresses the native context menu, which is where Reload lives on all
three platforms, for any target that is not an editable field or the
element holding a live selection (Copy / Paste / Look Up are what that
menu is for in an app). Off in dev, where reload and Inspect Element are
the loop.

A reload here is a browser artifact: no address bar, no tab to restore,
and nothing on screen saying the window is disposable — while it
discards every piece of state that lives only in the renderer, an open
modal and a half-typed secret included. But `location.reload()` is
deliberately untouched: guarding the *input* is what leaves the
ErrorBoundary's recovery button working, and it is also why this is not
Tauri's navigation handler, which sees a reload and cannot tell a
keypress from the app's own decision.

Four details are load-bearing:

- **It cancels and never stops propagation**, so ⌘R still reaches the
  section handler that refreshes the list. Only Accounts and Projects
  pass one — on the other eight sections the documented ⌘R does nothing,
  which is how the key was reaching the webview at all.
- **It is deliberately not behind `isShortcutContextBlocked()`.** A
  suppression is not a shortcut: a focused field is where a reload costs
  the most, and cancelling a keystroke the webview would have eaten
  takes nothing from the person typing.
- **Capture phase**, so a modal or palette input that calls
  `stopPropagation` on its own keydown cannot hide the key from it. The
  context-menu half is the opposite — bubble phase on `document`, after
  React's root handlers, so every app context menu (account card,
  project row, session row) keeps the `preventDefault` it already does.
- **The exposure differs per platform, and only part of it is closed
  from here.** On macOS ⌘R appears never to have reached the webview
  at all: wry's `performKeyEquivalent` hands the key to the app menu,
  and `app_menu.rs` binds no accelerator on View items on purpose —
  read from that key path rather than measured, so treat the context
  menu as the Mac story and the keys as belt-and-braces. On WebView2 F5 / Ctrl+R /
  Ctrl+Shift+R are *browser accelerator keys*, on by default; wry can
  turn them off (`with_browser_accelerator_keys`) and Tauri 2.11 does
  not expose it, so the keydown is the only layer the renderer has —
  and whether Chromium lets a page cancel those is **not verified**.
  WebView2's own context menu is likewise still native. Closing either
  properly means `with_webview` plus `ICoreWebView2Settings3`, which no
  machine here can behaviourally test.

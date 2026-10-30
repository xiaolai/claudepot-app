# The data directory — what lives in `~/.claudepot/`

Working notes, moved out of `AGENTS.md` on 2026-10-02 and merged with
the per-store reasoning that had been sitting in `gui-shell.md`.
`AGENTS.md` keeps the cross-cutting rules in one line each; this file is
the inventory, each file's contract, and the argument behind it.

Read it before adding a store, changing what one holds, or changing how
one recovers from corruption.

## The inventory is gated

Override the root with `CLAUDEPOT_DATA_DIR`. The authoritative list is
whatever joins onto `claudepot_core::paths::claudepot_data_dir()`, and
`cargo xtask verify-docs` fails when a `*.db`, `*.json` or `*.jsonl`
name in the source is missing from **this file** — it read `AGENTS.md`
until the inventory moved here. **It checks nothing else**: the `.pem`,
`.lock` and directory rows below are held by this table alone, which is
why they were absent until 2026-09-17.

## Databases

**Every database opens through
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

Eight SQLite databases:

| File | Owner | Contract |
|---|---|---|
| `accounts.db` | `cli_backend` | authoritative account + verification state, linked to Keychain |
| `boards.db` | `board::store` | **user data, not a cache** — a board's contents exist nowhere else once the writing session ends, so migrations preserve rows and nothing prunes automatically. Opened directly by GUI, CLI and the MCP subprocess with no IPC between them, so `writer_id` is self-reported: every surface renders provenance as "Reported by …", never as verified identity |
| `sessions.db` | `session_index` | one row per `.jsonl` transcript, keyed by file_path; `(size, mtime_ns)` is the re-parse guard. Rebuild via Settings → Cleanup or `claudepot session rebuild-index`. Four invariants: a refresh with an empty plan must take **no** write lock; a per-project read must scope **both** sides of the diff; index-backed Tauri commands borrow the **shared** `SessionIndex` rather than opening their own (`shared_or_open`); a re-scan of a changed transcript writes **only the rows that differ** — delete-and-reinsert of a live 263 MB transcript every two minutes was 34 GB in 20 hours (`docs/notes/gui-shell.md`) |
| `env-vault.db` | `env_vault::store` | the local named-secret vault (`env_secrets`, secret in a 0600 column, no OS Keychain) |
| `keys.db` | `keys::store` | the Keys tab's API-key inventory, same at-rest pattern |
| `memory_changes.db` | `memory_log` | append-only log of detected CLAUDE.md / memory-file writes |
| `activity_metrics.db` | `session_live::metrics_store` | one row per session per tick for Activity Trends |
| `corpus.db` | `corpus` | the analysis corpus: every transcript from every machine, deduped, built by `claudepot corpus index`. **Deliberately not in `sessions.db`**, whose `refresh` deletes every row it cannot see under one `config_dir` — correct for a cache, fatal for an archive. Derived data: safe to delete, one ~5-minute pass to rebuild. Outputs do *not* live here; distilled claims go to `memories` in `sessions.db`, which carries no foreign key to `sessions` and is never cascaded |

## JSON and JSONL state

**A corrupt file is never fatal at boot**: every
`json_store`-backed store moves it aside to
`<name>.corrupt.<unix-ts>` and starts empty. What differs is whether
anyone is told — `load_or_recover` hands the caller a
`CorruptionRecovery` marker to surface, and `load` / `load_or_default`
discard it. The four that surface it are marked **reports** below, and
they are the four whose silent reset would hand something back to an
attacker or leave a door open with nothing obliging it to close.

| File | Owner | Contract |
|---|---|---|
| `agents.json` | `agent` | the v2 agents store (scheduled headless `claude -p` runs) |
| `automations.json` | `agent` | **legacy v1**, read only by the v1 → v2 migration in `AgentStore::open_at`, never written |
| `agent-events.json` | `agent::events::store` | capped log of agent run events |
| `notifications.json` | `notification_log` | ≤ 500 dispatched toast + OS-banner entries behind the bell popover. Capture sites: `pushToast` in `src/hooks/useToasts.ts`, `dispatchOsNotification` in `src/lib/notify.ts` |
| `preferences.json` | `preferences` | UI preferences, including `window_always_on_top` and the `Option<String>` locale where `None` means "follow the OS" |
| `routes.json` / `routing-rules.json` | `routes` | third-party provider definitions and their routing rules |
| `updates.json` | `updates` | update channel + skipped-version state |
| `usage-snapshot.json` | `usage_snapshot` | the last usage fetch. **The panel renders this file, never a live call**, so something has to write it — `claudepot usage refresh` on a machine with no GUI, and an *older* desktop build silently drops fields it was compiled without |
| `usage_alert_state.json` | `usage_alert` | per-window alert de-duplication |
| `rotation-rules.json` | `rotation::store` | user-authored auto-rotation rules, hand-edit-friendly. Empty or no rules = feature off |
| `rotation-audit.json` | `rotation::audit` | ≤ 500 rotation outcomes with rule_id, from/to and reason |
| `rotation-breaker.json` | `rotation::breaker_store` | per-rule consecutive-failure ledgers. 3 failures running quarantines a rule until a 6-hour cooldown probe |
| `permission-grants.json` | `permission::store` | **reports.** One grant per project_path; **the record is the capability** — `claudepot hook pre-tool-use` reads it inside every CC tool call, so deleting a row ends the grant at the next call. Schema-1 rows are migrated on read, not moved aside |
| `peer-inbound-grant.json` | `peer::inbound::store` | **reports.** One grant, not a list: CC's `crossSessionInbound` has a single machine-wide value, so the blast radius is narrowed temporally instead of spatially. This store is the only thing obliging anything to close that window |
| `remote-devices.json` | `remote::store` | **reports.** SHA-256 of each device token, never the token (there is a test). **The revocation list** — a silent reset would erase `revoked_at` for every device that was turned off. At most one `pending` pairing. Bounded on the way in by `remote::prune` through the single `admit` path |
| `remote-config.json` | `remote::config` | **reports.** Server settings plus `password_hash`, `totp_secret_base32`, `totp_last_counter`, `failed_attempts` and **public-key-only** passkeys. `enabled` defaults to false. A silent reset would hand an attacker unlimited guesses *and* reopen every burned TOTP code's replay window |
| `remote-read-state.json` | `remote::panel::read_state` | per-device unread marks, as a **count of events consumed** — not a timestamp (clock skew) and not an index (off by one). Absent ≠ zero. Writes take a process-local mutex; atomic rename is crash-safety, not concurrency-safety |
| `relay-devices.json` | `relay::store` | **reports.** Relay devices — a separate list from `remote-devices.json`, so neither surface's token reaches the other. SHA-256 of each `cprl_` token, never the token. Each grant is `(upstream, accepted_generation, epoch)`. **The revocation list**, for the same reason as remote's. A save that fails changes nothing in memory |
| `relay-config.json` | `relay::store` | **reports.** `server_id`, the config `epoch`, bind, port, leaf names and the publications with their target digests and generations; each publication names an `account_id` or a `route_id` (into `routes.json`), never both. A reset mints a new `epoch`, so every grant recorded under the old numbering reads as `acceptance_required` instead of matching a reissued generation |
| `relay-usage.jsonl` | `relay::ledger` | append-only, 0600, one line per relayed request (`schema` 1): device id, upstream, target digest, generation, operation, requested and answering model, outcome, status, TTFB, byte count and token usage. **No prompt, response, header or token ever.** Cost is priced only for subscription upstreams and always marked approximate. Each line is one `write_all` under a process mutex, and a torn last line is skipped on read. Not regenerable, and nothing prunes it |
| `relay-servers.json` | `relay::servers` | **reports.** The *client* side: relay servers this machine has paired with, in pinned order — `server_id`, name, base URL, the pinned CA PEM and its fingerprint, `device_id`, the default upstream and, per upstream, the target fingerprint and generation this client approved. Plus the per-client `client_salt` that blinds account fingerprints. **The device token is not here** — it is in the OS credential store (`relay::token_store`, service `claudepot-relay-device`, keyed by `server_id`), so a reset orphans tokens the server must revoke, and the reset log says so |
| `quick-prompts.json` | `quick_prompt` | the panel composer's chips. **Absent and empty are different states**: no file yields the built-in four, an empty file yields nothing |
| `pricing-history.json` | `pricing::history` | append-only record of observed model-rate changes; not regenerable |
| `pricing-cache.json` | `pricing` | pure cache of the live pricing scrape; safe to delete |
| `migrate-peers.json` | `migrate::peer` | per-`(peer, project)` fingerprints for delta export. **Transport state, not cache** — it must survive a `sessions.db` rebuild, or every file re-sends to every peer. `(size, mtime_ns)`, not a watermark, because `session slim` rewrites transcripts smaller in place |
| `cc_tips_snapshots.jsonl` | `cc_tips::history` | append-only: converts CC's counter-only tips state into wall-clock time. Deleting it loses a mapping that cannot be reconstructed |
| `cc_tips_catalog.json` | `cc_tips::catalog` | cache of the tips catalog extracted from the CC binary. Resolved through `paths::claudepot_data_dir()`, never a hand-built `$HOME/.claudepot` |
| `doctor-parse-failures.jsonl` | `cc_doctor::parse_failures` | append-only log of inputs `cc doctor` could not parse; safe to delete |

## Everything else

Everything else in the directory, none of which `verify-docs` can see:

| Path | Owner | Notes |
|---|---|---|
| `remote-cert.pem`, `remote-key.pem` | `remote::tls` (`CERT_FILENAME` / `KEY_FILENAME`) | the TLS leaf the remote server binds with. **`remote-key.pem` is a private key living in the data dir** — do not sync, back up or attach this directory anywhere without knowing that |
| `remote-ca-key.pem`, `remote-ca.crt`, `remote-ca.srl` | `scripts/mint-remote-cert.sh` | the private CA that signs the leaf, and its serial. The CA key signs anything your devices trust |
| `relay-ca.pem`, `relay-ca-key.pem`, `relay-cert.pem`, `relay-key.pem` | `relay::tls` (`CA_CERT_FILENAME` / `CA_KEY_FILENAME` / `CERT_FILENAME` / `KEY_FILENAME`) | the relay's own CA and leaf — an independent root from remote control's, so rotating either leaves the other byte-equal (there is a test). **`relay-ca-key.pem` and `relay-key.pem` are private keys (0600) living in the data dir**; the CA key signs anything paired relay clients trust. A missing or mismatched CA key fails closed when the leaf needs renewing; it is never re-minted silently |
| `relay-profiles/u-<upstream>/` | `relay::launch` (`PROFILES_DIR`) | the `CLAUDE_CONFIG_DIR` that `claudepot relay run` gives Claude Code, one per upstream (0700; the `u-` prefix keeps an id such as `con` from naming a reserved Windows device), so a relayed session never reads or writes the local `~/.claude` login or settings. CC owns what is inside; it holds transcripts and settings like any CC config dir |
| `relay-codex/` | `relay::translate::codex` (`default_home`) | the `CODEX_HOME` only the relay uses, for Codex translated routes (0700), signed in once with `mkdir -p -m 700 ~/.claudepot/relay-codex && CODEX_HOME=~/.claudepot/relay-codex codex login` (codex refuses a `CODEX_HOME` that does not exist yet). **Codex owns what is inside, including `auth.json` with the ChatGPT refresh token** — the relay only asks Codex's `app-server` for an access token, so no second owner ever spends that refresh token |
| `.swap.lock` | `cli_backend::swap` | cross-process lock for a CLI slot swap |
| `desktop.lock` | `desktop_lock` | cross-process lock for Desktop profile writes |
| `agents.json.lock` | `agent::store` | held for the whole open→mutate→save of `agents.json` |
| `relay-serve.lock` | `relay::service` (`SERVE_LOCK_FILENAME`) | held for as long as a relay serves, by whichever process serves — CLI or GUI. Its presence is what `serving` means, and while it is held every offline mutation (publish, bind, unpublish, offline revoke) is refused, because the serving process read those files at start and would overwrite them. `relay revoke` from another process goes through the serving relay's revoke socket instead |
| `relay-revoke/` | `relay::revoke_socket` (`DIR_NAME`) | 0700, holding only `socket`, and only while a relay serves (Unix). The serving relay's revoke socket: one verb, revoke, answered only to a peer running as the same user. Checked on every start and refused — never chmodded — unless it is a directory of this user's with no group or other bits. A socket left by a killed relay is replaced by the next serve, which holds the lock |
| `agents/<agent_id>/runs/` | `agent::install`, scanned by `agent::liveness` | per-run trees (`result.json` + logs) |
| `approvals/` | `remote::approval` | one file per request and one per decision — **one writer each**, because these two writers are in different processes and a mutex buys nothing there |
| `bin/`, `bin/.helpers` | `routes::path_setup`, `routes::helper` | route wrapper binaries materialized on PATH for third-party providers. The agent shim puts this directory on its own PATH, so it is not only the Providers tab's |
| `credentials/` | `cli_backend::storage` | per-account credential material handled through the verified-write Keychain pattern |
| `desktop/<account_id>/` | `paths::desktop_profile_dir` | per-account Claude Desktop profile snapshots |
| `imports/<bundle_id>/staging/` | `migrate::apply` | in-flight import staging |
| `repair/` | `paths` | rename journals, per-project locks, pre-rename / pre-clean snapshots |
| `trash/` | `trash` (under `trash/sessions`) and `artifact_lifecycle` | Settings → Cleanup's recoverable deletions — two owners, two subtrees |

Stores also leave timestamped siblings behind on purpose —
`<name>.corrupt.<unix-ts>` from the recovery above, and the migration
backups (`automations.json.pre-v2-backup`, `agents.json.bak.<stamp>`).
A file matching one of those shapes is history, not state.

## Per-store detail

The tables above are the contract. What follows is the longer account of
the stores whose contract needed defending. Where it says a store
**fails loud**, that is the `reports` behaviour in the table: the file is
moved aside and the reset is surfaced to the caller — never fatal at
boot, never silent.

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
  pattern — no OS Keychain. See `core-modules.md`.
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
  put back. Empty file or no grants = feature off. See `cc-integration.md`.
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
  file or no grant = feature off. See `cc-integration.md`.
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
  policy. See `remote-control.md`.
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
  bind on the way to disk, not only at bind time. See `remote-control.md`.
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
  See `remote-control.md`.
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
  observed, and the bundled rate history stands alone. See `core-modules.md`.
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
  rate *changes* and is not regenerable — see `core-modules.md`.
- `cc_tips_catalog.json` — cache of the tips catalog extracted from
  the CC binary. Owned by `claudepot-core::cc_tips::catalog`. Pure
  cache: safe to delete, rebuilt on next extraction. Resolved
  through `paths::claudepot_data_dir()` rather than a hand-built
  `$HOME/.claudepot` — the hardcoded form bypassed both the
  `CLAUDEPOT_DATA_DIR` override and the test-isolation guard, which
  let a test write into the developer's live data root.

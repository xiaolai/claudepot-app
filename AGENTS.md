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

## How this file is split

This file states the rules. `docs/notes/` carries the reasoning behind
them — what was measured, which obvious alternative was tried and
reverted, and which shipped bug each gate was written after. The split
exists because this file is `@`-included into **every** session and the
notes are not; it is not a signal that the notes are optional.

| Note | Covers |
|---|---|
| [`docs/notes/remote-control.md`](docs/notes/remote-control.md) | the appliance security model, the certificate work, every design decision inside the phone panel |
| [`docs/notes/gui-shell.md`](docs/notes/gui-shell.md) | the renderer and data-dir measurements, and three optimisations that were reverted |
| [`docs/notes/test-gates.md`](docs/notes/test-gates.md) | the failure each gate was written after |
| [`docs/notes/cc-integration.md`](docs/notes/cc-integration.md) | retention, permission grants and peer messaging, against dated CC versions |
| [`docs/notes/i18n.md`](docs/notes/i18n.md) | the three catalogs and the bugs behind each rule |
| [`docs/notes/assets-and-release.md`](docs/notes/assets-and-release.md) | icons, screenshots, release validation |

**A rule in this file whose reasoning is in a note must not be changed
from this file alone** — the note is where the argument against the
obvious alternative lives, which is the part that stops a decision
being silently reverted.

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
scripts/build-panel.sh               # Remote panel → committed embed dir
```

`scripts/build-panel.sh` is separate because `panel/` has its own
install and its output is **committed** — see "## Remote control". A
source change under `panel/` that nobody rebuilt ships the previous
bundle with no error anywhere.

## Test

```bash
cargo test --workspace               # Rust
cargo xtask verify-cc-parity         # CC settings-merge parity goldens (parity-harness/README.md)
cargo xtask verify-docs              # README / AGENTS.md / event-channel / screenshot contracts
pnpm test                            # React (Vitest + RTL, jsdom)
pnpm test:coverage                   # React with coverage report
cd panel && pnpm check:render        # the built remote panel actually mounts
pnpm check:classes                   # every className has a rule; every text field draws chrome
pnpm check:a11y                      # every role="switch" has an accessible name
pnpm check:motion                    # the reduced-motion override reaches the primitives
pnpm check:contrast                  # the prefers-contrast override is not lost to source order
pnpm check:catalogs                  # en↔zh key / placeholder / <Trans> tag parity
pnpm check:envvar-layout             # needs a screen; CI runs its unit half only
```

Every gate here was written after a specific failure had already
shipped green. What each one catches, and the three or four details
that make each one honest rather than decorative, is in
[`docs/notes/test-gates.md`](docs/notes/test-gates.md) — read it before
changing a gate, and especially before relaxing one.

| Gate | The question it answers that nothing else does |
|---|---|
| `check:render` | does the **committed** panel bundle *mount*? Seven passes: signed-out and offline, signed-in thread, the quick-prompt sheet, the slash-command sheet, staging a command, the offline queue as a round trip, the 1200px two-pane layout. `vite build` cannot answer it — it does not resolve free identifiers, so a missing import is a runtime `ReferenceError` in whatever path touches it |
| `check:classes` | does every `className` have a CSS rule behind it, and does every bare `input` / `textarea` draw chrome? Both are valid HTML and invisible to `tsc`. Refuses a vacuous pass under 100 defined / 100 used; `lucide*` is exempt |
| `check:a11y` | does every `role="switch"` have an accessible name? Requires an aria attribute outright — the visible text beside a switch is not a label |
| `check:motion` / `check:contrast` | do the `prefers-reduced-motion` and `prefers-contrast: more` overrides actually *reach* the primitives, which animate from inline styles? Both turn on source order inside `tokens.css` |
| `check:catalogs` | en↔zh parity. "Orphan" is **cross-locale only** — it cannot see a key that no source file references, and a green run is not evidence there are none |
| `check:envvar-layout` | does the env-var pane lay out at all? Drives the real app over the debug-only MCP bridge, so CI runs the pure `evaluate()` half instead |

Five carry a `:self-test` that forces the assertions to fail
(`check:classes`, `check:a11y`, `check:motion`, `check:contrast`, and
the panel's `check:render`), `check-envvar-layout.mjs` takes
`--self-test`, and `check:catalogs` is exercised by pointing
`CLAUDEPOT_LOCALES_DIR` at a fixture. A check nobody has watched fail
is indistinguishable from one that cannot fail.

CI runs the core + cli tests on a Linux/macOS/Windows matrix and the
`claudepot-tauri` crate's tests on macOS + Windows (Linux needs
webkit2gtk; release.yml's Linux build job is that crate's Linux
compile gate). The lint job fmt/clippy-gates `xtask` itself and runs
`cargo xtask verify-cc-parity`. Release builds preflight a five-site
version lock-step check (tag vs `Cargo.toml`, `package.json`,
`tauri.conf.json`, README status banner, web install-page banner).

**`scripts/build-panel.sh` is not in this list and nothing notices if
you skip it.** A source change under `panel/` that nobody rebuilt ships
the previous committed bundle with no error anywhere — see
"## Remote control".

## GUI (Tauri)

- `src-tauri/src/commands/` — async Tauri commands wrapping `claudepot-core`,
  sliced by domain (`mod.rs` + one file per surface). NO business logic.
- `src-tauri/src/dto.rs` — serde DTOs crossing to JS. Credentials never cross.
- `src/App.tsx` + `src/api/` (sliced by domain — `account`, `project`,
  `notification`, `activity`, etc., merged in `index.ts`) + `src/types/`
  (sliced by domain, merged in `index.ts`) — React UI, plain CSS.
- `AccountStore.db` is `Mutex<Connection>` so stores can cross `await` points in Tauri commands.

What was measured behind this shell — the transcript that was parsed
four times, the 29,810-`stat` directory walk and the cache that absorbs
90% of it, the session index that took a write lock to answer a read,
launch-at-login, the status-bar pin, and the release-only page-reload
guard — is in [`docs/notes/gui-shell.md`](docs/notes/gui-shell.md),
together with the three optimisations that were written, measured and
**reverted**. Read it before optimising anything here; two of those
three look obviously right.

### What lives in `~/.claudepot/`

Override the root with `CLAUDEPOT_DATA_DIR`. The authoritative list is
whatever joins onto `claudepot_core::paths::claudepot_data_dir()`, and
`cargo xtask verify-docs` fails when a `*.db` or `*.json` name in the
source is missing from this file. **It checks nothing else** — the
`.pem`, `.jsonl`, `.lock` and directory rows below are held by this
table alone, which is why they were absent from it until 2026-09-17.

**Every database opens through
`claudepot-core::db_pragmas::apply_standard_pragmas`**, and
`verify-docs` fails a `Connection::open` that doesn't. Hand-rolling the
pragma batch is the failure, not getting it wrong: `corpus.rs` rolled
one that *looked* deliberate and silently omitted `journal_size_limit`
+ `wal_autocheckpoint`, leaving the largest database in the app outside
the bound that exists because `sessions.db-wal` once reached 6.3 GB.
Per-store extras (`synchronous=NORMAL`, `foreign_keys=ON`) go in a
second batch *after* the helper, never instead of it. The helper also
retries the `delete` → `wal` transition, which SQLite does not run the
busy handler for.

Eight SQLite databases:

| File | Owner | Contract |
|---|---|---|
| `accounts.db` | `cli_backend` | authoritative account + verification state, linked to Keychain |
| `boards.db` | `board::store` | **user data, not a cache** — a board's contents exist nowhere else once the writing session ends, so migrations preserve rows and nothing prunes automatically. Opened directly by GUI, CLI and the MCP subprocess with no IPC between them, so `writer_id` is self-reported: every surface renders provenance as "Reported by …", never as verified identity |
| `sessions.db` | `session_index` | one row per `.jsonl` transcript, keyed by file_path; `(size, mtime_ns)` is the re-parse guard. Rebuild via Settings → Cleanup or `claudepot session rebuild-index`. Three invariants: a refresh with an empty plan must take **no** write lock; a per-project read must scope **both** sides of the diff; index-backed Tauri commands borrow the **shared** `SessionIndex` rather than opening their own |
| `env-vault.db` | `env_vault::store` | the local named-secret vault (`env_secrets`, secret in a 0600 column, no OS Keychain) |
| `keys.db` | `keys::store` | the Keys tab's API-key inventory, same at-rest pattern |
| `memory_changes.db` | `memory_log` | append-only log of detected CLAUDE.md / memory-file writes |
| `activity_metrics.db` | `session_live::metrics_store` | one row per session per tick for Activity Trends |
| `corpus.db` | `corpus` | the analysis corpus: every transcript from every machine, deduped, built by `claudepot corpus index`. **Deliberately not in `sessions.db`**, whose `refresh` deletes every row it cannot see under one `config_dir` — correct for a cache, fatal for an archive. Derived data: safe to delete, one ~5-minute pass to rebuild. Outputs do *not* live here; distilled claims go to `memories` in `sessions.db`, which carries no foreign key to `sessions` and is never cascaded |

JSON and JSONL state. **A corrupt file is never fatal at boot**: every
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
| `quick-prompts.json` | `quick_prompt` | the panel composer's chips. **Absent and empty are different states**: no file yields the built-in four, an empty file yields nothing |
| `pricing-history.json` | `pricing::history` | append-only record of observed model-rate changes; not regenerable |
| `pricing-cache.json` | `pricing` | pure cache of the live pricing scrape; safe to delete |
| `migrate-peers.json` | `migrate::peer` | per-`(peer, project)` fingerprints for delta export. **Transport state, not cache** — it must survive a `sessions.db` rebuild, or every file re-sends to every peer. `(size, mtime_ns)`, not a watermark, because `session slim` rewrites transcripts smaller in place |
| `cc_tips_snapshots.jsonl` | `cc_tips::history` | append-only: converts CC's counter-only tips state into wall-clock time. Deleting it loses a mapping that cannot be reconstructed |
| `cc_tips_catalog.json` | `cc_tips::catalog` | cache of the tips catalog extracted from the CC binary. Resolved through `paths::claudepot_data_dir()`, never a hand-built `$HOME/.claudepot` |
| `doctor-parse-failures.jsonl` | `cc_doctor::parse_failures` | append-only log of inputs `cc doctor` could not parse; safe to delete |

Everything else in the directory, none of which `verify-docs` can see:

| Path | Owner | Notes |
|---|---|---|
| `remote-cert.pem`, `remote-key.pem` | `remote::tls` (`CERT_FILENAME` / `KEY_FILENAME`) | the TLS leaf the remote server binds with. **`remote-key.pem` is a private key living in the data dir** — do not sync, back up or attach this directory anywhere without knowing that |
| `remote-ca-key.pem`, `remote-ca.crt`, `remote-ca.srl` | `scripts/mint-remote-cert.sh` | the private CA that signs the leaf, and its serial. The CA key signs anything your devices trust |
| `.swap.lock` | `cli_backend::swap` | cross-process lock for a CLI slot swap |
| `desktop.lock` | `desktop_lock` | cross-process lock for Desktop profile writes |
| `agents.json.lock` | `agent::store` | held for the whole open→mutate→save of `agents.json` |
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

Optional feature: for a time-boxed window (or until revoked), Claude
Code's tool calls in one project run without permission prompts. **The
grant is a record, never a mode** — visible, with a countdown and a
Revoke button, and it lapses on its own.

It used to write `bypassPermissions` into
`.claude/settings.local.json`, and **CC stopped honouring that in
2.1.257** (*"projectSettings and localSettings are repo-controllable"*,
re-verified against 2.1.274). Every grant written before that was a
silent no-op while the pane said "Bypass active".
`permission::settings::PROJECT_SCOPE_IGNORES_SINCE` is the pin, and the
resolver reports such a value as
`PermissionDecisionSource::ProjectScopeIgnored` — rendered as *ignored*
with one-click removal, never as elevated.

**A grant is now CC's `PreToolUse` hook answering `allow`**
(`claudepot hook pre-tool-use`, hidden — CC invokes it). `PermissionRequest`
was measured and rejected: in **auto** mode — the built-in starting
mode on Pro, Max and Team — it never fires, because the classifier has
already decided. `PreToolUse` runs before the permission system decides
anything.

Five properties hold it together; the measurement table and the
adversarial review that produced them are in the note.

- **Two hooks, two lifetimes, one installer.** `cc_hook_entry` writes
  and removes a verb-matched exec-form entry through `settings_mutex`;
  the approval entry leaves with `remote serve`, the grant entry with
  the last live grant, and one's uninstall cannot take the other.
- **The hook never touches the file.** `hook::load_readonly` parses and
  nothing else — no recovery, no rename-aside, no log line. A corrupt
  file reads as "no grant" and the GUI's next tick is what moves it
  aside.
- **Scope is the session's working directory** — the same scope
  `bypassPermissions` had. `/p/a-evil` is not under `/p/a`; a symlinked
  root is matched on a second canonicalized pass. Deny and ask rules
  still apply.
- **A grant with no hook is an error, not a grant.** `permission_grant`
  rolls the record back if the entry cannot be written, and
  `hook_installed` on the DTO makes a missing entry render as a warning
  rather than as active.
- **The entry points at the CLI, never at `current_exe()` blindly.**
  From the GUI `current_exe()` is the Tauri app, which has no `hook`
  verb — an entry aimed at it would launch the desktop app on every
  tool call and block the call at the timeout. Both installers go
  through `cc_hook_entry::hook_binary`.

Cost is one spawn per tool call, ~13 ms here; there is deliberately no
`matcher`, since a fixed list of "tools that can prompt" would drift
with CC. Orchestrator at `src-tauri/src/permission_orchestrator.rs`
drops lapsed grants, migrates schema-1 records and reconciles the hook
entry every five minutes — which is also what re-points it after a
Homebrew upgrade or an app move.

## Peer messaging (`claudepot session live` / `send` / `inbound`)

Addressing a **running** Claude Code session over the Unix socket CC
binds per session (`messagingSocketPath` in
`~/.claude/sessions/<pid>.json`, newline-delimited JSON, a 0600 key
file holding `peerToken`). Pure logic in `claudepot-core::peer`
(`wire` / `key` / `client` / `discover` / `outcome` / `inbound`); CLI
verbs in `cli/commands/session/send.rs`. `peerProtocol == 1` is a hard
pin — this is an internal, feature-gated surface on a product shipping
~27 releases a month, so a session announcing anything else is refused
rather than addressed on a guess.

Five properties drive the design; changing any invalidates the feature.
The evidence for each, and why `permission_response` in the binary is
not a way in, is in
[`docs/notes/cc-integration.md`](docs/notes/cc-integration.md).

- **It can inject a prompt and nothing else.** No exit, interrupt or
  restart action exists, and `TIOCSTI` is refused by current macOS with
  EACCES even on a pty the caller owns. **A UI must not offer
  "restart" over this channel.**
- **Arrival is not delivery.** `crossSessionInbound` is
  `accept | hold | refuse`, and an unattested sender addressing a
  `bypassPermissions` session gets **held**. The success type is
  `Handoff`, not `Delivery`, and the CLI never prints "sent".
- **A peer prompt is not keyboard input.** CC wraps the text and
  attaches a standing caveat telling the session a peer cannot grant
  escalation — it calls trying to do so *permission laundering*. Asking
  a session to approve its own pending prompt is expected to be
  refused, and that refusal is correct.
- **Slash commands do not work.** CC's peer inbox dispatches with
  `skipSlashCommands: true`, so `/compact` arrives as literal text.
  Never present the input as a command line.
- **`accept` only counts from user scope.** A project-scoped writer
  would report success and change nothing.

**Two guards against misdelivery, at different layers**, because a
prompt landing in the wrong conversation is the worst thing this code
could do and pids are recycled: `procStart` from the key file is
compared against `ps -o lstart=` before connecting, and `session_id`
rides on every frame though CC treats it as optional.

**The grant** (`peer::inbound`) exists because both honest options are
bad: `hold` makes remote control useless, permanent `accept` leaves the
machine open forever. The setting is machine-wide by necessity, so the
blast radius is narrowed **temporally** — the deadline is the whole
feature, capped at `MAX_GRANT_HOURS`. Running sessions re-read the
setting **live**, which is what makes expiry meaningful rather than
advisory, and `eval::decide` checks **supersession before expiry** so a
hand-changed setting is left alone. Every CLI entry point calls
`ops::tick` first, so a window whose deadline passed while the GUI was
closed still closes.

## Remote control (LAN appliance, admin password)

Reaching Claudepot from a phone or another machine. Pure logic in
`claudepot-core::remote` (`bind` / `password` / `token` / `tls` /
`store` + the pairing state machine in `mod`); the client that ships at
`/` is `panel/`.

**The model is an appliance** — reachable on the LAN, over Tailscale or
not, behind one admin password, and whoever holds that password is
admin and may do anything. There is no endpoint allowlist. That is
coherent *only* because the password is treated as the entire security
boundary, and behind it sits the ability to drive Claude Code sessions,
i.e. arbitrary code execution as this user.

The reasoning behind every rule below — the measurements on a real
iPhone, the two failure modes an adversarial review found, what the
panel deliberately does not do, and the four traps in minting the
certificate — is in
[`docs/notes/remote-control.md`](docs/notes/remote-control.md). Read it
before changing this surface; most of the rules look arbitrary until
you know what they replaced.

**The hard rules, each of which the feature stops being safe without:**

- **`bind` is an allowlist**, and the highest-consequence line here.
  Permitted: loopback, RFC1918, link-local, Tailscale's
  `100.64.0.0/10`, and `0.0.0.0`. Refused: anything **globally
  routable**. `0.0.0.0` returns `Exposure::EveryInterface` so the
  caller must say so. `100.x` is not automatically Tailscale —
  matching the first octet would allow routable space.
- **TLS is required iff the bind address is not loopback**
  (`BindAddr::requires_tls`), and there is no downgrade switch:
  `remote::tls` stops the server rather than falling back, because a
  silent downgrade leaves the user believing traffic is protected.
  Certificates come from a private CA (`scripts/mint-remote-cert.sh`)
  because this tailnet's control server cannot issue them.
- **The password is hashed with scrypt, and `remote::token` argues the
  opposite for itself deliberately.** A 256-bit machine token has
  nothing to brute force; a human-chosen password does. Both modules
  are correct for their own input — **do not unify them onto one hash.**
- **The throttle backs off; it never locks out.** A hard lockout on a
  LAN-reachable appliance is a denial of service handed to anyone on
  the wifi. Failures buy an exponential delay capped at 30s. The
  *pairing code* may burn itself, because the user can mint another at
  the machine; an admin locked out over the network cannot.
- **`verify_password` must distinguish "wrong password" from "stored
  hash unusable"** — a PHC string can parse and carry no hash output,
  and `.is_ok()` would tell the owner their correct password is wrong,
  forever, with nothing pointing at the file.
- **A bearer token defends against other devices, not against local
  code.** A same-UID process can write its own device record, and can
  already drive CC's socket directly. The honest claim is narrow: this
  stops another *device* acting without credentials, and a *revoked*
  device acting at all. Do not write docs or UI implying more.
- **TOTP is an optional SECOND factor and must never be the only one.**
  Its secret cannot be hashed, so it would be strictly more valuable at
  rest than a password hash. Codes are **burned** via a high-water
  counter (RFC 6238 §5.2; most implementations omit it), and SHA-1 is
  correct rather than an oversight.
- **Passkeys are the better end state and are built** (`remote::webauthn`
  / `remote::passkey` / `remote::api`): the server stores only a public
  key. Registration requires an authenticated session; the RP ID is
  derived from the request and never configured; `login/begin` sends an
  empty `allowCredentials`; a passkey login mints exactly the same
  `Device` row a password login does. **The origin must be a hostname,
  not an IP** — an IP origin has no RP ID, and
  `isUserVerifyingPlatformAuthenticatorAvailable()` reports `true` on
  exactly the origin that cannot use one.
- **Every verb is `remote::service`, and there is exactly one of it.**
  A Tauri command written against `claudepot_core::remote` directly
  would reimplement `revoke_all`'s refusal, `enable`'s preflight order
  and the recovery warning. What stays per-caller is presentation.
- **The server runs in-process on a tokio task**
  (`src-tauri/src/remote_server.rs`), not as a daemon, because the
  approval hook is armed for exactly as long as a server is up and
  that coupling is what makes the capability acceptable. The cost is
  disclosed, not discovered: quitting Claudepot stops the surface,
  enforced on `RunEvent::Exit` (not `ExitRequested`, which can be
  prevented).
- **Three states, not two, and two liveness fields.** `server.enabled`
  is a stored preference that survives `kill -9`;
  `approval::store::is_serving` is the heartbeat; `running_here`
  distinguishes our server from a `claudepot remote serve` in a
  terminal. Collapsing any of them is a review finding, and
  `RemotePane.test.tsx` asserts all three.
- **Secret direction is unchanged**: the password crosses *in* over IPC
  and is zeroized on every path of `service::set_password`; nothing
  returns it, and `DeviceSummary` does not carry the token hash. TOTP
  enrolment from the GUI would need the `key_*_copy` treatment — an
  `otpauth://` URI is a secret coming *back*.
- **Two chips, not one.** `RemoteServingChip` is this surface;
  `RemoteWindowChip` is CC's `crossSessionInbound`. Different blast
  radii; folding them would leave a user unable to tell which door is
  open. The pane is `group: "core"` because it is where you revoke a
  lost phone, and Quick prompts live inside it because they are the
  panel composer's chips and mean nothing elsewhere.

### The panel — the client that ships at `/`

`panel/` is a self-contained Vite app (its own install; **not** a
workspace member of the Tauri renderer, which carries over 300
`invoke` calls that mean nothing over HTTP). It builds into
`crates/claudepot-core/src/remote/assets/panel/`, which is
**committed**, so `cargo build` needs no Node. **Rebuild with
`scripts/build-panel.sh` after touching anything under `panel/`** —
nothing else notices, and the previous bundle ships silently. The
committed output is ~3.8 MB embedded in every binary whether or not the
remote surface is ever switched on: a ~426 KB base bundle plus 61
lazy mermaid chunks, whose route table is *generated* into
`assets/panel_chunks.rs` (a runtime directory walk would hand
`remote::assets` the traversal surface it exists not to have).

| Route | Notes |
|---|---|
| `GET /api/sessions` | live PID registry joined to `session_index`; live always, plus the 20 most recently touched. Carries `server_version`, which is what the panel's tap-to-reload bar compares |
| `GET /api/sessions/{id}/transcript` | `tail` / `after` / `before` windows, `no-store`. **The secret-bearing endpoint** |
| `POST /api/sessions/{id}/prompt` | the only write that reaches Claude Code, through `peer` |
| `POST /api/sessions/{id}/read` | per-device read mark |
| `GET /api/accounts`, `POST …/{email}/activate` | read-only except activate, which moves **the CLI slot only** and answers **409 `live_session`** unless the caller asks for `force` |
| `GET /api/sessions/{id}/commands`, `…/commands/{name}` | slash commands as **text**; cwd resolved from the session, never from the client |
| `GET /api/approvals`, `POST /api/approvals/{id}` | the only route that **grants a capability**; alive only while `remote serve` is |
| `POST /api/passkey/{register,login}/{begin,finish}` | register is authenticated, login is not |

**Panel rules that are not up for re-litigation** (reasoning in the
note): a card is titled by the **last** user prompt, not the first, and
the title is markdown-**stripped** while the body is markdown-rendered;
live cards order by `last_reply_ts`; no `failed`, `stuck`, `idle_ms` or
tool-call count is synthesised; token components travel, not one sum;
the transcript is **masked, never scrubbed**, and the UI says so; tool
output never renders as markdown; mermaid runs with `htmlLabels: false`
and the desktop keeps `dangerousDisableAssetCspModification:
["style-src"]`; there is no projects surface and no project move; there
is no interrupt, because CC's peer inbox has no such verb; the offline
outbox mints its idempotency key at **enqueue**; the back gesture is a
single `popstate` closer; `/` **stages** a slash command and `…`
**sends** a quick prompt; answering an `AskUserQuestion` sends a
prompt and says "handed off", because whether that resolves the tool
call has **not been measured**.

**Approving from the phone** (`remote::approval`) is the one thing here
that grants rather than reads, and it uses CC's `PermissionRequest`
hook — a different door from peer messaging, so the laundering
reasoning in "## Peer messaging" stays enforced. Five properties hold
it: **silence is the fall-through** (a hook that prints nothing leaves
the normal prompt at the machine, so every failure degrades to today's
behaviour); it is armed only while `remote serve` is up (SIGINT **and**
SIGTERM); **the runtime gate is the half that holds**
(`store::gate` believes the heartbeat, not the preference); the wait
ends before CC's does (`WAIT` 110s < `HOOK_TIMEOUT_SECS` 120s < CC's
300s clamp, which *kills* the hook and blocks the call); and **one
writer per file** — request and decision are two files in two
processes, where a mutex buys nothing. It has its own switch,
`approvals_enabled`, which defaults to **true** unlike
`server.enabled`, and is checked in `store::gate` at runtime rather
than only at install.

Be exact about what it widens: a stolen bearer token could already read
transcripts and inject text CC refuses to treat as approval; with this,
it can approve a tool call. That does not weaken the password
boundary — it means the boundary has less behind it in reserve.

Still absent by design: pairing-code display and QR, and TOTP/passkey
enrolment from the GUI. Still open: mDNS discovery for multiple
appliances on one LAN, and a streaming surface — which when added must
**close** on credential revocation, not merely refuse the next request.

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

Reads and writes CC's `cleanupPeriodDays` — **a setting that destroys
user data, and one CC's own UI never mentions**. Pure logic in
`claudepot-core::cc_retention` (not `retention`, which already owns
Claudepot's own pruning horizon); commands in
`src-tauri/src/commands/cc_retention.rs`; pane at
`src/sections/settings/RetentionPane.tsx`.

Four properties drive the whole design; changing any invalidates the
pane. The CC archaeology behind each — which version changed what, and
the one reversal that made a passing test assert the opposite of the
truth — is in
[`docs/notes/cc-integration.md`](docs/notes/cc-integration.md).

- **Sliding, not one-shot.** `getCutoffDate()` recomputes
  `now - cleanupPeriodDays` on *every* run, so loss is continuous.
- **`0` is not on the duration scale, and CC rejects it** since
  2.1.233 (`cc_retention::MIN_CLEANUP_PERIOD_DAYS` is the pin). There
  is **no way to disable transcript persistence for an interactive
  session**, so Claudepot no longer offers one — `disable_persistence()`
  was deleted rather than repointed. A `0` written by an older
  Claudepot now does the *opposite* of what its author chose:
  `RetentionMode::LegacyZero`, kept distinct from `Invalid` because the
  repair copy differs.
- **Any value CC's schema rejects suppresses cleanup entirely**, so an
  invalid value accidentally *protects* transcripts. The UI must say
  "fix the value", never "restore the default", and **any control that
  lifts suppression confirms first** — while suppressed, a preset
  button is a one-tap destructive action on the whole backlog. This is
  why `settings_writer::read_i64_setting` returns a three-state
  `SettingValue`: collapsing "absent" with "present but wrong type"
  reported a 30-day timer over history CC was leaving alone.
- **It is bigger than the transcript count, and it is not a transcript
  setting.** Deleting `projects/<slug>/<uuid>.jsonl` also `rm -rf`s the
  sibling session folder with no per-file age check, and the key is a
  global TTL over ~20 directories under `~/.claude`. `TranscriptRisk`
  counts `projects/`; `claudepot-core::cc_sweep` counts the rest **in
  the unit CC actually deletes** — files for some directories,
  immediate subdirectories for others, which is why `SweepUnit` is
  explicit per row.

`TranscriptRisk::scan_incomplete` is load-bearing: a scan that failed
must never render as "nothing is scheduled for deletion".

**There is a second destroying key, and Claudepot does not model it.**
`desktopSessionCleanupPeriodDays` (CC 2.1.248+, schema
`int().nonnegative().optional()`, re-seen in 2.1.274) is the retention
ceiling for transcripts created or last written by a desktop-host
surface (Claude Desktop, Cowork), **which are exempt from the
`cleanupPeriodDays` sweep entirely**. Three consequences: its `0`
default means "keep forever", the exact opposite of `cleanupPeriodDays`
rejecting `0`, so a control reusing this pane's validation would be
wrong; `TranscriptRisk` counts all of `projects/` and therefore
**over**-reports risk for desktop-written transcripts (the safe
direction, but a real gap); and CC's suppression check loops over both
keys, while `cleanup_suppressed` models one. The row in
`crates/xtask/cc-upstream-watch.md` carries the full measurement.

Boot check at `src-tauri/src/retention_boot_check.rs` emits **at most
one** bell entry, choosing between two conditions core guarantees
cannot both hold:

| Condition | Core decision fn | Category |
|---|---|---|
| deletion is coming | `cc_retention::warning` | `TranscriptsExpiring` |
| deletion is switched **off** and you were not told | `cc_retention::cleanup_suppressed_warning` | `TranscriptCleanupSuppressed` |

Two categories rather than one because the **mute decision differs**.
Neither has a dismissal flag — gating on the condition means fixing the
setting silences it. Both bodies are locked byte-for-byte against
core's `message()` under `en`, and the suppressed body additionally
asserts it never contains "will be deleted": the two entries say
opposite things, and a user who reads the wrong one takes the wrong
action on a setting that destroys data.

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

English is the source of truth everywhere; a missing translation falls
back to English rather than failing, which is why the catalogs are
gated. Full design in `dev-docs/i18n-plan.md` (local, untracked); the
rules below each cost a shipped bug, and the stories are in
[`docs/notes/i18n.md`](docs/notes/i18n.md).

**Three catalogs, three owners. They are not interchangeable:**

| Catalog | Covers | Loaded by |
|---|---|---|
| `src/locales/<locale>/<ns>.json` | the React UI, 16 namespaces | `src/lib/i18n.ts`, **synchronously** (`initAsync: false`) so the first paint is localized. `src/types/i18next.d.ts` types `t()` against the *English* catalogs, so a missing key is a compile error rather than a runtime English leak |
| `src-tauri/i18n/<locale>.json` | app menu, tray, the four OS-banner modules | hand-rolled lookup in `src-tauri/src/i18n.rs`, `include_str!`. Deliberately not a crate dependency: ~130 flat keys and zh needs no plural rules |
| **nothing in `claudepot-core`** | — | core's `thiserror` strings stay canonical English: the CLI prints them verbatim and the GUI uses them as its fallback. Localizing core would fork the CLI's output |

**What stays English, permanently:** CLI stdout/stderr and `--json`,
logs and tracing, core error `Display` text, and technical identifiers
(paths, model ids, CC setting keys, env var names, anything the user
copies or types). **Type-to-confirm gates are the one deliberate
exception** — a zh user must be able to type the phrase they are shown.
The surviving gate is `projects:repair.abandonPhrase`, and
`src/lib/i18n.test.ts` locks it.

**Load-bearing rules:**

- **Module-level label constants freeze the boot language.** A
  `const X = { label: "Foo" }` evaluated at import time never follows a
  language switch. Use a `labelKey` resolved where rendered, or a lazy
  `get label()` — `sections/settings/panes.ts` and
  `sections/global/tabs.ts` are the reference implementations, and they
  stay JSX-free so the ⌘K palette can import them without dragging
  their section chunks into the main bundle.
- **Locale preference is `Option<String>`, and `None` means follow the
  OS.** Never write a resolved locale back into `preferences.json`, or
  "follow system" stops following. `localStorage` mirrors the
  *preference* only so first paint is correct before IPC returns.
- **`sys-locale`, not `LANG`.** Dock-launched macOS apps inherit no
  env, so env-var detection silently resolves everyone to English.
- **CJK glyphs come from Sarasa Mono SC**, `unicode-range`-gated in
  `index.html` so an English-only session never downloads ~9 MB.
- **Section labels live in `shell:sections.*`** keyed by registry
  `labelKey`; log tags and `ErrorBoundary` labels use the section `id`
  instead, because machine-facing strings must not move with the UI
  language.
- **Notification category names key off the category id**
  (`src/lib/notifications/labels.ts`), not the English label core ships
  over IPC. The fixture test in `src/lib/notifications/types.test.ts`
  fails when a new core category lacks catalog entries — the moment the
  English fallback would start leaking into a zh UI.

`pnpm check:catalogs` is the gate (see "## Test" for what its
"orphan" check can and cannot see).

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

CI's clippy + Windows-test gates run on runners local macOS can't
reproduce. Two validator hosts stand in, reached over the legio
tailnet — real names in `CLAUDE.local.md`, read by
`scripts/pre-push` from the gitignored `.validator-hosts` or from
`CLAUDEPOT_VALIDATOR_LINUX_SSH` / `CLAUDEPOT_VALIDATOR_WINDOWS_SSH`:

```bash
# <runner-a>, Ubuntu aarch64 — the same command as CI's Format / Clippy (Linux)
cargo clippy --all-targets -p claudepot-core -p claudepot-cli -- -D warnings
# <runner-b>, Win 11 MSVC x86_64 — the same compile step as CI's Tests (windows-latest)
cargo test -p claudepot-core -p claudepot-cli --no-run
```

The hook is committed at `scripts/pre-push` and installed per clone
with `scripts/install-hooks.sh`. It runs both validators **only** when
the push contains a `refs/tags/v*` tag; branch pushes skip.

Four rules, each of which was learned by the gate silently not running
(the full account is in
[`docs/notes/assets-and-release.md`](docs/notes/assets-and-release.md)):

- **Never hand-symlink the hook into `.git/hooks/`.** A global
  `core.hooksPath` — set by the git-lfs installer and most dotfile
  setups — makes git ignore that directory entirely, so a symlinked
  hook reports "Installed" and never runs. Four tags shipped that way.
  `install-hooks.sh` points `core.hooksPath` at a generated, gitignored
  `.githooks/` that chains to whatever the clone previously inherited.
- **Verify an install rather than trusting it**: `git config
  core.hooksPath` should print a `.githooks` path, and a dry-run push
  of a throwaway `v*` tag should print the validator banner.
- **Every generated hook carries a re-entry guard**
  (`CLAUDEPOT_HOOK_<NAME>`), because the chain runs both ways: this
  clone's `.githooks/pre-push` calls the inherited hook, and
  `~/.git-hooks/pre-push` calls `$root/.githooks/pre-push` on the
  assumption that a repo using that directory leaves `core.hooksPath`
  alone. Without the guard they called each other until 796 processes
  were running and a release push hung (2026-09-17). Shims are also
  generated for **every client hook name**, not a snapshot of the
  inherited directory — a global `pre-commit` added after install had
  been silently skipped. `bash scripts/install-hooks.sh --self-test`
  reproduces both, bounded, and runs in CI's lint job. Re-run
  `scripts/install-hooks.sh` in an existing clone to pick it up.
- **When a host is unreachable the hook defers to CI, and the
  asymmetry is deliberate**: absence of evidence falls back to the
  green `ci.yml` run for the same commit, contrary evidence never
  does. "Runs the gate" is the operative phrase and has been misjudged
  twice in the same direction — a host that could not reach GitHub and
  one that could not download a crate were both reported as "clippy
  failed" over a gate that never started. Only the gate command's own
  exit is contrary evidence.
- **The lookup dereferences `^{commit}` first** — an annotated tag's
  own sha differs from the commit's, and CI indexes runs by commit — and
  it needs an authenticated `gh`, since an unauthenticated one reads as
  "no run" and aborts, which is the safe direction.

The workflow that keeps the gate real: push the branch, let CI finish,
then push the tag. `--no-verify` was becoming the reflex, and a bypass
used routinely is indistinguishable from no gate at all.

## Architecture

See `dev-docs/implementation-plan.md` for the full plan.

- Five nouns: **account**, **cli**, **desktop**, **project**, **agent**
  (see `.claude/rules/architecture.md` for each noun's scope)
- `claudepot-core` = pure Rust library, no Tauri dependency
- `claudepot-cli` = thin clap wrapper over core
- `src-tauri` = Tauri app consuming same core
- `crates/xtask` = workspace automation: `verify-cc-parity` (the
  settings-merge goldens over `parity-harness/`), `verify-docs` (the
  doc/code contracts CI enforces), `verify-screenshots` (on demand),
  `cc-drift` (the CC watchlist report) and `screenshot-fixture`
- Separate keychain surfaces on macOS — CC's item vs Claudepot's own
  slots, `keyring` vs `/usr/bin/security` (see rules/architecture.md)
- Account identity = email, resolved by prefix matching
- GUI is paper-mono shell: custom 38px `WindowChrome` at top
  (breadcrumb + ⌘K palette hint + bell + theme toggle), 240px `Sidebar`
  on the left (swap targets + primary nav + live Activity strip
  + synced strip — **starts collapsed to its rail**; ⌘\ or either
  chevron expands it, the choice is remembered per device in
  localStorage as an explicit three-state value, and there is no
  Settings toggle for it), content column, 24px `StatusBar` at bottom.
  The screenshot capture expands it before navigating, because the
  rail hides the labels the script matches on.
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

`dev-docs/kannon/reference.md` — 3400-line verified reference for
CC/Desktop internals. **`dev-docs/` is gitignored**, so every pointer
into it in this file resolves only on a machine that already has it;
nothing there is part of a fresh clone. Tracked long-form notes live in
`docs/notes/`.

**Verify against the installed binary, not the source mirror.**
`~/github/claude_code_src` is a third-party mirror pinned at **2.1.88**
and abandoned upstream on 2026-04-15 — 145+ versions stale, and it does
not move again. Treat it as archaeology. Telling every agent to "verify
against CC source" there made it a *drift generator*: each pass would
confidently confirm April's behaviour and report success.

Claude Code ships as a bun-compiled binary that retains readable JS and
string literals, so it is the authority:

```bash
strings -n 60 ~/.local/share/claude/versions/<ver> | grep '<pattern>'
claude --help | grep -- '--<flag>'
```

That is how the `cleanupPeriodDays` inversion surfaced — the complete
validation message sits in the binary in plain text, and contradicted
both this repo's docs and the mirror.

CC ships **~27 releases a month**, so any CC claim more than a few weeks
old is a hypothesis — including the dated pins in this file. Installed
here on 2026-09-17: **2.1.274**, against verification pins that run from
2.1.233 to 2.1.259, and a parity harness still pinned at 2.1.88.
`cargo xtask cc-drift` prints exactly that gap plus the changelog
candidates; it reports candidates, not findings. `.claude/rules/cc-upstream-watch.md` carries the two
standing rules; `crates/xtask/cc-upstream-watch.md` is the list of
surfaces that drift and how to check each one (it sits by the tool that
reads it, since `.claude/rules/` is loaded into every session);
`dev-docs/cc-upstream-watch.md` is the routine's design.

## Icon assets

**The authored set lives in `assets/icon-set/`** — isometric block on
an anodised plate, every coordinate a multiple of 16 on a 1024 grid.
That directory is the source; `src-tauri/icons/` holds only what
`scripts/regen-icons.sh` derives from it, plus the two masters the
script reads directly (`icon.svg`, `icon-flat.svg`). There is
deliberately no second copy of the artwork anywhere — two plausible
masters in one directory is how the wrong one gets regenerated from.

The full v0.1.13–0.1.19 Dock-blur arc is in
`dev-docs/icon-design-notes.md` (local, untracked); the measurements
that produced the rules below are in
[`docs/notes/assets-and-release.md`](docs/notes/assets-and-release.md).

- **SVG must use a power-of-2-friendly grid** (16-unit multiples in a
  1024 viewBox). Avoid 22, 28, 30 — they don't divide 128/256 cleanly
  and rsvg AA-softens at every Dock size.
- **Generate rasters with `scripts/regen-icons.sh`, not
  `pnpm tauri icon`**, which resamples some `.icns` layers lossily and
  writes ~50 dead-byte files for targets we don't ship. Its output
  paths are `.gitignore`d so a stray invocation cannot re-stage them.
- **Three masters, and the split is not cosmetic:** `icon.svg` (plated,
  `feTurbulence` grain, 48 px and up), `icon-flat.svg` (same artwork,
  no filter, **below 48 px** — the grain is computed at render size, so
  it coarsens relative to the tile and reads as dirt), and
  `assets/icon-set/windows/icon-glyph.svg` (plateless, for `icon.ico`,
  because Windows draws no enclosure).
- **Tray icons are generated too** (`tray-icon{,Alert}{Template,Mono}@2x.png`,
  44×44). Template is inverted by macOS; Mono is the same alpha filled
  `#808080` because Windows and Linux have no template concept.
  **`tray-icon` normalises the tile to 18 points tall**, so the tile's
  pixel size is irrelevant and its padding is pure loss — only the
  fraction of the tile the glyph inks decides how large it lands.
  Both variants share one viewBox *size* so the block does not change
  scale when the alert badge appears, and the badge sets the floor on
  how tight the crop can go.
- **`scripts/verify-icons.py` is the structural gate** — 58 checks over
  the PNG ladder, the `.icns` layer list, ICO layer encoding, tray
  sizes and the grain floor. **The rendered point height is the
  assertion that matters**: every dimension check passed while the tray
  icon was visibly too small. Run it after any icon change; it does not
  check how the artwork *looks*, which is what launching the app is for.
- **The bundle path and the `setIcon` path want OPPOSITE artwork**, and
  swapping them is the classic macOS icon bug:

  | Path | Wants |
  |---|---|
  | bundle `.icns` / `bundle.icon` list | **full bleed** — macOS applies the squircle mask, inset and shadow |
  | `setApplicationIconImage` (`dock_icon.rs`) | **everything already applied** — drawn verbatim at slot size |

  So `dock_icon.rs` embeds `icon-dock.png`, not `icon.png`: artwork
  inset to 824/1024 = 0.805, superellipse corner (n = 5). A full-bleed
  image on that path renders ~22% larger than every neighbouring Dock
  icon. The call itself is **required** — Tauri's runtime only does it
  in dev, and without it the prod Dock downscales the 128 layer
  bilinearly and looks soft.

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

**Quit the installed Claudepot first** — the debug binary shares the
single-instance identifier, so it hands off to the running app and
exits 0, which then reports as "no MCP bridge on 9223".

Three rules, with the reasoning in
[`docs/notes/assets-and-release.md`](docs/notes/assets-and-release.md):

- **The fixture is a fake `HOME`, not a pair of env overrides.**
  `CLAUDE_CONFIG_DIR` + `CLAUDEPOT_DATA_DIR` cover two of the three
  places the app reads; Claude Desktop's directory resolves through
  `dirs::data_dir()` with no override and leaked a real account.
- **The fake home goes to the app, not the build** (`HOME=… pnpm tauri
  dev` takes rustup's toolchain with it), and **the fixture lives
  outside the repo** (`/tmp/claudepot-demo-home`) because the app
  displays the paths it reads.
- **Never mask real data to take a screenshot.** It was tried and it is
  architecturally wrong — substring replacement corrupts legitimate UI
  and free text defeats it entirely. Full reasoning in
  `crates/xtask/src/screenshot_fixture.rs`.

Adding a screenshot means two edits: a `SHOTS` row in
`scripts/capture-screenshots.mjs` and a `SCREENSHOTS` row in
`crates/xtask/src/verify_docs.rs`. Two checks read that table, and the
split is deliberate: **`verify-docs`** (in CI) asserts each shot exists
and that `assets/screenshots/` and `web/public/screenshots/` hold the
same bytes — content-based, and the fix is a file copy;
**`verify-screenshots`** (on demand, **not** a PR gate) reports shots
whose sources have moved, comparing commit dates per *directory*, so
adjacency is not staleness. Re-capturing needs a macOS GUI session,
Vite, a debug build and a window — CI has none of them, and a gate
whose remedy cannot run where it fires is how `--no-verify` became a
reflex elsewhere.

Known limitation: `HOME` does not redirect the macOS keychain, so the
Accounts pane's credential probe finds nothing and each card shows
"Saved login is missing or broken".

## Conventions

- Grill reports go in `dev-docs/reports/`. Never drop them at the repo root.

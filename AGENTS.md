# Claudepot

Control center for Claude Code and Claude Desktop. Tauri 2 + Rust + React:
accounts, projects, sessions, API keys, third-party providers, agents,
memory files, usage and cost, updates, notifications and a LAN remote
panel, under one shell with tray and menubar.

The domain model is five nouns — **account**, **cli**, **desktop**,
**project**, **agent**. Every other surface is a presentation layer or a
behaviour over those nouns and over CC's filesystem. Do not add a noun
casually (`.claude/rules/architecture.md`); that limit is on the domain
model, not on what the UI may expose.

## How the docs are split

This file is `@`-included into **every** session, so it holds commands,
layout and the rules that must not be broken, in a line or two each.
`docs/notes/` carries the reasoning: what was measured, which obvious
alternative was tried and reverted, and which shipped bug each rule was
written after. Notes are read on demand; they are not optional.

| Note | Read it before changing |
|---|---|
| [`data-dir.md`](docs/notes/data-dir.md) | anything in `~/.claudepot/` — the store inventory and each file's contract |
| [`gui-shell.md`](docs/notes/gui-shell.md) | the Tauri shell, the section registry, the ⌘K palette, event channels, the session index |
| [`cc-integration.md`](docs/notes/cc-integration.md) | retention, permission grants, peer messaging, CC env variables, any write to a CC settings file |
| [`remote-control.md`](docs/notes/remote-control.md) | `claudepot-core::remote`, `panel/`, approving from the phone |
| [`relay.md`](docs/notes/relay.md) | `claudepot-core::relay`, `claudepot relay`, Settings → Relay |
| [`core-modules.md`](docs/notes/core-modules.md) | pricing, the env vault, auto-rotation, token refresh, the corpus |
| [`i18n.md`](docs/notes/i18n.md) | any user-facing string or catalog |
| [`test-gates.md`](docs/notes/test-gates.md) | a `check:*` gate, and especially before relaxing one |
| [`assets-and-release.md`](docs/notes/assets-and-release.md) | icons, screenshots, git hooks, release validation |
| [`web.md`](docs/notes/web.md) | `web/` (claudepot.com) |

**A rule in this file whose reasoning is in a note must not be changed
from this file alone** — the note holds the argument against the obvious
alternative, which is what stops a decision being silently reverted.

`.claude/rules/*.md` are loaded into every session as well and are not
repeated here. `dev-docs/` is gitignored: a pointer into it resolves only
on a machine that already has it.

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

`panel/` has its own install and its build output is **committed**. Run
`scripts/build-panel.sh` after any change under `panel/` — nothing else
notices, and the previous bundle ships with no error anywhere.

## Test

```bash
cargo test --workspace               # Rust
cargo xtask verify-cc-parity         # CC settings-merge parity goldens (parity-harness/README.md)
cargo xtask verify-docs              # README / data-dir note / event-channel / screenshot contracts
pnpm test                            # React (Vitest + RTL, jsdom)
pnpm test:coverage                   # React with coverage report
cd panel && pnpm check:render        # the built remote panel actually mounts
pnpm check:classes                   # every className has a rule; every text field draws chrome
pnpm check:a11y                      # every role="switch" has an accessible name
pnpm check:motion                    # the reduced-motion override reaches the primitives
pnpm check:contrast                  # the prefers-contrast override is not lost to source order
pnpm check:catalogs                  # en↔zh key / placeholder / <Trans> tag parity
pnpm check:inline-flow               # prose inside a flex/grid container keeps its spaces
pnpm check:envvar-layout             # needs a screen; CI runs its unit half only
```

Every gate was written after a failure that had already shipped green,
and each can be forced to fail (`:self-test`, `--self-test`, or a fixture
for `check:catalogs`). Read `test-gates.md` before changing one. CI runs
core + cli tests on Linux, macOS and Windows, and the `claudepot-tauri`
crate's tests on macOS and Windows.

On the test host — real `<user>`, `<host>` and `<password>` are in the
gitignored `CLAUDE.local.md`:

```bash
cargo build -p claudepot-cli
scp target/debug/claudepot <user>@<host>:/tmp/claudepot
ssh <user>@<host> "security unlock-keychain -p <password> ~/Library/Keychains/login.keychain-db; /tmp/claudepot <command>"
ssh <user>@<host> "security unlock-keychain -p <password>; bash /tmp/claude-login-local.sh <email>"   # set up CC login state
```

## Layout

| Path | Role |
|---|---|
| `crates/claudepot-core` | all business logic. Pure Rust, no Tauri dependency |
| `crates/claudepot-cli` | thin clap wrapper over core |
| `src-tauri` | the Tauri app over the same core. `src/commands/` is one file per surface with **no business logic**; `src/dto.rs` holds what crosses to JS, and credentials never do |
| `src/` | React UI, plain CSS. `api/` and `types/` are sliced by domain and merged in `index.ts`; sections in `sections/`, primitives in `components/primitives/` |
| `panel/` | the remote panel — a separate Vite app whose build output is committed into core |
| `crates/xtask` | `verify-docs`, `verify-cc-parity`, `cc-drift`, `verify-screenshots`, `screenshot-fixture` |
| `web/` | claudepot.com — a separate Next.js app with its own install |

## GUI shell

- **`src/sections/registry.tsx` is the one list of sections.** Ten, in
  order: Accounts, Activities (id `events`), Projects, Knowledge
  (`shared-memory`), Keys, Providers (`third-party`), Agents
  (`automations`), Config (`config`), Boards (`boards`, off by default),
  Settings. The palette, ⌘1..⌘9, the shortcuts modal, the launch picker
  and the sidebar all derive from it through
  `src/lib/optionalSections.ts`. A second list of sections is a review
  finding. A new section is one registry entry, and that entry says what
  ⌘R does there.
- **The ⌘K palette has one ordering and one `activate(row)`** shared by
  keyboard and mouse. Add a row kind there, not at a second call site.
  Pane metadata (`sections/settings/panes.ts`, `sections/global/tabs.ts`)
  stays JSX-free so the palette can import it.
- **Long-running ops use the op-progress pipeline**: a `*_start` command
  spawns a task that emits on `op-progress::<op_id>`, and the modal
  subscribes by id. `RunningOps` in `src-tauri/src/ops.rs` is the polling
  backstop.
- **Every event channel in `src-tauri/src/events.rs` needs a subscriber
  in `src/`**; `verify-docs` fails otherwise. A deliberate exception goes
  in `UNSUBSCRIBED_BY_DESIGN` with its reason.
- **Measure before optimising.** Three optimisations that look obviously
  right were written, measured and reverted; `gui-shell.md` says which.
- `AccountStore.db` is `Mutex<Connection>` so stores can cross `await`
  points in Tauri commands.

## Data directory (`~/.claudepot/`)

- **Resolve it through `claudepot_core::paths::claudepot_data_dir()`**
  (override: `CLAUDEPOT_DATA_DIR`), never a hand-built `$HOME/.claudepot`.
- **A new store needs a row in `data-dir.md`.** `verify-docs` fails when
  a `*.db`, `*.json` or `*.jsonl` joined onto the data dir is missing
  from that note.
- **Every database opens through `db_pragmas::apply_standard_pragmas`**;
  `verify-docs` fails a `Connection::open` that doesn't. Per-store extras
  go in a second batch after the helper, never instead of it.
- **`sessions.db` and `corpus.db` are rebuildable; `boards.db` is user
  data** — migrations preserve its rows and nothing prunes it. Never put
  an archive in `sessions.db`: its refresh deletes every row it cannot
  see.
- **`sessions.db` has four invariants**: a refresh with an empty plan
  takes no write lock; a per-project read scopes both sides of the diff;
  index-backed Tauri commands borrow the shared `SessionIndex`
  (`shared_or_open`); a re-scan writes only the rows that differ.
- **A corrupt JSON store is never fatal at boot** — it is moved aside and
  the store starts empty. Four stores must also *report* the reset
  (`load_or_recover`), because a silent one reopens a door:
  `permission-grants.json`, `peer-inbound-grant.json`,
  `remote-devices.json`, `remote-config.json`.
- `remote-key.pem` and `remote-ca-key.pem` are private keys living in
  this directory. Do not sync, back up or attach it without knowing that.

## Pricing

- **`claudepot-core::pricing::PriceBook::resolve(model, day)` is the only
  rate resolution.** Rates are dated (`RATE_TIERS`) so a rate change does
  not re-score the past.
- **`src/costs.ts` mirrors it**, locked by
  `crates/claudepot-core/testdata/rate-resolution-vectors.json` and
  `cost-vectors.json`, which both sides run. Change one, change the
  other, add a vector.
- **Usage is counted per API message, not per transcript line**
  (`session::usage::UsageLedger`).
- A family estimate is always marked `≈`, never shown as a quote.

## Permission grants

- **A grant is a record, never a mode.** It is CC's `PreToolUse` hook
  (`claudepot hook pre-tool-use`) answering `allow`. Do not write
  `bypassPermissions` into project settings — CC ignores it since 2.1.257.
- **The hook never writes, recovers or logs** (`hook::load_readonly`). A
  corrupt file reads as "no grant".
- **A grant with no hook entry is an error, not a grant.**
- **Hook entries go through `cc_hook_entry` and point at the CLI**
  (`cc_hook_entry::hook_binary`), never at `current_exe()` blindly — from
  the GUI that is the desktop app.

## Peer messaging

- **It can inject a prompt and nothing else.** No interrupt, exit or
  restart exists; a UI must not offer one. Slash commands arrive as
  literal text.
- **Arrival is not delivery.** The success type is `Handoff`; never print
  "sent".
- **A peer cannot grant escalation.** CC calls that permission
  laundering, and a session refusing to approve its own prompt is
  correct.
- **`peerProtocol == 1` is a hard pin**, and both misdelivery guards stay:
  `procStart` checked before connecting, `session_id` on every frame.
- **The inbound grant is time-boxed** (`MAX_GRANT_HOURS`), counts only
  from user scope, and checks supersession before expiry.

## Remote control

- **The admin password is the entire security boundary**, and behind it
  is code execution as this user. A bearer token defends against other
  devices, not against local code. Do not write docs or UI implying more.
- **`bind` is an allowlist**: loopback, RFC1918, link-local, Tailscale's
  `100.64.0.0/10`, and `0.0.0.0` (returned as `Exposure::EveryInterface`).
  Anything globally routable is refused. `100.x` is not automatically
  Tailscale.
- **TLS is required iff the bind address is not loopback**, with no
  downgrade switch.
- **The password uses scrypt and the device token SHA-256. Do not unify
  them.** The login throttle backs off and never locks out. TOTP is only
  ever a second factor.
- **Every verb goes through `remote::service`** — CLI and GUI alike.
- **The server runs in-process**, and the approval hook is armed only
  while it serves; that coupling is what makes handing a network client a
  permission grant acceptable. A hook that prints nothing falls through
  to the prompt at the machine.
- **Three states, not two**: `server.enabled` is a preference,
  `is_serving` is the heartbeat, `running_here` says whose server it is.
- **Panel**: the transcript is masked, never scrubbed, and the UI says
  so; tool output never renders as markdown; there is no interrupt and no
  project move.

## Subscription relay

- **A behaviour over `account`, not a noun**, and not Remote Control:
  its own listener, port, device store, `cprl_` token namespace and CA.
  Share `remote::` code only as parameterised functions, never a service.
- **Never serve the CC-active account** — an outside renewal empties
  CC's login (measured). Checked at start and every renewal;
  `owner_conflict` stops serving.
- **Tokens come from `relay::lease`, never `launcher::get_access_token`.**
  One refresh per rejected generation.
- **User-actionable refusals are 403 with a fixed enum** — CC retries
  401/503 for minutes. Upstream text never reaches a log, error or event.
- **Binding is checked per request on the server**: a changed target
  is `acceptance_required` before any upstream contact.
- **The relay launch env sets `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=1`**
  and clears provider variables from `cc-env-spec.json`, not a list.
- **Always TLS, own CA; a missing CA key fails closed** — never mint a
  new CA silently. Offline config edits are refused while
  `relay-serve.lock` is held; the GUI relay stops on quit.
- **The revoke socket has one verb, and it only takes access away**
  (`relay::revoke_socket`, Unix): never add one that grants access. The
  peer uid is its boundary, checked on both ends; `relay-revoke/` is
  refused, never repaired.
- Non-Anthropic upstreams are **unpriced**, never priced by model name.

## Env secret vault

- An inbound secret is zeroized on every exit path. An outbound one
  crosses only through the Rust-side clipboard write and
  `KeyCopyReceiptDto` — never rendered.
- `.env` edits touch only the target key's line, and renderer-supplied
  file names are validated as bare dotenv names.

## Auto-rotation

- Rule logic is pure (`rotation::eval::evaluate`, clock injected); the
  runtime bridge is `src-tauri/src/rotation_orchestrator.rs`.
- Confirm is the default mode; promote to auto only after watching the
  rule fire correctly.

## Transcript retention

- **`cleanupPeriodDays` is the only CC setting that destroys user data.**
- **`0` is not on the duration scale** and CC rejects it since 2.1.233.
  There is no way to disable persistence for an interactive session, so
  do not offer one.
- **A value CC rejects suppresses cleanup entirely.** The UI says "fix
  the value", never "restore the default", and any control that lifts
  suppression confirms first.
- **A failed scan must never render as "nothing is scheduled for
  deletion"** (`TranscriptRisk::scan_incomplete`).
- **Do not model the desktop-session exemption.** Over-reporting is the
  safe direction.

## CC env variables (Config → Files → Env variables)

- The spec is generated: `scripts/build-cc-env-spec.py` writes
  `crates/claudepot-core/data/cc-env-spec.json`, and `verify-docs`
  regenerates it byte-for-byte. Never hand-edit it.
- **Clearing a value is never live** — CC's re-apply is additive-only —
  and every clear or restore confirmation says so.
- **Restore-default removes the key.** Never write the documented
  default, and never treat `""` as unset.
- **Build cross-check data is valid only on an exact version match**;
  otherwise the section renders "unavailable for this version".
- **"Safe to apply" is not "safe to display"** —
  `ANTHROPIC_CUSTOM_HEADERS` is pre-trust-safe and can still carry
  `Authorization: Bearer …`.
- User scope only. `CLAUDE_CONFIG_DIR` is read-only here.

## Locating CC's global config — one resolver

`paths::global_claude_json_target()` (or `resolved_global_claude_json()`
when "it doesn't exist" is a distinct answer) is **the** file CC reads.
Never join `.claude.json` by hand; `verify-docs` fails it.

## Settings-file mutation boundary

`claudepot-core::settings_mutex::mutate_settings_file` is the **only**
way to read-modify-write a CC settings file. A transition that changes
two keys is one closure, and the read that decides what to write happens
inside it — deciding from a snapshot taken outside is a race by
construction. CC and a text editor do not hold this mutex, so external
writes get change-detection and a rebase retry, not exclusion.

## Internationalization (en + zh-CN)

- English is the source of truth. Three catalogs: `src/locales/` (React),
  `src-tauri/i18n/` (menu, tray, OS banners), and **nothing in
  `claudepot-core`**.
- **Never localize** CLI output, logs, core error `Display` text,
  technical identifiers, or anything the user types or copies.
  Type-to-confirm phrases are the one exception.
- **No module-level label constants** — they freeze the boot language.
  Use a `labelKey` or a lazy `get label()`.
- **Locale preference `None` means follow the OS.** Never write a
  resolved locale back into `preferences.json`.
- Run `pnpm check:catalogs` after touching a catalog. It cannot see a key
  nothing references, so prune dead keys by hand.

## Corpus + detectors

- `corpus.db` is deliberately not `sessions.db`. No model calls anywhere
  in `claudepot-core::corpus`.
- Vocabulary: a *repetition cluster*, never a "recurrence"; `unresolved`,
  never "abandoned".

## Proactive token refresh

- **Never the active account** — that token belongs to Claude Code.
- **It does not implement a refresh.** It calls
  `services::identity::verify_account_identity`; reimplementing the
  exchange forks the email-drift protection.
- One account per tick. The five-minute cadence is the rate limit.

## Release validation

- A push containing a `refs/tags/v*` tag runs the Linux clippy and
  Windows compile gates on two validator hosts through `scripts/pre-push`;
  branch pushes skip. Hosts are named in `CLAUDE.local.md`.
- **Install hooks with `scripts/install-hooks.sh`.** Never hand-symlink
  into `.git/hooks/` — a global `core.hooksPath` makes git ignore it.
- **Push the branch, let CI finish, then push the tag.** `--no-verify`
  used as a reflex is no gate at all.
- An unreachable host defers to CI's green run for the same commit. A
  reachable host that fails never does.

## Icon assets

- `assets/icon-set/` is the only source. Generate with
  `scripts/regen-icons.sh`, not `pnpm tauri icon`, then run
  `scripts/verify-icons.py`.
- SVG coordinates stay on 16-unit multiples of a 1024 grid.
- **The bundle `.icns` wants full-bleed artwork; `dock_icon.rs` wants
  `icon-dock.png` with the mask and inset already applied.** Do not swap
  them.

## Documentation screenshots

- Capture from a synthetic profile: `cargo xtask screenshot-fixture`,
  then run the debug app with `HOME=/tmp/claudepot-demo-home` and
  `pnpm screenshots`. The fake home goes to the app, not the build.
- **Never mask real data to take a screenshot.**
- Adding a shot is two edits: a `SHOTS` row in
  `scripts/capture-screenshots.mjs` and a `SCREENSHOTS` row in
  `crates/xtask/src/verify_docs.rs`.

## Web (claudepot.com)

`web/` is a separate Next.js app deployed by Vercel. Schema changes follow
`.claude/rules/db-migrations.md` — never `drizzle-kit push` against
production. `pnpm test` there discovers `tests/*.test.ts`; do not go back
to a hand-kept list.

## Claude Code is a moving target

`.claude/rules/cc-upstream-watch.md` is the rule: verify CC behaviour
against the **installed binary**, never the source mirror, and give every
CC-facing module a row in `crates/xtask/cc-upstream-watch.md`.
`cargo xtask cc-drift` lists what moved since the pins — candidates, not
findings. Every CC version pin in this repo is dated, and a pin more than
a few weeks old is a hypothesis. `dev-docs/kannon/reference.md` (local,
untracked) is the long reference for CC and Desktop internals.

## Conventions

- Grill reports go in `dev-docs/reports/`. Never drop them at the repo root.

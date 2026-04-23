<p align="center">
  <img src="assets/pixel-claudepot.png" width="120" alt="Claudepot">
</p>

<h1 align="center">Claudepot</h1>

<p align="center">
  <strong>Multi-account switcher and session manager for Claude Code &amp; Claude Desktop.</strong><br>
  GUI + CLI. macOS-first, Windows/Linux aware. Tauri 2 · Rust · React.
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#features">Features</a> ·
  <a href="#cli">CLI</a> ·
  <a href="#architecture">Architecture</a>
</p>

# Clau++Deopt++

## Why

If you live in Claude Code, you have hit at least one of these:

- `/login` does not actually switch accounts when one is already signed in.
- Sessions vanish the moment you `mv` or rename a project directory.
- `~/.claude/` grows unbounded — multi-GB transcripts, no cleanup, eventual cascade failure.
- Claude Code freezes when a single transcript crosses \~50 MB.
- Personal vs. work account juggling means signing out, signing in, repeat.
- Rate limits (5-hour, weekly, Opus split) are invisible until you hit them.
- Tokens leak into pasted screenshots, exported chats, and clipboard.

Claudepot fixes each of these as a first-class feature — not a workaround.

## Features

### Multi-account, instantly switchable

- **Two independent slots**: one for the Claude Code CLI, one for Claude Desktop. Switch them separately.
- **One-click switch** from the sidebar, ⌘K command palette, or the menu-bar tray submenu.
- **Browser OAuth, ********`--from-current`******** import, or stdin refresh-token bootstrap** when adding accounts.
- **Email-prefix resolution** — `claudepot cli use li` is enough when the prefix is unambiguous.
- **Account verification** against `/api/oauth/profile` detects "drift" — when a slot labeled A is actually authenticated as B.
- **Split-brain warnings** if you switch while a Claude Code process is running and may revert the swap.
- **Per-account secrets in the OS keychain** (macOS Keychain, Windows Credential Manager, Linux Secret Service). The `Claude Code-credentials` keychain item is treated specially via `/usr/bin/security`, never the generic keyring API.

### Real-time activity (which session needs me right now?)

- **Live presence strip** in the sidebar: project · model · status (`busy · idle · waiting`) sorted by who needs attention first.
- **OS notifications** when a session goes from `busy` to `waiting`.
- **Tray submenu** lists every running CC top-level process (CLI, SDK, `-p`, daemon).
- **Status-bar live count**, ⌘⇧L to toggle the Activity pane.

### Usage that's actually visible

- **5-hour rolling cap, 7-day cap, Opus / Sonnet splits, extra-credit balance** — all rendered per account.
- **Anomaly-only banners**: if everything is healthy, the UI stays quiet.
- **Sidebar usage bar** with percentage and reset time inline (`████████░░ 72% · resets 4pm`).

### Project rename without losing sessions

`claudepot project move <old> <new>` does what `mv` doesn't:

- Moves the directory **and** rewrites every CC reference (`~/.claude/projects/<slug>/`, session JSONLs, `.claude.json` projects map, history file, project memory, settings).
- **Journaled in 9 phases** with snapshots — crashed runs are resumable via `claudepot project repair --resume`, reversible via `--rollback`.
- **`--dry-run`**** shows the full plan** before any byte moves.
- **Lock detection** prevents stomping a session that CC currently has open.
- Picks up **orphan slugs** left behind by `git worktree remove` and adopts them into a live `cwd`.
- **Windows-aware** (drive letters, UNC, `\\?\` verbatim) — golden-tested on `windows-latest` CI.

### Session management built for the JSONL reality

- **Sessions tab** with cross-session text search, repository grouping (worktrees collapsed), filters by project/date/error/sidechain/size/tokens.
- **`session view`** classifies one transcript: chunks, tool calls, subagents, phases, context attribution.
- **`session move`** relocates a single transcript across projects; `adopt-orphan` does it in batch.
- **SQLite index** (`~/.claudepot/sessions.db`) keyed by `(size, mtime_ns)` so CC compaction and `session_move` rewrites do not poison the cache.

### Disk reclamation, dry-run by default

- **`session prune`** — bulk delete by `--older-than`, `--larger-than`, `--project`, `--has-error`, `--sidechain`. Dry-run unless `--execute` is passed.
- **`session slim`** — rewrite one transcript, dropping `tool_result` payloads over a size threshold (default 1 MiB). Never drops user prompts, assistant text, tool *calls*, compaction markers, or sidechain pointers. Atomic; aborts if the file changed mid-rewrite.
- **`session trash`** — every prune/slim writes to a journaled trash with 7-day retention. `list` / `restore` / `empty`. Nothing is irreversible for a week.

### Export and share, with redaction on by default

- **Formats**: `markdown`, `markdown-slim`, `json`, `html` (interactive copy buttons; strip with `--html-no-js`).
- **Destinations**: `file`, `clipboard`, `gist` (secret by default; `--public` opt-in).
- **Redaction layers** — `sk-ant-*`, `Authorization` headers, JWTs, OAuth bearer tokens, URL params, cookies are stripped before any export, any preview event, or any line in `sessions.db`. Optional: `--redact-paths {relative|hash}`, `--redact-emails`, `--redact-env`, `--redact-regex <pattern>` (repeatable).
- **GitHub PAT** stored in the OS keychain via `keyring` (`claudepot.github-token`), or `GITHUB_TOKEN` env var.

### Diagnostics that point at the real problem

- **`claudepot doctor`** — platform, data dir, CC binary, Desktop install, keychain readability, OAuth beta header, API reachability (`Reachable | GeoBlocked | Unreachable`), per-account token status, last-known verify state.
- **`claudepot status`** — ground-truth check: reads CC's slot, calls `/api/oauth/profile`, prints `MATCH | DRIFT | UNTRACKED | NO BLOB`. Exit-code contract suitable for CI.

### CLI + GUI parity

- **Same Rust core** under both. The CLI handler is the reference implementation; the Tauri app wraps the same function with a DTO layer; React calls Tauri. No business logic in either shell.
- **Every CLI command supports ****`--json`** with a stable shape — scripting works.
- **Exit-code contract**: `0` ok · `1` general · `2` ambiguous/drift · `3` auth/uncheckable · `4` Desktop quit failed · `5` network.
- **`claudepot cli run <email> -- <cmd>`** launches a one-shot command with `CLAUDE_CODE_OAUTH_TOKEN` injected, leaving the on-disk slot untouched.

## Install

> **Status:** alpha (`0.0.x`). macOS daily-driven; Windows/Linux build green, less seasoned.

### From source

```bash
git clone https://github.com/xiaolai/claudepot-app.git
cd claudepot-app

# CLI
cargo build -p claudepot-cli --release
# binary: ./target/release/claudepot

# GUI (Tauri)
pnpm install
pnpm tauri build --no-bundle    # binary only, no .dmg
pnpm tauri dev                  # hot-reload dev mode
```

Data lives at `~/.claudepot/` (override with `CLAUDEPOT_DATA_DIR`). Two SQLite files:

- `accounts.db` — account registry, linked to keychain blobs.
- `sessions.db` — transcript metadata cache, rebuildable any time.

## CLI

```text
claudepot account   list | add | remove | inspect | verify
claudepot cli       status | use <email> | clear | run <email> -- <cmd>
claudepot desktop   status | use <email>
claudepot project   list | show | move | clean | repair
claudepot session   list-orphans | move | adopt-orphan | rebuild-index
                    view | export | search | worktrees
                    prune | slim | trash {list|restore|empty}
claudepot doctor
claudepot status
```

Global flags on every command: `--json`, `--quiet`, `--verbose`, `--yes`.

Full reference: \``.

## Architecture

Four user-facing nouns. Nothing else gets added without explicit discussion.

| Noun        | What it is                                               |
| ----------- | -------------------------------------------------------- |
| **account** | A registered Anthropic identity. Email is the name.      |
| **cli**     | The single slot Claude Code reads credentials from.      |
| **desktop** | The single slot Claude Desktop reads session files from. |
| **project** | A CC project session directory.                          |

Three crates:

- **`claudepot-core`** — pure Rust library. No Tauri dependency. All business logic. Testable without a webview.
- **`claudepot-cli`** — thin clap wrapper. No business logic. No HTTP. No keychain.
- **`src-tauri`** — Tauri app calling the same core functions. DTOs in `dto.rs`; credentials never cross to JS.

Two distinct keychain surfaces on macOS: the `keyring` crate for Claudepot's own secrets, the `/usr/bin/security` subprocess for CC's `Claude Code-credentials` item. They are not interchangeable.

See `and` for the full design.

## Development

```bash
cargo check --workspace               # Rust
cargo test  --workspace               # Rust tests
pnpm test                             # React (Vitest + RTL, jsdom)
pnpm test:coverage                    # with coverage report
pnpm tauri dev                        # GUI hot reload (Vite :1430, HMR :1431)
```

Path-handling code (sanitize, unsanitize, canonicalize) is the highest-risk surface and is golden-tested on Linux/macOS/Windows in CI. See \``.

## Security posture

- Tokens never logged, never toasted, never crossed to the webview, always truncated in any human output (`sk-ant-oat01-Abc…xyz`).
- `sk-ant-*`, OAuth bearer tokens, JWTs, Authorization headers, URL params, and cookies redacted before any session event reaches the UI or the index.
- `first_user_prompt` redacted before insert into `sessions.db`.
- SQLite WAL/SHM sidecars forced to materialise then `chmod 0600`.
- Tauri `opener` scope narrowed to `https://console.anthropic.com/settings/keys`.
- See \`` for the full rules.

## License

ISC — see [LICENSE](./LICENSE).

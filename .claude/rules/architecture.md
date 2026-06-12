---
description: Core architecture constraints for all Claudepot work
globs: "{**/*.rs,src/**/*.{ts,tsx},src-tauri/**/*.rs}"
---

# Architecture

## Domain model — five nouns

- **account** — a registered Anthropic identity. Email IS the name.
- **cli** — the single slot CC reads credentials from.
- **desktop** — the single slot Claude Desktop reads session files from.
- **project** — a CC project session directory. Available in both CLI and GUI.
- **agent** — a scheduled headless `claude -p` run: a
  `(binary, prompt, schedule, cwd, …)` record that materializes into
  a per-OS scheduler artifact (launchd plist / Task Scheduler XML /
  systemd-user timer). Each run lands as `result.json` + logs.
  Triggers: cron, manual, and the `session-settled` reactive event.
  Store: `~/.claudepot/agents.json` (`claudepot-core::agent`).
  Added after explicit discussion — the only noun added since the
  original four; its cardinality/lifecycle rules live in
  `claudepot-core::agent`'s module docs.

CLI and Desktop are independent. Never couple them.

Credential, Profile, Usage are internal — not user-facing nouns.
Routes (the Providers tab, `claudepot-core::routes`) are provider
definitions — `(provider, base URL, auth, models)` — not accounts and
not a noun: they carry no Anthropic identity and no slot/swap
semantics; they install additively (wrapper binary on PATH + separate
Desktop profile) and never touch the first-party slots.
Do not add new top-level nouns without explicit discussion — agent
is the precedent for what that discussion must produce: a recorded
design doc, cardinality rules, and a store boundary in core.

**Domain nouns vs tool surfaces.** The five nouns describe the
user's mental model. Read-only introspection surfaces (Sessions,
Config) are presentation layers over those nouns and CC's
filesystem. They do not appear in `claudepot-core` as domain types.

**Behavior surfaces.** Auto-rotation (Settings → Rotation) is a
*behavior* over the existing nouns, not a new noun. A rule
references accounts by email and acts via the existing `cli`
slot's swap primitive. The pure rule engine lives at
`claudepot-core::rotation` (rules / store / audit / eval); the
runtime bridge is `src-tauri/src/rotation_orchestrator.rs`. New
behaviors of this shape (rules over existing nouns, evaluated on
snapshot data, dispatched via existing primitives) belong in
`claudepot-core` as siblings of `rotation`, not as new domain
modules.

**Tool-orchestration boundary.** For *interactive* sessions,
Claudepot observes and controls existing CC processes; it does not
spawn them or orchestrate parallel sessions across branches. The one
sanctioned spawn path is the **agent** noun: scheduled headless
`claude -p` dispatch through per-OS scheduler artifacts — batch
runs with a recorded result, not interactive sessions. The
interactive-orchestration space is filled by
[`Bendzae/claude-manager`](https://github.com/Bendzae/claude-manager) —
a tmux-based TUI that creates one CC instance per
(project, branch, session) triple. Claudepot deliberately stops at
the read/control boundary: PR badges, live status dots, and similar
list-level signals are *observability* over existing CC state, not
workflow primitives. Proposals to add interactive session spawning,
per-session worktrees, or a Task domain noun should be redirected to
claude-manager rather than ported into Claudepot — the five nouns
and the "behaviors over existing nouns" pattern (see `rotation` for
the canonical example) are what keep Claudepot lean.

## Crate separation

- `claudepot-core` — pure Rust library. NO Tauri dependency. All
  business logic lives here. Must be testable without a webview.
- `claudepot-cli` — thin clap wrapper. Calls core, formats output.
  No business logic. No HTTP calls. No keychain operations.
- `src-tauri` — Tauri app. Calls the same core functions as CLI.

If you're writing business logic in `claudepot-cli` or `src-tauri`,
stop — it belongs in `claudepot-core`.

## Keychain surfaces on macOS

1. `/usr/bin/security` subprocess — for CC's `Claude Code-credentials`
   Keychain item. NEVER use `keyring` or `SecItem*` for this.
   See `cli_backend/keychain.rs`.
2. `/usr/bin/security` subprocess (verified-write pattern) — for
   Claudepot's own secrets that BOTH the differently-signed GUI
   `.app` and `claudepot` CLI binaries must read: account credential
   blobs (`cli_backend/storage.rs`) and the GitHub PAT
   (`session_export_delivery.rs`). The `keyring` crate is wrong for
   these slots for two reasons: (a) its SecItem-based write silently
   succeeds into an ephemeral per-app keychain on Developer ID-signed
   binaries without a provisioning profile, and (b) SecItemAdd items
   are ACL-locked to the creating executable, so a GUI-written item
   prompts or fails when the CLI reads it. The pattern: write via
   `security -i` (secret hex-encoded over stdin, never argv), then
   verify by reading the item back — `security -i` exiting 0 is not
   proof the item landed.
3. `keyring` crate — only for Claudepot-owned slots without the
   cross-binary requirement (route secrets,
   `routes/keychain.rs`), and for all Claudepot-owned slots on
   Linux/Windows (Secret Service / Credential Manager are per-user,
   not per-binary — the GitHub PAT slot uses `keyring` there,
   cfg-gated `not(target_os = "macos")`).

Any PR that uses `keyring` to touch `Claude Code-credentials` must
be rejected. A new macOS secret slot that both the GUI and the CLI
read must use the `/usr/bin/security` verified-write pattern, not
`keyring`.

## Account resolution

Email prefix matching. One rule: find all registered emails where
input is a prefix. Exactly one match → use it. Zero or multiple → error.

No fuzzy matching, no edit distance, no aliases, no labels.

## IPC trust + secret direction

Tauri 2 IPC is in-process — JS bridge is **not** a cross-trust
boundary. Real exposure surfaces are DevTools, JS exception
serialization, Rust-side logging, and toast paths that interpolate
errors. The renderer is our own code; CSP keeps third-party JS out.

Direction matters. Secrets entering Rust via paste (e.g. `key_*_add`,
`settings_github_token_set`) are acceptable — the user typed them.
Secrets *returning* over IPC (the old `key_*_copy → string` shape)
are not — they sit in the JS heap waiting for a DevTools snapshot.

Rules:

- `key_*_copy` / `key_oauth_copy_shell` write the OS clipboard
  Rust-side via `tauri-plugin-clipboard-manager` and return only a
  `KeyCopyReceiptDto` (label + preview + clear deadline).
- `key_*_add` / `settings_github_token_set` accept the secret as
  an IPC arg and **zeroize** the local `String` (and any owned
  copies) on every exit path — success and error alike. Use the
  `zeroize` crate; `Drop` alone does not scrub.
- Errors must not interpolate the secret. `KeyError` Display impl
  is audited for this; new error types touching secrets must follow.
- Renderer-side `setToken("")` runs in a `finally` block on the
  Add modals so React state doesn't outlive the single bridge call.

## Clipboard plugin scope

`tauri-plugin-clipboard-manager` is enabled with three permissions in
`capabilities/default.json`:

- `clipboard-manager:allow-write-text` — writes from `key_*_copy*`.
- `clipboard-manager:allow-read-text` — readback for the 30s
  self-clear gate (only clear if the clipboard still holds our
  payload).
- `clipboard-manager:allow-clear` — the actual self-clear.

These permissions are renderer-callable in principle, but the
renderer is our own code — no third-party JS reaches them under our
CSP. New surfaces that touch the clipboard must go through Rust;
adding a `__TAURI_INVOKE__('plugin:clipboard-manager|…')` call from
JS is a review finding.

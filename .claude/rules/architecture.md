---
description: Core architecture constraints for all Claudepot work
globs: "**/*.rs"
---

# Architecture

## Domain model — four nouns

- **account** — a registered Anthropic identity. Email IS the name.
- **cli** — the single slot CC reads credentials from.
- **desktop** — the single slot Claude Desktop reads session files from.
- **project** — a CC project session directory. Available in both CLI and GUI.

CLI and Desktop are independent. Never couple them.

Credential, Profile, Usage are internal — not user-facing nouns.
Do not add new top-level nouns without explicit discussion.

**Domain nouns vs tool surfaces.** The four nouns (account, cli,
desktop, project) describe the user's mental model. Read-only
introspection surfaces (Sessions, Config) are presentation layers
over those nouns and CC's filesystem. They do not appear in
`claudepot-core` as domain types.

## Crate separation

- `claudepot-core` — pure Rust library. NO Tauri dependency. All
  business logic lives here. Must be testable without a webview.
- `claudepot-cli` — thin clap wrapper. Calls core, formats output.
  No business logic. No HTTP calls. No keychain operations.
- `src-tauri` — Tauri app. Calls the same core functions as CLI.

If you're writing business logic in `claudepot-cli` or `src-tauri`,
stop — it belongs in `claudepot-core`.

## Two Keychain surfaces on macOS

1. `keyring` crate — for Claudepot's OWN secrets (stored credentials).
   Same code-signing identity reads and writes. Safe.
2. `/usr/bin/security` subprocess — for CC's `Claude Code-credentials`
   Keychain item. NEVER use `keyring` or `SecItem*` for this.
   See `cli_backend/keychain.rs`.

Any PR that uses `keyring` to touch `Claude Code-credentials`
must be rejected.

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

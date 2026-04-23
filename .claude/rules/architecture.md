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

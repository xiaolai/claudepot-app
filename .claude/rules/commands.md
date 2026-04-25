---
description: CLI command conventions
globs: "crates/claudepot-cli/**/*.rs"
---

# CLI command conventions

## Every command handler:

1. Lives under `cli/commands/<noun>.rs` for nouns with ≤2 verbs.
   Nouns with ≥3 verbs use a `cli/commands/<noun>/` directory plus a
   `cli/commands/<noun>.rs` entry file that holds shared formatters,
   types, gate helpers, the submodule declarations, and the
   `pub use` re-exports `main.rs`'s match block depends on. Inside
   the directory, organize verbs whichever way reads cleanest:

   - **One verb per file** — preferred when verbs are independent
     (`commands/account/{add,list,remove,verify,login}.rs`).
   - **Verb-group per file** — preferred when several verbs share
     state, helpers, or a sub-domain inside the noun. The session
     module is the canonical example: `orphan.rs` (list-orphans /
     move / adopt-orphan / rebuild-index — all about transcripts'
     project-association lifecycle), `inspect.rs` (view + chunk /
     summary printers), `search.rs` (search + worktrees), `prune.rs`,
     `trash.rs`, `slim.rs`. Splitting these into one-per-file would
     fragment closely-related code without buying clarity.

   Submodules access the entry file's private helpers via
   `use super::*;` — Rust's privacy is outward-not-upward, so
   children reach the parent's private items without `pub(super)`.
2. Takes parsed clap args + shared `AppContext` (store, http client, platform)
3. Returns `anyhow::Result<()>`
4. Calls ONLY `claudepot-core` functions — no direct I/O
5. Uses `output.rs` helpers for human vs `--json` formatting

## Output format:

- Human output goes to stdout. Errors go to stderr.
- `--json` outputs a single JSON object or array to stdout.
- `--quiet` suppresses progress messages, outputs only the result.
- Progress/status messages use `eprintln!` (not `println!`).

## Adding a new command:

1. Add the verb to the clap enum in `main.rs`
2. Create or update the handler at `cli/commands/<noun>.rs`,
   or `cli/commands/<noun>/<verb>.rs` if the noun is already split
   (or the addition pushes it past 2 verbs)
3. Wire the handler in the `match` block in `main.rs`
4. Update `dev-docs/implementation-plan.md` if the command
   wasn't planned

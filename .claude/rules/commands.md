---
description: CLI command conventions
globs: "crates/claudepot-cli/**/*.rs"
---

# CLI command conventions

## Every command handler:

1. Lives in `cli/commands/<noun>.rs`
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
2. Create or update the handler in `cli/commands/<noun>.rs`
3. Wire the handler in the `match` block in `main.rs`
4. Update `dev-docs/implementation-plan.md` if the command
   wasn't planned

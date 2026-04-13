# Claudepot

Multi-account Claude Code / Claude Desktop switcher. Tauri 2 + Rust + React.

## Build

```bash
cargo check --workspace              # Rust
cargo build -p claudepot-cli         # CLI binary
pnpm build                           # Frontend bundle
pnpm tauri dev                       # GUI in dev mode (hot reload)
pnpm tauri build --no-bundle         # GUI release binary (no .dmg)
```

## GUI (Tauri)

- `src-tauri/src/commands.rs` — async Tauri commands wrapping `claudepot-core`. NO business logic.
- `src-tauri/src/dto.rs` — serde DTOs crossing to JS. Credentials never cross.
- `src/App.tsx` + `src/api.ts` + `src/types.ts` — React UI, plain CSS.
- `AccountStore.db` is `Mutex<Connection>` so stores can cross `await` points in Tauri commands.

## Test on test-host

```bash
cargo build -p claudepot-cli
scp target/debug/claudepot joker@192.0.2.1:/tmp/claudepot
ssh joker@192.0.2.1 "security unlock-keychain -p xiaolai ~/Library/Keychains/login.keychain-db; /tmp/claudepot <command>"
```

Automated login for setting up CC state on test-host:
```bash
ssh joker@192.0.2.1 "security unlock-keychain -p xiaolai; bash /tmp/claude-login-local.sh <email>"
```

## Architecture

See `dev-docs/implementation-plan.md` for the full plan.

- Three nouns: **account**, **cli**, **desktop**
- `claudepot-core` = pure Rust library, no Tauri dependency
- `claudepot-cli` = thin clap wrapper over core
- `src-tauri` = Tauri app consuming same core
- Two separate keychain surfaces on macOS (see rules/architecture.md)
- Account identity = email, resolved by prefix matching

## Reference

`dev-docs/kannon/reference.md` — 3400-line verified reference for CC/Desktop internals.
Always verify claims against CC source at `~/github/claude_code_src/src` before coding.

## Conventions

- Grill reports go in `dev-docs/reports/`. Never drop them at the repo root.

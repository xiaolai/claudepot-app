# Getting started

::: warning Status
Alpha (`0.0.15`). Daily-driven on macOS. Windows and Linux builds are green but less seasoned. Pre-built installers are coming — until then, it's a source build.
:::

## Requirements

- A recent **Rust toolchain** ([rustup.rs](https://rustup.rs)).
- **Node 20+** with **pnpm** ([pnpm.io](https://pnpm.io)).

That's it. No other system dependencies.

## Install

```bash
git clone https://github.com/xiaolai/claudepot-app.git
cd claudepot-app
```

### Desktop app

```bash
pnpm install
pnpm tauri build --no-bundle      # builds the binary, no installer
```

For a hot-reloading dev session:

```bash
pnpm tauri dev
```

### Command-line tool

```bash
cargo build -p claudepot-cli --release
# Built binary: ./target/release/claudepot
```

Add it to your `PATH` (or copy it somewhere on `PATH`):

```bash
cp target/release/claudepot /usr/local/bin/
```

## Where your data lives

`~/.claudepot/` — your registered accounts and the session index. Override with the `CLAUDEPOT_DATA_DIR` environment variable if you want it somewhere else.

Your Claude Code data (`~/.claude/`) is read by ClauDepot but stays where Claude put it. ClauDepot never moves it.

## Next

- [First run](./first-run) — what to do the first time you open the app.

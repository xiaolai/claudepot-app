# `panel/` — the remote panel

The phone client served by `claudepot remote serve` at `/`. Built by
Vite into `crates/claudepot-core/src/remote/assets/panel/`, which is
**committed**, so `cargo build` needs no Node.

```bash
cd panel && pnpm install && pnpm build     # or: scripts/build-panel.sh
```

## Why this is its own install and not a workspace member

The Tauri renderer at `../src/` carries 328 `invoke` calls. Every one of
them means nothing over HTTP, and a shared install would put
`@tauri-apps/api` one auto-import away from a file that runs in Safari on
a phone. The two apps share a design language and no code.

## Layout

| Path | Role |
|---|---|
| `src/vendor/` | The delivered design system, **byte-identical**: `ds-tokens.css`, `ds-icons.jsx`, `ds-kit.jsx`. They publish onto `window` and read `React` from it. |
| `src/globals.js` | Puts `React` on `window` before the vendor files load. Imported first — see its comment. |
| `src/fonts.css` + `src/fonts/` | Self-hosted Instrument Sans / Serif and JetBrains Mono, latin + latin-ext. |
| `src/app/` | Everything wired to the host: `api.js`, the five screens, `webauthn.js`, `format.js`. |

The delivered `panel-home.jsx` / `panel-screens.jsx` / `mobile2-thread.jsx`
are **not** vendored. They are written against mock shapes the host
cannot fill (per-account usage windows, a device list, notification
transport), and wiring components to data that does not exist is how a
screen ends up rendering a constant. The app layer here reuses the design
system and is written against the real endpoints.

## Rules that are not obvious

- **Fixed output filenames.** `assets.rs` resolves request paths with an
  exhaustive `match` over string literals. A content hash would mean
  editing Rust on every build, and a stale literal would 404 the app's
  own bundle on a device rather than at build time.
- **Nothing off-origin.** The server's CSP is `default-src 'self'`, and
  an appliance on a tailnet has no route to a CDN anyway. A test in
  `remote::assets` asserts the built bundle contains no remote origin,
  with a named exemption list for the XML namespaces and React's
  error-doc pointer.
- **Rebuild after touching anything here.** The committed output is the
  artifact; a source change that is not rebuilt ships the old bundle.
- **A rebuild also regenerates Rust.** `scripts/build-panel.sh` writes
  `crates/claudepot-core/src/remote/assets/panel_chunks.rs` — one
  `include_bytes!` arm per emitted chunk. Mermaid splits into ~60 of
  them, so the table is generated rather than hand-kept, and two tests in
  `remote::assets` fail if it and the directory disagree in either
  direction. Commit both.

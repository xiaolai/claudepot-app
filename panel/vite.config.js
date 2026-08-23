import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// The panel builds straight into `remote::assets`' embed directory.
//
// Fixed filenames, not content hashes. `assets.rs` resolves a request
// path with an exhaustive `match` over string literals, so a hashed
// name would mean editing Rust on every rebuild — and a stale literal
// would 404 the app's own bundle on a phone rather than at build time.
// Cache-busting is not needed: every asset is served `no-store` except
// the fonts, whose bytes never change under a given name.
//
// `emptyOutDir` is off. The output directory is inside the Rust crate
// and holds `.gitkeep`-adjacent siblings (the probe's own assets sit one
// level up); wiping it is a bigger hammer than this build needs.
const OUT = '../crates/claudepot-core/src/remote/assets/panel';

// Where `vite dev` forwards `/api` — a running `claudepot remote serve`.
//
// From the environment, never hardcoded: a real deployment binds a
// tailnet or LAN address, and committing one would put somebody's
// private topology in a public repo. Loopback is the default because it
// is the only address that means the same thing on every machine.
//
//   CLAUDEPOT_API=https://<host>:8420 pnpm dev
const API = process.env.CLAUDEPOT_API || 'http://127.0.0.1:8420';

export default defineConfig({
  plugins: [react()],
  // Dev only — `vite build` ignores this block, so nothing here reaches
  // the shipped bundle.
  server: {
    proxy: {
      '/api': {
        target: API,
        // Host becomes the target's, which `guard_origin` checks
        // against `allowed_hosts`.
        changeOrigin: true,
        // The appliance's certificate comes from a private CA node has
        // no reason to trust. This is the developer's machine talking
        // to their own server.
        secure: false,
        configure: (proxy) => {
          proxy.on('proxyReq', (proxyReq) => {
            // `guard_origin` refuses a PRESENT Origin that does not
            // match Host, and allows an absent one. The browser sends
            // `http://localhost:5173`, which can never match — so drop
            // it rather than forge it. Dropping is the honest form:
            // this request genuinely did not come from a page on the
            // appliance's own origin.
            proxyReq.removeHeader('origin');
          });
        },
      },
    },
  },
  // Assets are referenced as `/panel/…` because that is where the
  // server serves them from. A relative base would break the moment the
  // app is opened at a nested path.
  base: '/panel/',
  build: {
    outDir: OUT,
    emptyOutDir: false,
    // The panel is loaded over a LAN by a phone. One request per asset
    // kind beats a hashed-chunk graph, and there is no code-splitting to
    // gain from: every screen is on the first paint path.
    modulePreload: false,
    rollupOptions: {
      output: {
        entryFileNames: 'panel.js',
        // No content hash. `assets.rs` matches literal paths, and every
        // asset but the fonts is served `no-store`, so a hash would only
        // churn the generated route table on each build.
        chunkFileNames: 'chunks/[name].js',
        assetFileNames: (info) => {
          // Rollup 4 renamed `name` to `names`; read both so a minor
          // bump does not silently rename every font.
          const name = (info.names && info.names[0]) || info.name || '';
          return name.endsWith('.css') ? 'panel.css' : 'fonts/[name][extname]';
        },
      },
    },
  },
});

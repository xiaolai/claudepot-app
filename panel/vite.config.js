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

export default defineConfig({
  plugins: [react()],
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

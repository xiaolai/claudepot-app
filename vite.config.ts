import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 11220,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 11221,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri` and cargo's
      // build output — on Windows, watching `target/` while cargo
      // is writing .o files causes EBUSY FSWatcher crashes.
      ignored: ["**/src-tauri/**", "**/target/**"],
    },
    // Pre-bundle these in dev so the webview doesn't fire 50+ separate
    // module requests to cold-compile React + Tauri wrappers on first
    // load. Saves multiple seconds on the `pnpm tauri dev` cold start.
    warmup: {
      clientFiles: [
        "./src/main.tsx",
        "./src/App.tsx",
        "./src/sections/AccountsSection.tsx",
      ],
    },
  },
  optimizeDeps: {
    include: [
      "react",
      "react-dom",
      "react-dom/client",
      "@tauri-apps/api/core",
      "@tauri-apps/api/event",
    ],
  },
  build: {
    // Chromium target — Tauri's webview always supports it, so we skip
    // the legacy polyfills Vite would otherwise ship.
    target: "es2022",
    minify: "esbuild",
    cssMinify: true,
    sourcemap: false,
    rollupOptions: {
      // A circular *chunk* cycle ships a prod bundle that throws
      // "Cannot access X before initialization" at module-eval — React
      // never mounts and the app white-screens on the boot splash
      // (this shipped as v0.1.47). It only manifests in the minified,
      // code-split production bundle, so dev / tsc / vitest all pass.
      // Treat the warning as FATAL so `pnpm build` (and thus CI + the
      // release pipeline) fails here instead of in users' webviews.
      onwarn(warning, defaultHandler) {
        if (
          warning.code === "CYCLIC_CROSS_CHUNK_REEXPORT" ||
          (warning.message && warning.message.includes("Circular chunk"))
        ) {
          throw new Error(`Fatal build warning (circular chunk): ${warning.message}`);
        }
        defaultHandler(warning);
      },
      output: {
        // VENDOR chunks only. App section code is split automatically
        // by the `React.lazy(() => import(...))` loaders in the section
        // registry — those produce acyclic async chunks per section.
        //
        // We deliberately do NOT manual-group section *source* by path
        // anymore: doing so forced eagerly-imported helpers (e.g.
        // App.tsx's static import of `sections/projects/sessionMoveProgress`)
        // into a named `section-*` chunk, and once sections began
        // cross-importing (config↔projects↔sessions) those named chunks
        // became mutually circular. Rollup emitted a `Circular chunk`
        // warning and the circular init order threw "Cannot access X
        // before initialization" at module-eval — so the production
        // bundle white-screened on the boot splash while dev (unbundled)
        // and the test suite (no chunking) were fine. Vendor-only manual
        // chunks + lazy() auto-splitting keep the shell-paints-first
        // perf win without the cycle.
        manualChunks(id: string) {
          // Config preview pulls react-markdown + remark-gfm +
          // rehype-highlight + highlight.js grammars (~150 KB
          // gzipped). Isolate the vendor family in its own chunk so it
          // loads lazily with the (auto-split) Config section instead
          // of leaking into the main bundle. Match BEFORE the react
          // rule so `react-markdown` isn't swept into `react` by the
          // `node_modules/react` substring. Covers the full unified
          // ecosystem (unified, vfile, hast/mdast, micromark, link
          // attribute helpers, devlop) so transitive deps stay grouped.
          if (id.includes("node_modules/react-markdown") ||
              id.includes("node_modules/remark-") ||
              id.includes("node_modules/rehype-") ||
              id.includes("node_modules/micromark") ||
              id.includes("node_modules/mdast-") ||
              id.includes("node_modules/hast-") ||
              id.includes("node_modules/unist-") ||
              id.includes("node_modules/unified") ||
              id.includes("node_modules/vfile") ||
              id.includes("node_modules/devlop") ||
              id.includes("node_modules/property-information") ||
              id.includes("node_modules/html-url-attributes") ||
              id.includes("node_modules/space-separated-tokens") ||
              id.includes("node_modules/comma-separated-tokens") ||
              id.includes("node_modules/decode-named-character-reference") ||
              id.includes("node_modules/character-entities") ||
              id.includes("node_modules/trim-lines") ||
              id.includes("node_modules/zwitch") ||
              id.includes("node_modules/bail") ||
              id.includes("node_modules/is-plain-obj") ||
              id.includes("node_modules/extend") ||
              id.includes("node_modules/ccount") ||
              id.includes("node_modules/longest-streak") ||
              id.includes("node_modules/markdown-table") ||
              id.includes("node_modules/escape-string-regexp") ||
              id.includes("node_modules/lowlight") ||
              id.includes("node_modules/fault") ||
              id.includes("node_modules/highlight.js")) {
            return "markdown-vendor";
          }
          if (id.includes("node_modules/react/") ||
              id.includes("node_modules/react-dom/") ||
              id.includes("node_modules/scheduler/")) {
            return "react";
          }
          return undefined;
        },
      },
    },
  },
}));

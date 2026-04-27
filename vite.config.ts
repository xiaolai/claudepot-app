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
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1431,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
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
      output: {
        // Split React + the per-section trees into their own chunks
        // so the shell can paint before Projects / Sessions / Settings
        // parse. Without this, everything ends up in a single 384 KB
        // main bundle that the webview has to parse before React
        // mounts.
        manualChunks(id: string) {
          // Config preview pulls react-markdown + remark-gfm +
          // rehype-highlight + highlight.js grammars (~150 KB
          // gzipped). Keep them isolated so the rest of the app
          // doesn't pay the parse cost until the user opens
          // Config. Match these BEFORE the react chunk rule so
          // `react-markdown` isn't swept into `react` by the
          // `node_modules/react` substring match. The pattern
          // covers the full unified-ecosystem family (unified,
          // vfile, hast/mdast utilities, micromark grammars, link
          // attribute helpers, devlop) so transitive deps land in
          // the same lazy chunk instead of leaking into the main
          // bundle.
          if (id.includes("/sections/config/") ||
              id.includes("/sections/ConfigSection") ||
              id.includes("node_modules/react-markdown") ||
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
            return "section-config";
          }
          if (id.includes("node_modules/react/") ||
              id.includes("node_modules/react-dom/") ||
              id.includes("node_modules/scheduler/")) {
            return "react";
          }
          if (id.includes("/sections/projects/") ||
              id.includes("/sections/ProjectsSection")) {
            return "section-projects";
          }
          if (id.includes("/sections/sessions/") ||
              id.includes("/sections/SessionsSection")) {
            return "section-sessions";
          }
          if (id.includes("/sections/settings/") ||
              id.includes("/sections/SettingsSection")) {
            return "section-settings";
          }
          return undefined;
        },
      },
    },
  },
}));

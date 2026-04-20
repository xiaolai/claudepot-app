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
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
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
          if (id.includes("node_modules/react") ||
              id.includes("node_modules/react-dom") ||
              id.includes("node_modules/scheduler")) {
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

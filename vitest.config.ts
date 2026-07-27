import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    environmentOptions: {
      jsdom: {
        // Node 26 disables jsdom's default storage when no origin is
        // configured. Keep browser persistence deterministic across Node
        // versions and test workers.
        url: "http://localhost",
      },
    },
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    // Vitest's 5000 ms default was never calibrated for this suite's
    // heaviest files. `App.test.tsx` calls `vi.resetModules()` in
    // `beforeEach` and re-imports the ENTIRE App module graph per test
    // — mandatory, because `vi.doMock` only applies to a module loaded
    // after it. Measured under a full 116-file parallel run, its
    // slowest test takes ~2.7 s, leaving under 2x headroom; unlucky
    // worker scheduling pushed it (and occasionally
    // `AgentForm.test.tsx`) past 5 s and failed the suite
    // non-deterministically — 1 failure in some runs, 3 in others.
    //
    // The failure was also self-amplifying: a timed-out worker stalls
    // the run, so a full pass at 60 s finished in 34 s while the same
    // suite with timeouts took 78-85 s.
    //
    // 15 s is ~5x the measured worst case — still fails fast on a
    // genuine hang, which is what a timeout is for. If a test legitimately
    // needs longer than this, make that test cheaper rather than raising
    // this number.
    testTimeout: 15_000,
    coverage: {
      provider: "v8",
      reporter: ["text", "html"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.test.{ts,tsx}", "src/test/**", "src/main.tsx"],
    },
  },
});

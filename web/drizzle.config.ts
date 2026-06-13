import { config as loadEnv } from "dotenv";
import { defineConfig } from "drizzle-kit";

// Drizzle-kit's built-in dotenv only loads `.env`. Load `.env.local`
// explicitly so it picks up the v1 secrets we pulled from Vercel.
loadEnv({ path: ".env.local" });
loadEnv(); // fallback to .env for shared defaults

/**
 * Connection-string resolution is deliberately prod-hostile.
 *
 * On 2026-05-06 a `drizzle-kit push` ran against prod (the config used
 * to fall back to NEON_DATABASE_URL from .env.local) and dropped the
 * submissions.search_vec FTS column + its GIN index — see
 * .claude/rules/db-migrations.md. Prose rules didn't prevent it, so
 * this config now refuses the prod URL structurally:
 *
 *   1. DRIZZLE_DATABASE_URL or TEST_DATABASE_URL (the variable
 *      tests/integration/README.md documents) — explicit, non-prod
 *      target. Never set either in .env.local.
 *   2. DRIZZLE_ALLOW_PROD=1 — explicit escape hatch that opts into the
 *      prod DATABASE_URL/NEON_DATABASE_URL. Read-only commands
 *      (studio, introspect) only; `push` against prod stays forbidden
 *      by db-migrations.md — apply hand-written migrations via
 *      scripts/apply-migration.ts instead.
 *   3. Otherwise: a sentinel URL pointing at a closed local port.
 *      `drizzle-kit generate` never connects, so schema generation
 *      keeps working with zero env; any command that does connect
 *      (push, migrate, studio) fails fast with connection-refused
 *      instead of silently targeting prod.
 */
const explicitUrl =
  process.env.DRIZZLE_DATABASE_URL ?? process.env.TEST_DATABASE_URL;
const prodUrl = process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;

const SENTINEL_URL =
  "postgresql://drizzle-guard@localhost:1/set_DRIZZLE_DATABASE_URL_or_DRIZZLE_ALLOW_PROD";

let connectionString: string;
if (explicitUrl) {
  connectionString = explicitUrl;
} else if (process.env.DRIZZLE_ALLOW_PROD === "1") {
  if (!prodUrl) {
    throw new Error(
      "drizzle.config.ts: DRIZZLE_ALLOW_PROD=1 set but no DATABASE_URL / NEON_DATABASE_URL found.",
    );
  }
  console.warn(
    "drizzle.config.ts: DRIZZLE_ALLOW_PROD=1 — targeting the PRODUCTION database. " +
      "`push` against prod is forbidden (see .claude/rules/db-migrations.md).",
  );
  connectionString = prodUrl;
} else {
  console.warn(
    "drizzle.config.ts: no DRIZZLE_DATABASE_URL / TEST_DATABASE_URL set — " +
      "using a sentinel URL. `generate` works; commands that connect will " +
      "fail fast instead of touching prod.",
  );
  connectionString = SENTINEL_URL;
}

export default defineConfig({
  schema: "./src/db/schema/*.ts",
  out: "./src/db/migrations",
  dialect: "postgresql",
  dbCredentials: { url: connectionString },
  /**
   * Enable verbose logging during dev so generated SQL is reviewable
   * before any push. Strict mode prevents accidental destructive
   * operations from being applied without confirmation.
   */
  verbose: true,
  strict: true,
});

import { config as loadEnv } from "dotenv";
import { defineConfig } from "drizzle-kit";

// Drizzle-kit's built-in dotenv only loads `.env`. Load `.env.local`
// explicitly so it picks up the v1 secrets we pulled from Vercel.
loadEnv({ path: ".env.local" });
loadEnv(); // fallback to .env for shared defaults

const connectionString =
  process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;

if (!connectionString) {
  throw new Error(
    "drizzle.config.ts: missing DATABASE_URL (or NEON_DATABASE_URL).",
  );
}

export default defineConfig({
  schema: "./src/db/schema.ts",
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

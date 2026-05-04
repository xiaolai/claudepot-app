/**
 * One-shot SQL applier for hand-written migrations.
 *   pnpm exec tsx --env-file=.env.local scripts/apply-migration.ts <file.sql>
 * Splits on `--> statement-breakpoint` to support drizzle-kit-style files.
 */

import { readFileSync } from "node:fs";
import { neon } from "@neondatabase/serverless";

const file = process.argv[2];
if (!file) {
  console.error("usage: apply-migration.ts <path/to/file.sql>");
  process.exit(1);
}

const url = process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;
if (!url) {
  console.error("missing DATABASE_URL / NEON_DATABASE_URL");
  process.exit(1);
}

const sql = neon(url);
const raw = readFileSync(file, "utf8");
const statements = raw
  .split(/-->\s*statement-breakpoint/g)
  .map((s) => s.trim())
  .filter((s) => s.length > 0 && !/^(--.*)?$/.test(s));

for (const stmt of statements) {
  console.log(`applying:\n${stmt}\n`);
  await sql.query(stmt);
}
console.log(`done — ${statements.length} statement(s) applied`);

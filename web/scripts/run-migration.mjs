// One-shot migration runner via @neondatabase/serverless HTTP driver.
// Usage: node --env-file=.env.local scripts/run-migration.mjs <path>
//
// Splits on drizzle's `--> statement-breakpoint` marker and runs
// each statement separately. Reports which already-applied
// statements no-op'd vs which actually ran.

import { neon } from "@neondatabase/serverless";
import { readFileSync } from "node:fs";

const url = process.env.NEON_DATABASE_URL;
if (!url) {
  console.error("NEON_DATABASE_URL missing");
  process.exit(1);
}

const path = process.argv[2];
if (!path) {
  console.error("usage: run-migration.mjs <path-to-sql>");
  process.exit(1);
}

const sql = neon(url);

// Identify which DB we're hitting before changing anything.
const [{ host_db }] = await sql`SELECT current_database() AS host_db`;
console.log(`Target DB: ${host_db}`);

const file = readFileSync(path, "utf8");
const statements = file
  .split("--> statement-breakpoint")
  .map((s) => s.replace(/^\s*--.*$/gm, "").trim())
  .filter((s) => s.length > 0);

console.log(`Running ${statements.length} statements from ${path}...`);

for (let i = 0; i < statements.length; i++) {
  const stmt = statements[i];
  const head = stmt.split("\n")[0].slice(0, 80);
  try {
    await sql.query(stmt);
    console.log(`  [${i + 1}/${statements.length}] OK   ${head}`);
  } catch (err) {
    console.error(`  [${i + 1}/${statements.length}] FAIL ${head}`);
    console.error(`         ${err.message}`);
    process.exit(1);
  }
}

console.log("Migration applied.");

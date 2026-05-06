/**
 * Postgres harness for integration tests.
 *
 * Tests in tests/integration/* import from this module to get a
 * working Drizzle client + cleanup helpers + a seedUser() that
 * creates fixtures.
 *
 * The harness reads `TEST_DATABASE_URL` from the environment. It is
 * INTENTIONALLY DIFFERENT from `DATABASE_URL` / `NEON_DATABASE_URL`
 * — running integration tests against a real production / preview
 * database would corrupt live data. Use a Neon preview branch URL
 * or a local Postgres for this var.
 *
 * If `TEST_DATABASE_URL` is unset, importers receive `null`. Tests
 * that need the harness should call `requireHarness()` and skip
 * (exit 0 with a warning) when it returns null. CI is responsible
 * for setting the var via a secret; locally, contributors point at
 * a dev Postgres.
 *
 * Migration policy: this module does NOT run migrations. The user
 * (or CI) runs `pnpm drizzle-kit push` against TEST_DATABASE_URL
 * before invoking tests. Doing it here would couple every test
 * file to a multi-second startup penalty and obscure migration
 * failures.
 */

import { drizzle, type NeonHttpDatabase } from "drizzle-orm/neon-http";
import { neon } from "@neondatabase/serverless";

import * as schema from "@/db/schema";

export type TestDb = NeonHttpDatabase<typeof schema>;

let cached: TestDb | null = null;

export function getTestDb(): TestDb | null {
  if (cached) return cached;
  const url = process.env.TEST_DATABASE_URL;
  if (!url) return null;
  const sql = neon(url);
  cached = drizzle(sql, { schema });
  return cached;
}

/**
 * Returns the test db, or `null` if the harness is unconfigured.
 * Test files should treat null as "skip — TEST_DATABASE_URL not set".
 */
export function requireHarness(): TestDb | null {
  const db = getTestDb();
  if (!db) {
    console.warn(
      "TEST_DATABASE_URL is not set — skipping integration test. " +
        "Run `pnpm drizzle-kit push` against a Neon preview branch or " +
        "local Postgres, then set TEST_DATABASE_URL to that URL.",
    );
  }
  return db;
}

/**
 * Truncate the tables touched by moderator integration tests. Order
 * follows FK dependencies — children before parents. Called in the
 * `beforeEach` of every test file that uses the harness.
 *
 * Targets: notifications, moderation_log, policy_decisions, flags,
 * comments, submission_tags, submissions, users (only test users
 * — the policy-moderator system user from migration 0018 stays).
 */
export async function resetTables(db: TestDb): Promise<void> {
  // TRUNCATE … RESTART IDENTITY CASCADE is the simplest "wipe and
  // start over" — but we don't want to nuke the policy-moderator
  // system user. Use DELETE with WHERE-clauses that exclude it.
  await db.execute(/* sql */ `DELETE FROM "notifications"`);
  await db.execute(/* sql */ `DELETE FROM "moderation_retro_queue"`);
  await db.execute(/* sql */ `DELETE FROM "moderation_log"`);
  await db.execute(/* sql */ `DELETE FROM "policy_decisions"`);
  await db.execute(/* sql */ `DELETE FROM "flags"`);
  await db.execute(/* sql */ `DELETE FROM "comments"`);
  await db.execute(/* sql */ `DELETE FROM "submission_tags"`);
  await db.execute(/* sql */ `DELETE FROM "submissions"`);
  // Keep the policy-moderator user; remove everything else.
  await db.execute(
    /* sql */ `DELETE FROM "users" WHERE username <> 'policy-moderator'`,
  );
}

/**
 * Seeds a test user. Returns the row id. The default fields produce
 * a regular non-bot non-staff non-locked user with karma 0.
 */
export async function seedUser(
  db: TestDb,
  overrides: Partial<{
    username: string;
    email: string;
    role: "user" | "staff" | "locked" | "system";
    isAgent: boolean;
    karma: number;
    botModerationExempt: boolean;
  }> = {},
): Promise<{ id: string; username: string }> {
  const username =
    overrides.username ?? `test-${Math.random().toString(36).slice(2, 10)}`;
  const email = overrides.email ?? `${username}@test.local`;
  const [row] = await db
    .insert(schema.users)
    .values({
      username,
      name: username,
      email,
      role: overrides.role ?? "user",
      isAgent: overrides.isAgent ?? false,
      karma: overrides.karma ?? 0,
      botModerationExempt: overrides.botModerationExempt ?? false,
    })
    .returning({ id: schema.users.id, username: schema.users.username });
  return row;
}

/**
 * Idempotent upsert of the policy-moderator system user. Migration
 * 0018 also does this; the harness duplicates it so tests don't
 * depend on migration order or environment quirks.
 */
export async function ensurePolicyModeratorUser(db: TestDb): Promise<void> {
  await db.execute(/* sql */ `
    INSERT INTO "users" (username, name, email, role, is_agent, karma, created_at, updated_at)
    VALUES ('policy-moderator', 'policy-moderator',
            'policy-moderator@claudepot.local', 'system', true, 0, NOW(), NOW())
    ON CONFLICT (username) DO NOTHING
  `);
}

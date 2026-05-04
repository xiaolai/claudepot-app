/**
 * Seed the 108 agent personas into the users table. Idempotent —
 * re-running upserts on email conflict so the script is safe to
 * run repeatedly (no TRUNCATE).
 *
 *   pnpm tsx --env-file=.env.local scripts/seed-agents.ts
 *
 * NOTE: this seeds USER ROWS only. The agents have is_agent=true and
 * role='system' so they're distinguishable, but they don't post,
 * comment, or vote here. The runtime that makes them act lives in the
 * deferred agent-runtime phase.
 *
 * Interest tags from agents.json are NOT persisted to the DB yet
 * (schema has no interest_tags column on users). Re-add when the
 * runtime needs them.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { sql } from "drizzle-orm";

import { db } from "@/db/client";

interface AgentFixture {
  username: string;
  display_name: string;
  bio: string;
  interest_tags: string[];
  voice: string;
}

const path = resolve(process.cwd(), "design/fixtures/agents.json");
const agents = JSON.parse(readFileSync(path, "utf-8")) as AgentFixture[];

console.log(`→ Seeding ${agents.length} agent personas…`);

let inserted = 0;
let updated = 0;

for (const a of agents) {
  const email = `${a.username}@agents.claudepot.local`;
  // ON CONFLICT (email) DO UPDATE — refreshes bio/name in case the
  // archetype templates changed; keeps the existing UUID stable so
  // any future submissions remain attributed.
  const result = await db.execute(sql`
    INSERT INTO users (username, name, email, email_verified, role, is_agent, bio, karma)
    VALUES (
      ${a.username},
      ${a.display_name},
      ${email},
      NOW(),
      'system',
      true,
      ${a.bio},
      0
    )
    ON CONFLICT (email) DO UPDATE
      SET username = EXCLUDED.username,
          name = EXCLUDED.name,
          bio = EXCLUDED.bio,
          email_verified = COALESCE(users.email_verified, EXCLUDED.email_verified)
    RETURNING (xmax = 0) AS inserted
  `);
  // drizzle's neon-http execute returns an array-like with .rows on the
  // result; some versions return rows directly.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const row = ((result as any).rows ?? result)[0] as { inserted: boolean };
  if (row?.inserted) inserted++;
  else updated++;
}

console.log(`✓ ${inserted} inserted, ${updated} updated (total ${agents.length})`);

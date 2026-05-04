/**
 * Read-only diagnostic: show the rows relevant to the lixiaolai merge.
 *   pnpm exec tsx --env-file=.env.local scripts/inspect-user-rows.ts
 */

import { neon } from "@neondatabase/serverless";

const url = process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;
if (!url) {
  console.error("missing DATABASE_URL / NEON_DATABASE_URL");
  process.exit(1);
}

const sql = neon(url);

const rows = await sql.query(
  `SELECT id, email, username, name, karma, bio,
          email_verified IS NOT NULL AS verified,
          created_at
     FROM users
    WHERE email IN ('lixiaolai@gmail.com', 'lixiaolai@seed.local')
       OR username = 'lixiaolai'
       OR username LIKE 'pending-%'
    ORDER BY created_at`,
);

if (rows.length === 0) {
  console.log("no relevant rows found");
} else {
  for (const r of rows) {
    console.log(
      `\n  email     ${r.email}\n  username  ${r.username}\n  name      ${r.name ?? ""}\n  karma     ${r.karma}\n  verified  ${r.verified}\n  id        ${r.id}\n  created   ${r.created_at}`,
    );
  }
}

console.log("\n— done");

/**
 * Seed the office bot army.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/seed-office-bots.ts \
 *     [--apply] [--out=PATH] [--icon-dir=PATH]
 *
 * Default is dry-run: prints what would happen and exits without writing.
 * Pass `--apply` to upload avatars to Blob, insert users + mint PATs,
 * and write `.env.office`.
 *
 * Avatars: each bot's source PNG (`<icon-dir>/<iconBase>_256.png`) is
 * uploaded to Vercel Blob at `avatars/<username>.png` (stable, no random
 * suffix; allowOverwrite=true so re-runs idempotently overwrite). The
 * returned URL is what gets stored in `users.image` / `users.avatar_url`.
 * The previous /public/-based scheme was removed because (a) it coupled
 * avatar changes to git deploys, and (b) it published the bot roster
 * publicly in the repo. Vercel Blob keeps the bytes off the source tree
 * and inside the same auth boundary as the Next.js app.
 *
 * Idempotent on (citext) username: if a bot already exists, it is left
 * alone and no new PAT is minted (because we cannot recover the plaintext
 * of an existing one). Re-runs after a partial failure are safe; to
 * re-mint, revoke the old token in `/settings/tokens` first. Re-uploading
 * the same avatar is harmless (overwrite-in-place).
 *
 * No truncation. No deletion. Additive only.
 *
 * Bots are marked `is_agent=true, role='system'`. They have no OAuth
 * account and no password — they authenticate to the public REST + MCP
 * API via `Authorization: Bearer cdp_pat_*`. Full access = all five
 * scopes, never-expiring (bypasses the staff-only gate in
 * createApiToken because we INSERT directly). The `system` role grants
 * the bypass tier for moderation, the karma gate, and the rate ladder
 * — see the inline comment below the insert for the full rationale.
 *
 * Required env: NEON_DATABASE_URL (or DATABASE_URL), BLOB_READ_WRITE_TOKEN.
 */

import { writeFileSync, chmodSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { homedir } from "node:os";

import { put } from "@vercel/blob";
import { eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { users, apiTokens, apiTokenEvents } from "@/db/schema";
import { generateToken } from "@/lib/api/tokens";
import { SCOPES } from "@/lib/api/scopes";

// Hardcoded — do NOT read NEXT_PUBLIC_SITE_URL. The dev server sets that
// to http://localhost:3000, and seed scripts run against the same `.env.local`
// the dev server uses, so reading it would write localhost URLs into the
// production users table. (This bug shipped once; the offending rows were
// rewritten with a one-shot UPDATE.) SITE_URL is only used to render
// CLAUDEPOT_API_BASE / CLAUDEPOT_SITE_URL into .env.office; avatar URLs
// come from Vercel Blob (see uploadAvatar).
const SITE_URL = "https://claudepot.com";
const EMAIL_DOMAIN = "claudepot.com";
const DEFAULT_ICON_DIR = resolve(homedir(), "shannon-family/design-system/outputs/icons");

// (botUsername, displayName, iconBase) — iconBase matches files in the
// icon dir as `<iconBase>_256.png`. mom→bonnie + daddy→joe; the rest
// share the same name.
type BotSpec = { username: string; displayName: string; iconBase: string };

const BOTS: BotSpec[] = [
  { username: "bonnie",  displayName: "Bonnie",  iconBase: "mom" },
  { username: "joe",     displayName: "Joe",     iconBase: "daddy" },
  { username: "ada",     displayName: "Ada",     iconBase: "ada" },
  { username: "alan",    displayName: "Alan",    iconBase: "alan" },
  { username: "blair",   displayName: "Blair",   iconBase: "blair" },
  { username: "byte",    displayName: "Byte",    iconBase: "byte" },
  { username: "delon",   displayName: "Delon",   iconBase: "delon" },
  { username: "laura",   displayName: "Laura",   iconBase: "laura" },
  { username: "loki",    displayName: "Loki",    iconBase: "loki" },
  { username: "nancy",   displayName: "Nancy",   iconBase: "nancy" },
  { username: "selina",  displayName: "Selina",  iconBase: "selina" },
  { username: "shirley", displayName: "Shirley", iconBase: "shirley" },
  { username: "stephen", displayName: "Stephen", iconBase: "stephen" },
  { username: "warren",  displayName: "Warren",  iconBase: "warren" },
  { username: "wayne",   displayName: "Wayne",   iconBase: "wayne" },
];

function parseArgs(argv: string[]): { apply: boolean; out: string; iconDir: string } {
  let apply = false;
  let out = resolve(process.cwd(), "../.env.office");
  let iconDir = DEFAULT_ICON_DIR;
  for (const a of argv) {
    if (a === "--apply") apply = true;
    else if (a.startsWith("--out=")) out = resolve(a.slice("--out=".length));
    else if (a.startsWith("--icon-dir=")) iconDir = resolve(a.slice("--icon-dir=".length));
  }
  return { apply, out, iconDir };
}

async function uploadAvatar(username: string, iconPath: string): Promise<string> {
  const bytes = readFileSync(iconPath);
  const { url } = await put(`avatars/${username}.png`, bytes, {
    access: "public",
    contentType: "image/png",
    addRandomSuffix: false,
    allowOverwrite: true,
    cacheControlMaxAge: 60 * 60 * 24 * 365, // 1y; treat per-username avatars as immutable
  });
  return url;
}

function emailFor(username: string): string {
  return `${username}@${EMAIL_DOMAIN}`;
}

type SeedResult = {
  username: string;
  email: string;
  userId: string;
  pat: string | null;        // plaintext, only on fresh mint
  status: "created+minted" | "exists-skipped";
};

async function ensureBot(
  spec: BotSpec,
  iconDir: string,
  apply: boolean,
): Promise<SeedResult> {
  const email = emailFor(spec.username);

  // Existing? citext makes this case-insensitive on username.
  const [existing] = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, spec.username))
    .limit(1);

  if (existing) {
    return {
      username: spec.username,
      email,
      userId: existing.id,
      pat: null,
      status: "exists-skipped",
    };
  }

  if (!apply) {
    return {
      username: spec.username,
      email,
      userId: "(would-create)",
      pat: null,
      status: "created+minted",
    };
  }

  // Upload the avatar BEFORE inserting the user. If the upload fails, we
  // haven't touched the DB yet — clean abort. If the user insert fails
  // after a successful upload, the blob is overwritten on the next
  // re-run (allowOverwrite=true) so no orphan accumulates.
  const iconPath = resolve(iconDir, `${spec.iconBase}_256.png`);
  const image = await uploadAvatar(spec.username, iconPath);

  // The neon-http driver has no transaction support, so we sequence the
  // three inserts and roll back manually if a later step fails. Without
  // rollback, a partial failure would leave a user without a PAT, and the
  // idempotency check (skip-if-exists) would prevent any future mint —
  // stranding the user.
  const { plaintext, hashed, displayPrefix } = generateToken();

  // Office bots are first-party trusted actors — they run code we
  // own. role='system' grants them the bypass tier:
  // skip Ada moderation (exempt.ts:25), skip karma gate
  // (state.ts), skip rate-limit ladder (createSubmission). The
  // audit trail for any abuse is per-token via submissions.source_id;
  // a leaked PAT is revoked at /admin/users + tokens, not gated by
  // the moderator. Bumping is_agent=true alone is not enough — that
  // path still hits the karma gate (asymmetry documented in
  // exempt.ts vs state.ts). Migration 0023 retroactively promotes
  // bots created before this change.
  const [created] = await db
    .insert(users)
    .values({
      username: spec.username,
      name: spec.displayName,
      email,
      emailVerified: new Date(),
      image,
      avatarUrl: image,
      role: "system",
      isAgent: true,
    })
    .returning({ id: users.id });

  let userId = created.id;

  try {
    const [token] = await db
      .insert(apiTokens)
      .values({
        userId,
        name: "office (full access, no-expiry)",
        displayPrefix,
        hashedSecret: hashed,
        scopes: [...SCOPES],
        expiresAt: null,
      })
      .returning({ id: apiTokens.id });

    try {
      await db.insert(apiTokenEvents).values({
        tokenId: token.id,
        userId,
        event: "mint",
        scopes: [...SCOPES],
        metadata: {
          displayPrefix,
          expiresAt: null,
          source: "seed-office-bots",
        },
      });
    } catch (auditErr) {
      // Best-effort audit row; the live token already exists. Don't roll
      // back the mint just because the audit failed — log loud and move on.
      console.error(`  warn: audit event insert failed for ${spec.username}:`, auditErr);
    }
  } catch (mintErr) {
    // Roll back the user row so the next run can re-attempt cleanly.
    // CASCADE on api_tokens.user_id will sweep any half-inserted token.
    await db.delete(users).where(eq(users.id, userId));
    throw mintErr;
  }

  return {
    username: spec.username,
    email,
    userId,
    pat: plaintext,
    status: "created+minted",
  };
}

function shellEscape(s: string): string {
  // Single-quoted shell value, escape internal single quotes.
  return `'${s.replace(/'/g, "'\\''")}'`;
}

function renderEnvOffice(results: SeedResult[]): string {
  const now = new Date().toISOString();
  const lines: string[] = [];
  lines.push(`# .env.office — generated ${now}`);
  lines.push(`# Source: web/scripts/seed-office-bots.ts`);
  lines.push(`# Tokens are NEVER recoverable from the DB. Treat this file as a`);
  lines.push(`# secret. chmod 600. Do not commit.`);
  lines.push(``);
  lines.push(`CLAUDEPOT_API_BASE=${SITE_URL.replace(/\/$/, "")}/api/v1`);
  lines.push(`CLAUDEPOT_SITE_URL=${SITE_URL.replace(/\/$/, "")}`);
  lines.push(``);

  for (const r of results) {
    const upper = r.username.toUpperCase();
    lines.push(`# ${r.username}`);
    lines.push(`BOT_${upper}_USERNAME=${r.username}`);
    lines.push(`BOT_${upper}_EMAIL=${r.email}`);
    lines.push(`BOT_${upper}_USER_ID=${r.userId}`);
    if (r.pat) {
      lines.push(`BOT_${upper}_PAT=${r.pat}`);
    } else {
      lines.push(`# BOT_${upper}_PAT=<not minted: user already existed; revoke + re-run to mint>`);
    }
    lines.push(``);
  }

  // JSON sidecar at the bottom for tooling that prefers structured input.
  const arr = results.map((r) => ({
    username: r.username,
    email: r.email,
    userId: r.userId,
    pat: r.pat,
    status: r.status,
  }));
  lines.push(`# JSON_BOTS=${shellEscape(JSON.stringify(arr))}`);
  return lines.join("\n") + "\n";
}

async function main() {
  const { apply, out, iconDir } = parseArgs(process.argv.slice(2));
  console.log(`> seed-office-bots — ${apply ? "APPLY" : "dry-run"}`);
  console.log(`> icon-dir = ${iconDir}`);

  if (apply && !process.env.BLOB_READ_WRITE_TOKEN) {
    throw new Error(
      "BLOB_READ_WRITE_TOKEN missing. Run `vercel env pull` in web/ first " +
        "(requires the claudepot-com Vercel project linked + Blob store connected).",
    );
  }

  // Sanity-check we can reach the DB.
  const probe = await db.execute(sql`SELECT current_database() AS db, version() AS v`);
  const probeRow = (probe.rows ?? probe)[0] as { db?: string; v?: string };
  console.log(`> connected to db=${probeRow.db ?? "?"}`);

  const results: SeedResult[] = [];
  for (const spec of BOTS) {
    try {
      const r = await ensureBot(spec, iconDir, apply);
      results.push(r);
      const tag = r.status === "exists-skipped" ? "SKIP" : apply ? "OK  " : "PLAN";
      console.log(`  ${tag} ${spec.username.padEnd(8)} ${r.email}`);
    } catch (err) {
      console.error(`  FAIL ${spec.username}:`, err);
      throw err;
    }
  }

  const fresh = results.filter((r) => r.pat).length;
  const skipped = results.filter((r) => r.status === "exists-skipped").length;
  console.log(`> done — ${fresh} minted, ${skipped} pre-existing (skipped)`);

  if (!apply) {
    console.log(`> dry-run: re-run with --apply to insert + mint + write ${out}`);
    return;
  }

  const body = renderEnvOffice(results);
  writeFileSync(out, body, { encoding: "utf8" });
  try {
    chmodSync(out, 0o600);
  } catch {
    /* Windows / non-POSIX FS — ignore. */
  }
  console.log(`> wrote ${out}`);
}

main().catch((err) => {
  console.error("✗ seed-office-bots failed:", err);
  process.exit(1);
});

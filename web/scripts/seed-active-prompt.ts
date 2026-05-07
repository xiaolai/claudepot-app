/**
 * One-shot seed: insert the current FALLBACK_SYSTEM_PROMPT into the
 * `moderation_prompts` table as the active row.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/seed-active-prompt.ts
 *
 * Why this exists. Migration 0021 created the table; the runtime
 * (`lib/moderation/prompt-store.ts`) reads the active row and falls
 * back to the in-code constant when the table is empty, stamping
 * every `policy_decisions.prompt_version` as `'fallback'`. With the
 * v2 prompt now shipping (commit 4ae556a — Ada-as-tagger,
 * `POLICY_PROMPT_V=2`), the audit trail should say `v2`, not
 * `'fallback'`. This script seeds that row so future decisions are
 * tagged with the real version label.
 *
 * Future tweaks go through /admin/policy-prompt (which writes a new
 * row, deactivates the prior, and logs to moderation_log). This
 * seed only runs once per fresh DB.
 *
 * Idempotent: re-runs are safe — if a row with version='v2' already
 * exists the script no-ops. If some OTHER version is already active,
 * the script refuses and reports — staff can re-activate via the
 * admin UI.
 *
 * Attribution: created_by is set to ada's user id (system bot,
 * created by migration 0009_persona_bots) so the seeded row is
 * clearly attributed to a non-staff system actor. The /admin path
 * uses the staff user instead.
 */

import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { moderationPrompts } from "@/db/schema";
import { FALLBACK_SYSTEM_PROMPT } from "@/lib/moderation/prompt";
import { getSystemUserId } from "@/lib/moderation/system-user";

const VERSION = "v2";

async function main(): Promise<void> {
  const adaId = await getSystemUserId();

  const [existing] = await db
    .select({ id: moderationPrompts.id, version: moderationPrompts.version })
    .from(moderationPrompts)
    .where(eq(moderationPrompts.version, VERSION))
    .limit(1);

  if (existing) {
    console.log(`moderation_prompts version='${VERSION}' already exists (id=${existing.id}); no-op.`);
    return;
  }

  const [activeOther] = await db
    .select({ version: moderationPrompts.version })
    .from(moderationPrompts)
    .where(eq(moderationPrompts.active, true))
    .limit(1);

  if (activeOther) {
    console.error(
      `another row is already active (version='${activeOther.version}'). ` +
        `Refusing to insert v2 — promote via /admin/policy-prompt instead.`,
    );
    process.exit(1);
  }

  const [inserted] = await db
    .insert(moderationPrompts)
    .values({
      version: VERSION,
      systemPrompt: FALLBACK_SYSTEM_PROMPT,
      active: true,
      createdBy: adaId,
      note: "Seeded from FALLBACK_SYSTEM_PROMPT to retire prompt_version='fallback' audit-trail tag.",
    })
    .returning({ id: moderationPrompts.id });

  console.log(
    `inserted moderation_prompts row id=${inserted.id} version='${VERSION}' active=true ` +
      `(${FALLBACK_SYSTEM_PROMPT.length} chars). prompt-store cache TTL is 60s; warm processes will pick it up within that window.`,
  );
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});

/**
 * DB-backed system-prompt loader for the policy moderator.
 *
 * Returns the active row from `moderation_prompts` (migration 0021)
 * if one exists, or the FALLBACK_SYSTEM_PROMPT constant from
 * prompt.ts otherwise.
 *
 *   - The cache is per-process and TTL-bounded so a recently-saved
 *     prompt becomes effective within CACHE_TTL_MS without a
 *     redeploy. Server-action publishNewPrompt clears the cache
 *     immediately after activation; the TTL is the second-line
 *     defense against stale process state across cold starts.
 *
 *   - Active-prompt resolution failures fall back to the constant
 *     rather than crashing the moderator. The persistence layer
 *     stamps `prompt_version='fallback'` so calibration analysis
 *     can detect the degraded mode.
 *
 *   - One DB query per CACHE_TTL_MS window (per warm process). At
 *     the platform's volume this is negligible.
 */

import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { moderationPrompts } from "@/db/schema";
import { FALLBACK_SYSTEM_PROMPT } from "./prompt";

const CACHE_TTL_MS = 60_000;
const FALLBACK_VERSION = "fallback";

export interface ActivePrompt {
  systemPrompt: string;
  version: string;
}

let cached: ActivePrompt | null = null;
let cachedAt = 0;

export async function getActiveSystemPrompt(): Promise<ActivePrompt> {
  const now = Date.now();
  if (cached && now - cachedAt < CACHE_TTL_MS) return cached;

  try {
    const [row] = await db
      .select({
        version: moderationPrompts.version,
        systemPrompt: moderationPrompts.systemPrompt,
      })
      .from(moderationPrompts)
      .where(eq(moderationPrompts.active, true))
      .limit(1);

    cached = row
      ? { systemPrompt: row.systemPrompt, version: row.version }
      : { systemPrompt: FALLBACK_SYSTEM_PROMPT, version: FALLBACK_VERSION };
  } catch (err) {
    // DB error → degrade to fallback rather than blocking moderation.
    // The moderator's failure-mode matrix already handles synthetic-
    // pass-on-error if the moderate() call subsequently fails; this
    // path keeps the prompt fetch from poisoning otherwise-healthy
    // calls.
    const msg = err instanceof Error ? err.message : String(err);
    console.warn(
      `[moderation/prompt-store] active-prompt lookup failed; using fallback: ${msg}`,
    );
    cached = {
      systemPrompt: FALLBACK_SYSTEM_PROMPT,
      version: FALLBACK_VERSION,
    };
  }
  cachedAt = now;
  return cached;
}

export function clearPromptCache(): void {
  cached = null;
  cachedAt = 0;
}

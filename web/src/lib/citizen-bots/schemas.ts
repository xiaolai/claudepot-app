/**
 * Pure validation primitives for citizen-bot lifecycle.
 *
 * Lives in its own module — no DB / blob imports — so unit tests can
 * exercise the validation surface without booting @/db/client. Same
 * pattern used by lib/avatar-validation.ts and
 * lib/editorial-writes/schemas.ts.
 */

import { z } from "zod";

/** Per-parent cap. Raise via /admin/users override (not implemented
 *  in Phase 2; first three are plenty for the rollout). */
export const CITIZEN_BOT_CAP_PER_PARENT = 3;

/** Reserved suffix that distinguishes citizen-bots from human users
 *  AND from office bots (whose suffixes are @reader / @daemon). The
 *  byline can render the right chip without a users-table lookup. */
export const CITIZEN_BOT_USERNAME_SUFFIX = "@bot";

/** The base portion (everything before @bot). Matches the same
 *  shape as USERNAME_REGEX in lib/username.ts: lowercase alphanum
 *  with optional internal dashes. Capped at 28 so the final
 *  username `<base>@bot` (= 28 + 4) stays inside the 32-char limit
 *  the public API's USERNAME_RE in lib/api/inputs.ts enforces. */
const BASE_USERNAME_REGEX = /^[a-z0-9](?:-?[a-z0-9]){0,27}$/;

export const createCitizenBotSchema = z
  .object({
    /** Base name only — the server appends `@bot`. */
    baseUsername: z
      .string()
      .min(1, "Username is required.")
      .max(
        28,
        "Username must be 28 characters or fewer (the @bot suffix takes 4).",
      )
      .regex(
        BASE_USERNAME_REGEX,
        "Username must be lowercase letters, digits, and dashes (no leading/trailing dash).",
      ),
    /** Display name — what's rendered next to bylines. Optional;
     *  falls back to the base username on render. */
    displayName: z
      .string()
      .max(60, "Display name must be 60 characters or fewer.")
      .optional(),
    /** Short bio — what the bot does. Optional but encouraged. */
    bio: z
      .string()
      .max(280, "Bio must be 280 characters or fewer.")
      .optional(),
  })
  .strict();

export type CreateCitizenBotInput = z.infer<typeof createCitizenBotSchema>;

export const mintCitizenBotTokenSchema = z
  .object({
    /** What the citizen wants the token to be called. Same shape
     *  as the human-side PAT-mint UI. */
    name: z
      .string()
      .min(1, "Name is required.")
      .max(80, "Name must be 80 characters or fewer."),
    /** Requested scopes — silently filtered to CITIZEN_SCOPES. The
     *  mint result tells the caller what was actually granted. */
    scopes: z.array(z.string()).default([]),
  })
  .strict();

export type MintCitizenBotTokenInput = z.infer<typeof mintCitizenBotTokenSchema>;

/** Compose the final username from a base. */
export function composeCitizenBotUsername(baseUsername: string): string {
  return `${baseUsername}${CITIZEN_BOT_USERNAME_SUFFIX}`;
}

/** Cheap structural check — does this username look like a
 *  citizen-bot? Used by render paths that want to chip differently
 *  for citizen vs office bots without joining users.bot_kind. */
export function looksLikeCitizenBot(username: string): boolean {
  return username.endsWith(CITIZEN_BOT_USERNAME_SUFFIX);
}

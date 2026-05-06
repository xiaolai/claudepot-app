/**
 * Single source of truth for the tag-slug shape.
 *
 * Three call sites depend on this:
 *
 *   - `lib/moderation/schema.ts` — Zod parse on Ada's structured
 *     output. A slug that fails this drops to is_new=false skip
 *     in apply-ai-tags (the regex check is enforced inside Zod).
 *   - `lib/actions/admin-tag.ts` — staff create/rename/merge/retire
 *     and approve/reject pending. Staff can never produce a slug
 *     Ada couldn't legally emit, and vice versa.
 *   - `lib/api/inputs.ts` — REST/MCP submission tags array. Same
 *     shape so external callers can't slip in a slug that admin
 *     can't render.
 *
 * The shape:
 *
 *   - lowercase ASCII letters, digits, hyphens
 *   - leading letter required (forbids "-foo", "9-foo")
 *   - no consecutive hyphens (forbids "foo--bar")
 *   - no trailing hyphen (forbids "foo-")
 *   - 2..40 characters total
 *
 * Why a leading letter? It keeps slugs distinct from numeric ids
 * in the routing layer ("/c/9" should never collide with a tag).
 * Why max 40? Display budget on a chip — anything longer wraps
 * awkwardly. Why no double hyphen? The slug is meant to be one
 * concept, not a smashed-together n-gram.
 */

import { z } from "zod";

/**
 * Canonical regex. Read it as: a letter, then any mix of alpha-
 * numeric + single hyphens, ending in alphanumeric. The
 * `(?:[a-z0-9]+-)*[a-z0-9]+` tail prevents both consecutive and
 * trailing hyphens without a lookahead.
 */
export const TAG_SLUG_RE = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;

/**
 * Zod schema for inputs that arrive as raw strings (form data,
 * external API payloads). `.trim()` runs first so trailing
 * whitespace from copy/paste doesn't trip the regex; then the
 * length and pattern checks fire.
 */
export const tagSlugSchema = z
  .string()
  .trim()
  .min(2)
  .max(40)
  .regex(TAG_SLUG_RE, {
    message:
      "Slug must be lowercase, start with a letter, use only letters/digits/hyphens, no consecutive or trailing hyphens.",
  });

/**
 * Pure regex test for hot paths that already have a string and
 * don't need Zod's overhead. Keeps the regex literal in one
 * place. Returns true on a valid slug, false otherwise.
 */
export function isValidTagSlug(s: string): boolean {
  if (s.length < 2 || s.length > 40) return false;
  return TAG_SLUG_RE.test(s);
}

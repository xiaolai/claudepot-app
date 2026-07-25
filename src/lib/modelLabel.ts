/**
 * Compact display label for a Claude model id.
 *
 * One helper, because there were two: the Sessions tab and the
 * Activities dashboard each carried their own near-copy, and they
 * disagreed. The Sessions copy required three hyphen-separated
 * segments (`opus-4-7`), so it silently fell through to the raw id for
 * the dateless `claude-opus-5` shape Anthropic introduced with the 4.6
 * generation — a session on the default Opus model rendered as
 * `opus-5` in one place and `Opus 5` in the other.
 *
 * Casing is applied here; several call sites additionally uppercase via
 * CSS, which composes fine.
 */

const CLAUDE_PREFIX = "claude-";

/** Trailing `-YYYYMMDD` (or longer) snapshot stamp. */
const SNAPSHOT_SUFFIX = /-\d{8,}$/;

/** Alias markers CC sometimes appends when resolving a model pointer. */
const ALIAS_SUFFIX = /-(preview|latest|experimental)$/i;

/**
 * `claude-opus-4-7` → `Opus 4.7`; `claude-opus-5` → `Opus 5`;
 * `claude-haiku-4-5-20251001` → `Haiku 4.5`.
 *
 * Ids that aren't in `claude-<family>-<version>` shape — a Bedrock id
 * like `us.anthropic.claude-sonnet-4-5-v1:0`, a third-party route's
 * model, an empty string — are returned verbatim rather than forced
 * into the pattern. Showing an unfamiliar id as-is is honest; dressing
 * it up as `Us 4` is not.
 */
export function compactModelLabel(id: string): string {
  const trimmed = id.trim();
  if (!trimmed.toLowerCase().startsWith(CLAUDE_PREFIX)) return trimmed;

  // Strip repeatedly rather than once each: the two suffixes can stack
  // in either order (`…-20260724-preview` puts the alias last, so a
  // single anchored snapshot strip would never match). Loop until the
  // id stops shrinking.
  let stripped = trimmed.slice(CLAUDE_PREFIX.length);
  for (;;) {
    const next = stripped.replace(SNAPSHOT_SUFFIX, "").replace(ALIAS_SUFFIX, "");
    if (next === stripped) break;
    stripped = next;
  }

  const [family, ...version] = stripped.split("-");
  if (!family) return trimmed;

  const label = family.charAt(0).toUpperCase() + family.slice(1);
  return version.length > 0 ? `${label} ${version.join(".")}` : label;
}

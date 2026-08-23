/**
 * Formatting helpers shared by the Sessions tab. All numbers are
 * rendered "render-if-nonzero" per design rules — callers should
 * branch on `formatTokens(n)` being empty / null rather than showing
 * "0 tokens" in the UI.
 */

import { compactModelLabel } from "../../lib/modelLabel";
import { basename } from "../../lib/paths";
import { sessionEventMs } from "../../lib/sessionTime";

export function formatTokens(n: number): string {
  if (n <= 0) return "";
  if (n < 1_000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0)}M`;
}

/**
 * Humanize the full set of models a session used. Keeps the UI calm
 * when a session bounced between Opus 5 and Haiku 4.5. The per-id
 * shortening lives in `lib/modelLabel` so this and the Activities
 * dashboard can't drift apart on what a model is called.
 */
export function modelBadge(models: string[]): string {
  if (models.length === 0) return "";
  const compact = models.map(compactModelLabel);
  if (compact.length === 1) return compact[0];
  const unique = Array.from(new Set(compact));
  if (unique.length === 1) return unique[0];
  return unique.join(" · ");
}

/**
 * Short project basename for table rows. `/a/b/c` → `c`; Windows-aware
 * (`C:\a\b` → `b`) via lib/paths (audit 2026-07 F2).
 * Empty string survives as-is so the caller can fall back to the slug.
 */
export function projectBasename(path: string): string {
  return basename(path);
}

export function shortSessionId(id: string): string {
  return id.length >= 8 ? id.slice(0, 8) : id;
}

/**
 * Prefer the live-event `last_ts` timestamp; fall back to file mtime
 * when CC hasn't written a dated event (new empty session).
 *
 * Re-exported from `lib/sessionTime` — the cost surfaces outside this
 * section need the same rule, and dated pricing makes "which timestamp"
 * a correctness question rather than a display preference.
 */
export const bestTimestampMs = sessionEventMs;

/**
 * CC's reference-placeholder regex — kept byte-compatible with the
 * upstream definition in `claude_code_src/src/history.ts::parseReferences`
 * so a new placeholder shape added upstream trips a single fixup here.
 */
const CC_REF_PLACEHOLDER_RE =
  /\[(?:Pasted text|Image|\.\.\.Truncated text) #\d+(?: \+\d+ lines)?\.*\]/g;

/**
 * Turn a raw `first_user_prompt` into a clean single-line title.
 *
 * What people type into Claude Code is markdown, and a title has no room
 * for it: a real session here opens with `## User Input`, a fenced path,
 * and a `- **Plan files**` bullet, all of which were on screen.
 *
 * The marks come **out** rather than being rendered — the opposite of
 * what the transcript body does, and deliberately so.
 *
 * ## Two implementations, locked together
 *
 * The same rule lives in `claudepot-core::session::title::derive`, which
 * is what the remote panel's DTO uses. This copy exists because the
 * desktop renders `SessionRow` straight over IPC, and adding a derived
 * column to the cache would be a schema change for a presentation
 * concern.
 *
 * Both run `crates/claudepot-core/testdata/session-title-vectors.json`.
 * **Change one, change the other, add a vector** — the same arrangement
 * `PriceBook::resolve` and `src/costs.ts` have.
 *
 * ## What is deliberately left alone
 *
 * Every underscore, including `__bold__`, and single `*`. A stripper
 * that treats them as emphasis destroys `__init__`, `some_var_name`,
 * `*.log` and `2 * 3` — all plausible openings for a prompt. The
 * vectors caught exactly that. One stray asterisk reads far better than
 * a mangled path.
 *
 * Returns `null` if nothing readable is left — callers fall back to a
 * session-id or subsession hint, and a blank row is worse than either.
 */
export function deriveSessionTitle(raw: string | null): string | null {
  if (raw == null) return null;
  const cleaned = raw.replace(CC_REF_PLACEHOLDER_RE, "");

  // Scaffolding is peeled per line, before the lines are joined. Doing
  // it only at the head leaves a list that starts on line five with its
  // markers; doing it after the join cannot tell a bullet from a hyphen
  // in "well - it depends", because only position at the start of a line
  // distinguishes them.
  const lines = cleaned.split("\n").map((line) => {
    let l = line;
    let prev = "";
    while (prev !== l) {
      prev = l;
      l = l.replace(/^\s+/, "");
      // A fence before a heading, so "```md" then "# Title" unwinds in
      // that order.
      l = l.replace(/^```[A-Za-z0-9_+-]*[ \t]*$/, "");
      l = l.replace(/^```[A-Za-z0-9_+-]*[ \t]*/, "");
      l = l.replace(/^#{1,6}\s+/, "");
      l = l.replace(/^>\s*/, "");
      l = l.replace(/^(?:[-*+]|\d+\.)[ \t]+/, "");
    }
    return l;
  });
  let s = lines.join("\n");

  // Inline marks. Images before links, or `![alt](url)` reads as a link
  // behind a stray `!`.
  s = s.replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1");
  s = s.replace(/\[([^\]]*)\]\([^)]*\)/g, "$1");
  s = s.replace(/<((?:https?|mailto):[^>\s]+)>/g, "$1");
  s = s.replace(/\*\*([^*]+)\*\*/g, "$1");
  s = s.replace(/~~([^~]+)~~/g, "$1");
  s = s.replace(/```[A-Za-z0-9_+-]*/g, " ");
  s = s.replace(/`([^`]*)`/g, "$1");

  // One line. A prompt is often a paragraph; a row is not.
  s = s.replace(/\s+/g, " ").trim();
  return s.length > 0 ? s : null;
}

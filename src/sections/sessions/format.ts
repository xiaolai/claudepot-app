/**
 * Formatting helpers shared by the Sessions tab. All numbers are
 * rendered "render-if-nonzero" per design rules — callers should
 * branch on `formatTokens(n)` being empty / null rather than showing
 * "0 tokens" in the UI.
 */

export function formatTokens(n: number): string {
  if (n <= 0) return "";
  if (n < 1_000) return `${n}`;
  if (n < 1_000_000) return `${(n / 1_000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(n < 10_000_000 ? 1 : 0)}M`;
}

/**
 * Humanize the full set of models a session used. Collapses the
 * `claude-<family>-<N>-<variant>` prefix (we keep the family and the
 * dotted version). Keeps the UI calm when a session bounced between
 * Opus 4.7 and Haiku 4.5.
 */
export function modelBadge(models: string[]): string {
  if (models.length === 0) return "";
  const compact = models.map(compactModel);
  if (compact.length === 1) return compact[0];
  const unique = Array.from(new Set(compact));
  if (unique.length === 1) return unique[0];
  return unique.join(" · ");
}

function compactModel(id: string): string {
  const trimmed = id.replace(/^claude-/, "").replace(/-\d{8,}$/, "");
  const parts = trimmed.split("-");
  if (parts.length >= 3) {
    // opus-4-7 → opus 4.7
    const [family, major, minor] = parts;
    return `${family} ${major}.${minor}`;
  }
  return trimmed;
}

/**
 * Short project basename for table rows. `/a/b/c` → `c`.
 * Empty string survives as-is so the caller can fall back to the slug.
 */
export function projectBasename(path: string): string {
  return path.split("/").filter(Boolean).pop() ?? path;
}

export function shortSessionId(id: string): string {
  return id.length >= 8 ? id.slice(0, 8) : id;
}

/**
 * Prefer the live-event `last_ts` timestamp; fall back to file mtime
 * when CC hasn't written a dated event (new empty session).
 */
export function bestTimestampMs(
  lastTs: string | null,
  lastModifiedMs: number | null,
): number | null {
  if (lastTs) {
    const t = Date.parse(lastTs);
    if (!Number.isNaN(t)) return t;
  }
  return lastModifiedMs;
}

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
 * CC embeds placeholders like `[Image #3]` and `[Pasted text #1 +42 lines]`
 * directly into the prompt text — they are stripped here because they
 * leak internal encoding into the UI (design.md: "No internal
 * identifiers in primary UI"). Leading Markdown scaffolding (headers,
 * opening code fences, blockquote markers) is also stripped so a prompt
 * that started with ` ```ts ` or `## spec` renders as the readable text
 * that followed it rather than raw markup.
 *
 * Returns `null` if the cleaned string is empty — callers fall back to
 * a session-id or subsession hint.
 */
export function deriveSessionTitle(raw: string | null): string | null {
  if (raw == null) return null;
  let s = raw.replace(CC_REF_PLACEHOLDER_RE, "");
  // Strip leading Markdown scaffolding, one layer at a time, tolerating
  // stacks (e.g. "> ## heading"). Each branch peels one marker plus its
  // trailing whitespace and loops until nothing matches.
  //
  // Order matters: the opening code fence must be removed before the
  // header-hash pass, so a line like "```md\n# Title" becomes "# Title"
  // first and then "Title".
  let prev = "";
  while (prev !== s) {
    prev = s;
    s = s.replace(/^\s+/, "");
    s = s.replace(/^```[a-zA-Z0-9_+-]*\s*\n?/, "");
    s = s.replace(/^#{1,6}\s+/, "");
    s = s.replace(/^>\s*/, "");
  }
  // Collapse internal runs of whitespace so a multi-line prompt
  // renders as one clean line in the row's truncated headline.
  s = s.replace(/\s+/g, " ").trim();
  return s.length > 0 ? s : null;
}

/**
 * Server-side syntax highlighter via Shiki.
 *
 * Singleton highlighter — created lazily on the first call and
 * reused across requests within the same Node instance. Shiki's
 * grammar + theme load is the expensive part; highlighting itself
 * is fast once the highlighter exists.
 *
 * The set of bundled languages is intentionally small: the common
 * ones for an AI-tools editorial site. Adding a language later is
 * a one-line edit; loading every grammar would balloon the
 * function bundle for languages that almost never appear.
 *
 * Returns an array of per-line HTML strings — token spans with
 * inline `color:` styles. The caller wraps each line in
 * .proto-code-line so the gutter / counter still works.
 */

import {
  createHighlighter,
  type BundledLanguage,
  type Highlighter,
} from "shiki";

const LANGS = [
  "typescript",
  "javascript",
  "tsx",
  "jsx",
  "bash",
  "shell",
  "json",
  "yaml",
  "css",
  "html",
  "python",
  "rust",
  "go",
  "sql",
  "markdown",
  "diff",
] as const;

const THEME = "github-light";

// Language aliases that markdown authors commonly write (e.g. ```sh
// instead of ```bash). Shiki accepts the canonical names; we map at
// the call site so authors can stay informal.
const LANG_ALIASES: Record<string, string> = {
  ts: "typescript",
  js: "javascript",
  sh: "bash",
  shellscript: "bash",
  yml: "yaml",
  py: "python",
  rs: "rust",
  golang: "go",
  md: "markdown",
};

const SUPPORTED = new Set<string>(LANGS);

function escapeHtmlText(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

let highlighterPromise: Promise<Highlighter> | null = null;

function getHighlighter(): Promise<Highlighter> {
  if (!highlighterPromise) {
    highlighterPromise = createHighlighter({
      themes: [THEME],
      langs: [...LANGS],
    });
  }
  return highlighterPromise;
}

/**
 * Resolve a markdown lang tag to a Shiki-supported language. Returns
 * null when the language isn't bundled — the caller should fall back
 * to plain text rendering (no syntax tokens, just escaped lines).
 */
function resolveLang(input: string): string | null {
  const norm = input.toLowerCase();
  const candidate = LANG_ALIASES[norm] ?? norm;
  return SUPPORTED.has(candidate) ? candidate : null;
}

/**
 * Build a single concatenated string of <span class="proto-code-line">
 * lines for `code`, syntax-highlighted in `lang`. Empty source lines
 * use a non-breaking space so the gutter's counter still increments
 * with visible vertical height.
 *
 * Falls back to escaped-only rendering when the language isn't
 * supported — the line gutter and copy button still work; only the
 * token colors are missing.
 */
export async function highlightCodeToLines(
  code: string,
  lang: string,
): Promise<string> {
  const resolved = resolveLang(lang);

  if (!resolved) {
    return code
      .split("\n")
      .map(
        (line) =>
          `<span class="proto-code-line">${
            line === "" ? "&#160;" : escapeHtmlText(line)
          }</span>`,
      )
      .join("");
  }

  try {
    const hl = await getHighlighter();
    // resolveLang() guards against unsupported languages, and LANGS
    // is a const tuple of BundledLanguage values, so the narrowing
    // here is safe — but Shiki types resolved as a plain string,
    // which doesn't satisfy the BundledLanguage union directly.
    const { tokens } = hl.codeToTokens(code, {
      lang: resolved as BundledLanguage,
      theme: THEME,
    });
    return tokens
      .map((line) => {
        if (line.length === 0) {
          return `<span class="proto-code-line">&#160;</span>`;
        }
        const inner = line
          .map((t) => {
            const color = t.color ? ` style="color:${t.color}"` : "";
            return `<span${color}>${escapeHtmlText(t.content)}</span>`;
          })
          .join("");
        return `<span class="proto-code-line">${inner}</span>`;
      })
      .join("");
  } catch (err) {
    // Highlighter or grammar failure — log and fall through to the
    // unhighlighted path. A dropped color set is strictly better
    // than a 500 from the post page.
    console.warn(
      `[highlight] failed for lang=${resolved}; falling back to plain.`,
      err,
    );
    return code
      .split("\n")
      .map(
        (line) =>
          `<span class="proto-code-line">${
            line === "" ? "&#160;" : escapeHtmlText(line)
          }</span>`,
      )
      .join("");
  }
}

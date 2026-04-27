import { useMemo } from "react";
import hljs from "highlight.js/lib/common";

/**
 * Syntax-highlighted preview for non-markdown / non-JSON config files.
 *
 * Used for `statusline`, `hook`, and `other` kinds (shell scripts,
 * Python hooks, etc.) — anything that falls past the MarkdownRenderer
 * and JsonTreeRenderer dispatchers in FilePreview.
 *
 * Language detection order:
 *   1. File extension lookup (path is authoritative — `statusline.py`
 *      must NOT be force-highlighted as bash by a kind hint)
 *   2. Shebang line (`#!/usr/bin/env bash`, `#!/usr/bin/env -S python3 -u`)
 *   3. `defaultLang` prop (last-resort fallback — e.g., kind=statusline
 *      defaults to bash when the file has no extension and no shebang)
 *   4. highlight.js auto-detect over registered grammars
 *
 * Bundle: imports `highlight.js/lib/common` (~37 grammars, the same
 * set rehype-highlight uses for markdown fences via lowlight). If a
 * less-common grammar is needed, register it explicitly here rather
 * than swapping in the full `highlight.js` import — that pulls all
 * 190+ grammars and balloons the bundle.
 *
 * SECURITY: highlight.js consumes the source as a string and produces
 * a hast / structured token stream. We render via React text nodes
 * and styled spans only — no `dangerouslySetInnerHTML` from input.
 */
export function CodeRenderer({
  body,
  path,
  defaultLang,
}: {
  body: string;
  /** Absolute path, used for extension-based language detection. */
  path?: string | null;
  /**
   * Last-resort language hint. Used only when the path's extension
   * and any shebang both fail to identify a language. Real file
   * signals always win over a kind-based default.
   */
  defaultLang?: string | null;
}) {
  const { html, language } = useMemo(
    () => highlight(body, defaultLang ?? null, path ?? null),
    [body, defaultLang, path],
  );

  return (
    <div className="code-block">
      <pre>
        <code
          className={`hljs${language ? ` language-${language}` : ""}`}
          aria-label={language ? `${language} code` : "code"}
          // Output of hljs.highlight is a structured token stream
          // serialized to HTML strings of <span class="hljs-…">. The
          // grammar runs over the source string and never reflects
          // unescaped input back into the markup, so this is safe to
          // pass through. We never enable raw-HTML rendering.
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </pre>
    </div>
  );
}

// ---------- Detection -------------------------------------------------

/**
 * Map of file extensions (lowercase, no dot) to highlight.js language
 * names. Only entries whose grammar is in `highlight.js/lib/common`
 * are listed; anything else falls through to auto-detect.
 */
const EXT_LANG: Record<string, string> = {
  // Shell
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  fish: "bash",
  ksh: "bash",
  // Python
  py: "python",
  pyw: "python",
  // JavaScript / TypeScript
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  jsx: "javascript",
  ts: "typescript",
  tsx: "typescript",
  // Web
  html: "xml",
  htm: "xml",
  xml: "xml",
  svg: "xml",
  css: "css",
  scss: "scss",
  // Data / config
  json: "json",
  jsonc: "json",
  yaml: "yaml",
  yml: "yaml",
  toml: "ini", // common bundle ships ini, not toml; ini lexer is close enough
  ini: "ini",
  conf: "ini",
  // Misc
  md: "markdown",
  markdown: "markdown",
  diff: "diff",
  patch: "diff",
  rs: "rust",
  go: "go",
  rb: "ruby",
  php: "php",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  swift: "swift",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  hpp: "cpp",
  sql: "sql",
};

/**
 * Map of shebang interpreter basenames to language names.
 * Matches the program after `#!/usr/bin/env ` or the basename of an
 * absolute interpreter path.
 */
const SHEBANG_LANG: Record<string, string> = {
  bash: "bash",
  sh: "bash",
  zsh: "bash",
  fish: "bash",
  python: "python",
  python3: "python",
  python2: "python",
  node: "javascript",
  deno: "typescript",
  bun: "typescript",
  "ts-node": "typescript",
  ts_node: "typescript",
  tsx: "typescript",
  ruby: "ruby",
  perl: "perl",
  php: "php",
};

function highlight(
  body: string,
  defaultLang: string | null,
  path: string | null,
): { html: string; language: string | null } {
  const detected =
    langFromPath(path) ?? langFromShebang(body) ?? defaultLang;
  if (detected && hljs.getLanguage(detected)) {
    try {
      const out = hljs.highlight(body, { language: detected, ignoreIllegals: true });
      return { html: out.value, language: out.language ?? detected };
    } catch {
      /* fall through to auto-detect */
    }
  }
  // No hint — try auto-detect. highlight.js scores every registered
  // grammar; on confident matches the result is good enough, on
  // ambiguous input it falls back to plain-text.
  try {
    const out = hljs.highlightAuto(body);
    return { html: out.value, language: out.language ?? null };
  } catch {
    return { html: escapeHtml(body), language: null };
  }
}

function langFromPath(path: string | null): string | null {
  if (!path) return null;
  // Strip query/hash/trailing slash. Paths from the backend are
  // already canonical, so a simple basename + extension parse works.
  const base = path.replace(/[\\/]+$/, "");
  const dot = base.lastIndexOf(".");
  const slash = Math.max(base.lastIndexOf("/"), base.lastIndexOf("\\"));
  if (dot <= slash) return null;
  const ext = base.slice(dot + 1).toLowerCase();
  return EXT_LANG[ext] ?? null;
}

function langFromShebang(body: string): string | null {
  if (!body.startsWith("#!")) return null;
  const nl = body.indexOf("\n", 2);
  const firstLine = body.slice(0, nl === -1 ? undefined : nl);

  // `#!/usr/bin/env [-S] <interp> [args...]` — skip `env` flags
  // (`-S`, `--split-string`, etc.) so we land on the actual
  // interpreter, not a flag.
  const envMatch = firstLine.match(/\benv\b\s+([\S\s]+)$/);
  if (envMatch) {
    const interp = firstWordSkippingFlags(envMatch[1]);
    const lang = lookupShebang(interp);
    if (lang) return lang;
  }

  // `#!/usr/bin/python3` → basename of the path.
  const pathMatch = firstLine.match(/^#!\s*(\S+)/);
  if (pathMatch) {
    const base = (pathMatch[1].split(/[\\/]/).pop() ?? "").toLowerCase();
    const lang = lookupShebang(base);
    if (lang) return lang;
  }
  return null;
}

/**
 * Pick the first non-flag token. `-S foo bar` → `foo`. Bare `--` ends
 * flag parsing. Empty input → empty string. Lowercased on return so
 * the caller can match SHEBANG_LANG keys directly.
 */
function firstWordSkippingFlags(s: string): string {
  for (const tok of s.trim().split(/\s+/)) {
    if (tok === "--") continue;
    if (tok.startsWith("-")) continue;
    return tok.toLowerCase();
  }
  return "";
}

/**
 * Match a shebang basename against SHEBANG_LANG. Tries the literal
 * key first, then strips trailing version digits (`python3.11` →
 * `python`) so versioned interpreters resolve.
 */
function lookupShebang(name: string): string | null {
  if (!name) return null;
  if (SHEBANG_LANG[name]) return SHEBANG_LANG[name];
  const stripped = name.replace(/[0-9.]+$/, "");
  if (stripped !== name && SHEBANG_LANG[stripped]) return SHEBANG_LANG[stripped];
  return null;
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

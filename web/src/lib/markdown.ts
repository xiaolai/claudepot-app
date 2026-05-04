/**
 * Markdown renderer with a strict allowlist.
 *
 * The list intentionally tracks GitHub Flavored Markdown's common
 * surface — headings, tables, images, GFM alerts (callouts), inline
 * markup, fenced code, mermaid — while keeping <script>, <iframe>,
 * <object>, raw <style>, and other active-content tags out.
 */

import { marked } from "marked";
import sanitizeHtml from "sanitize-html";

import { highlightCodeToLines } from "@/lib/highlight";

export const ALLOWED_TAGS = [
  "p",
  "br",
  "i",
  "em",
  "strong",
  "a",
  "code",
  "pre",
  "ul",
  "ol",
  "li",
  "blockquote",
  "del",
  // Headings — user-authored h1 is visually demoted in CSS so it
  // doesn't compete with the page-level <h1> for the post title.
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  // Block separator.
  "hr",
  // Tables (GFM). Each table is post-processed into a scroll wrapper
  // so wide tables don't break the content column.
  "table",
  "thead",
  "tbody",
  "tr",
  "th",
  "td",
  // Images. Locked to https in allowedSchemesByTag below; src is
  // also rewritten with safe defaults via transformTags.
  "img",
];

export const ALLOWED_ATTRS: sanitizeHtml.IOptions["allowedAttributes"] = {
  a: ["href", "title", "rel", "target"],
  th: ["align"],
  td: ["align"],
  img: [
    "src",
    "alt",
    "title",
    "width",
    "height",
    "loading",
    "referrerpolicy",
    "decoding",
  ],
};

// Lucide "Copy" outline (24×24) inlined for SSR. Kept in sync with
// the lucide-react source — when lucide ships a major redesign of the
// icon, rev this string. Stroke uses currentColor so theme tokens drive
// the rendered colour.
const COPY_ICON_SVG =
  '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
  '<rect width="14" height="14" x="8" y="8" rx="2" ry="2"></rect>' +
  '<path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"></path>' +
  "</svg>";

function escapeAttr(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/**
 * Decorate the sanitize-html output with a code-block shell:
 *
 *   <div class="proto-code" data-lang="X">
 *     <header class="proto-code-header">
 *       <span class="proto-code-lang">X</span>
 *       <button class="proto-code-copy" type="button" aria-label="Copy code">
 *         <svg .../><span class="proto-code-copy-label">Copy</span>
 *       </button>
 *     </header>
 *     <pre class="proto-code-body"><code class="language-X">
 *       <span class="proto-code-line">line 1</span>
 *       <span class="proto-code-line">line 2</span>
 *       …
 *     </code></pre>
 *   </div>
 *
 * Line numbers are produced by a CSS counter on .proto-code-line so
 * the SSR'd HTML doesn't carry redundant numeric strings.
 *
 * `language-mermaid` blocks pass through unchanged so the existing
 * client mermaid hydrator still finds and replaces them.
 *
 * Operates on already-sanitized HTML — the input is trusted output
 * from sanitize-html, and the inner code text is therefore HTML-
 * escaped already, which is what we want.
 */
/* sanitize-html escapes < > & inside code bodies. Shiki needs the
 * raw source to tokenize, so we reverse the escapes before passing
 * the code through, then let Shiki's own escaper handle the output.
 * (The five entities below are the only ones sanitize-html emits.) */
function unescapeHtmlText(s: string): string {
  return s
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, "&");
}

async function decorateCodeBlocks(html: string): Promise<string> {
  // String.replace doesn't accept async replacers, so we collect the
  // matches, run the highlighter calls in parallel via Promise.all,
  // then weave the results back into the original string in order.
  const pattern =
    /<pre><code(?:\s+class="language-([\w-]+)")?>([\s\S]*?)<\/code><\/pre>/g;
  const matches = Array.from(html.matchAll(pattern));
  if (matches.length === 0) return html;

  const replacements = await Promise.all(
    matches.map(async ([, lang, inner]) => {
      const language = lang || "plain";
      if (language === "mermaid") {
        // Hand back the same shape the mermaid hydrator expects.
        return `<pre><code class="language-mermaid">${inner}</code></pre>`;
      }
      // marked emits a trailing newline; drop it so the gutter isn't
      // off-by-one with a phantom blank line at the end.
      const escapedSource = inner.replace(/\n+$/, "");
      const rawSource = unescapeHtmlText(escapedSource);
      const lineHtml = await highlightCodeToLines(rawSource, language);
      const safeLang = escapeAttr(language);
      // Same hue palette as blockquotes — keeps the per-block accent
      // language unified across the page. Hashed on lang + source so
      // identical snippets pick identical hues; different snippets
      // visibly rotate.
      const hueIdx =
        Math.abs(hashString(language + "" + rawSource)) %
        ACCENT_HUE_PALETTE.length;
      const hue = ACCENT_HUE_PALETTE[hueIdx];
      return (
        `<div class="proto-code" data-lang="${safeLang}" style="--code-hue: ${hue}">` +
        `<header class="proto-code-header">` +
        `<span class="proto-code-lang">${safeLang}</span>` +
        `<button type="button" class="proto-code-copy" aria-label="Copy code" title="Copy code">` +
        COPY_ICON_SVG +
        `</button>` +
        `</header>` +
        `<pre class="proto-code-body"><code class="language-${safeLang}">` +
        lineHtml +
        `</code></pre>` +
        `</div>`
      );
    }),
  );

  // Walk the matches in document order and assemble the output.
  let out = "";
  let cursor = 0;
  for (let i = 0; i < matches.length; i += 1) {
    const m = matches[i];
    const start = m.index ?? 0;
    out += html.slice(cursor, start) + replacements[i];
    cursor = start + m[0].length;
  }
  out += html.slice(cursor);
  return out;
}

/* ── Accent hue palette (rotation source) ─────────────────────
 *
 * Six OKLCH hues anchored to the base accent. Used by
 * decorateBlockquotes (per-quote --bq-hue) and decorateCodeBlocks
 * (per-block --code-hue). Lightness + chroma stay in the
 * --accent-tone envelope (light: 68% 0.13; dark: 72% 0.17) so
 * every rotated surface keeps the same visual weight as the base
 * accent — only the hue moves.
 *
 *   15  — analogous warm (red-orange)
 *   45  — accent (copper)
 *   75  — analogous cool (amber)
 *   165 — triadic (teal)
 *   225 — complementary (blue)
 *   285 — split-complementary (purple)
 *
 * Selection is deterministic: each surface hashes the relevant
 * source (blockquote text; code lang + body) and indexes into the
 * palette modulo length. Same content → same hue across reloads;
 * different content → visibly varied across surfaces.
 *
 * Nested-blockquote limitation: the regex matches the outermost
 * <blockquote>…</blockquote> first, so an inner nested blockquote
 * keeps the default --bq-hue (45). User content rarely nests
 * blockquotes; if it becomes load-bearing, switch to a real HTML
 * parser here.
 */

const ACCENT_HUE_PALETTE = [15, 45, 75, 165, 225, 285];

function hashString(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i += 1) {
    h = (Math.imul(31, h) + s.charCodeAt(i)) | 0;
  }
  return h;
}

function decorateBlockquotes(html: string): string {
  return html.replace(
    /<blockquote>([\s\S]*?)<\/blockquote>/g,
    (_, inner: string) => {
      const idx =
        Math.abs(hashString(inner)) % ACCENT_HUE_PALETTE.length;
      const hue = ACCENT_HUE_PALETTE[idx];
      return `<blockquote style="--bq-hue: ${hue}">${inner}</blockquote>`;
    },
  );
}

/* ── GFM-style alert callouts ─────────────────────────────────
 *
 * GitHub Flavored Markdown's alert syntax: a blockquote whose
 * first line is `[!TYPE]` (NOTE / TIP / IMPORTANT / WARNING /
 * CAUTION) becomes a styled callout. After marked + sanitize-html,
 * the input shape we see is:
 *
 *   <blockquote style="--bq-hue: …">
 *     <p>[!NOTE]<br>
 *     This is a note.</p>
 *   </blockquote>
 *
 * (The style attribute came from decorateBlockquotes, which runs
 * earlier — we strip it on callout matches because the type carries
 * its own color.)
 *
 * We rewrite that to:
 *
 *   <blockquote class="proto-callout proto-callout-note">
 *     <p class="proto-callout-label">Note</p>
 *     <p>This is a note.</p>
 *   </blockquote>
 *
 * The label paragraph drives a uppercase eyebrow above the body
 * via CSS; the type-specific class paints the left rule and a
 * tinted background.
 */

const CALLOUT_TYPES = [
  "NOTE",
  "TIP",
  "IMPORTANT",
  "WARNING",
  "CAUTION",
] as const;
type CalloutType = (typeof CALLOUT_TYPES)[number];

const CALLOUT_LABELS: Record<CalloutType, string> = {
  NOTE: "Note",
  TIP: "Tip",
  IMPORTANT: "Important",
  WARNING: "Warning",
  CAUTION: "Caution",
};

function decorateGfmCallouts(html: string): string {
  // Match a blockquote (optionally with the hue-rotation inline
  // style emitted by decorateBlockquotes earlier in the chain) whose
  // first paragraph starts with [!TYPE]<br>. Capture: the rest of
  // the first paragraph, then the rest of the blockquote body.
  const pattern =
    /<blockquote(?: style="[^"]*")?>\s*<p>\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\](?:\s*<br\s*\/?>|\s)([\s\S]*?)<\/p>([\s\S]*?)<\/blockquote>/g;

  return html.replace(
    pattern,
    (_, type: CalloutType, firstParaRest: string, tail: string) => {
      const lower = type.toLowerCase();
      const label = CALLOUT_LABELS[type];
      const trimmedFirst = firstParaRest.trim();
      const innerFirstP = trimmedFirst ? `<p>${trimmedFirst}</p>` : "";
      return (
        `<blockquote class="proto-callout proto-callout-${lower}">` +
        `<p class="proto-callout-label">${label}</p>` +
        innerFirstP +
        tail +
        `</blockquote>`
      );
    },
  );
}

/* ── Table responsiveness wrapper ─────────────────────────────
 *
 * Tables can't be made horizontally scrollable without breaking
 * the table layout (display: block voids the table model). The
 * standard fix is a single-purpose wrapper div with overflow-x:
 * auto. We add one server-side per <table> here so wide tables
 * scroll inside their column instead of forcing the page wider.
 */
function decorateTables(html: string): string {
  return html.replace(
    /<table>([\s\S]*?)<\/table>/g,
    (_, inner: string) =>
      `<div class="proto-table-wrap"><table>${inner}</table></div>`,
  );
}

export async function renderMarkdown(source: string): Promise<string> {
  const html = marked.parse(source, { gfm: true, breaks: true }) as string;
  const sanitized = sanitizeHtml(html, {
    allowedTags: ALLOWED_TAGS,
    allowedAttributes: {
      ...ALLOWED_ATTRS,
      // marked emits e.g. <code class="language-mermaid"> for fenced
      // ```mermaid blocks; the client mermaid hydrator below keys off
      // that class. Restrict to the language-* shape so a hostile body
      // can't paint arbitrary class names onto its output.
      code: ["class"],
      pre: ["class"],
    },
    allowedClasses: {
      code: [/^language-[\w-]+$/],
      pre: [/^language-[\w-]+$/],
    },
    allowedSchemes: ["http", "https", "mailto"],
    // Tighter scheme list for <img> than for the rest. http and
    // mailto on an image src would either downgrade page security
    // (mixed content under HTTPS) or be nonsense (mailto on img).
    // data: is excluded because user content shouldn't be able to
    // ship arbitrary inline payloads.
    allowedSchemesByTag: { img: ["https"] },
    transformTags: {
      a: (tagName, attribs) => ({
        tagName,
        attribs: {
          ...attribs,
          rel: "noopener noreferrer ugc",
          target: "_blank",
        },
      }),
      // Lazy-load + no-referrer + async-decode by default so a
      // user-included image can't track the loading page through
      // the Referer header, and doesn't block paint.
      img: (tagName, attribs) => ({
        tagName,
        attribs: {
          ...attribs,
          loading: "lazy",
          referrerpolicy: "no-referrer",
          decoding: "async",
        },
      }),
    },
  });
  return decorateGfmCallouts(
    decorateTables(decorateBlockquotes(await decorateCodeBlocks(sanitized))),
  );
}

/**
 * Renderer for editorial-team docs (audience.md, transparency.md, etc.)
 * surfaced via /office/. Wider allowlist than user content — includes
 * headings, tables, horizontal rules — because the source is hand-authored
 * spec, not untrusted submission text.
 */
const EDITORIAL_DOC_TAGS = [
  ...ALLOWED_TAGS,
  "h1", "h2", "h3", "h4", "h5", "h6",
  "hr",
  "table", "thead", "tbody", "tr", "th", "td",
  "div", "span",
];

/**
 * Renderer for project README snapshots displayed at /projects/[slug].
 * Wider allowlist than user content (headings, tables, hr, images) plus
 * one extra job: rewrite relative paths against the source repo.
 *
 * - `<img src="docs/x.png">`     → raw.githubusercontent.com/{owner}/{repo}/HEAD/docs/x.png
 * - `<a href="src/index.ts">`    → github.com/{owner}/{repo}/blob/HEAD/src/index.ts
 *
 * Without this, the vast majority of badge / screenshot / source-link
 * references in a typical README break when surfaced off the repo page.
 */
const README_TAGS = [
  ...EDITORIAL_DOC_TAGS,
  "img",
];

export function renderProjectReadme(
  source: string,
  repoUrl: string | null | undefined,
): string {
  let rawBase: string | null = null;
  let htmlBase: string | null = null;
  if (repoUrl) {
    const m = /^https:\/\/github\.com\/([^/]+)\/([^/?#]+)/.exec(repoUrl);
    if (m) {
      const [, owner, repo] = m;
      rawBase = `https://raw.githubusercontent.com/${owner}/${repo}/HEAD/`;
      htmlBase = `https://github.com/${owner}/${repo}/blob/HEAD/`;
    }
  }

  const html = marked.parse(source, { gfm: true, breaks: false }) as string;
  return sanitizeHtml(html, {
    allowedTags: README_TAGS,
    allowedAttributes: {
      ...ALLOWED_ATTRS,
      "*": ["class", "id"],
      th: ["align"],
      td: ["align"],
      img: ["src", "alt", "title", "width", "height"],
    },
    // Default schemes apply to anchors and other URL-bearing tags. The
    // `data:` scheme is permitted ONLY on <img src> via
    // allowedSchemesByTag — letting it onto <a href> would let a README
    // ship `<a href="data:text/html,…">` and bounce the reader to an
    // attacker-shaped HTML payload.
    allowedSchemes: ["http", "https", "mailto"],
    allowedSchemesByTag: { img: ["http", "https", "data"] },
    transformTags: {
      a: (tagName, attribs) => {
        let href = attribs.href ?? "";
        if (
          htmlBase &&
          href &&
          !/^https?:\/\/|^\/\/|^mailto:|^#/.test(href)
        ) {
          href = htmlBase + href.replace(/^\.?\//, "");
        }
        const isExternal = /^https?:\/\//.test(href);
        return {
          tagName,
          attribs: {
            ...attribs,
            href,
            ...(isExternal
              ? { rel: "noopener noreferrer", target: "_blank" }
              : {}),
          },
        };
      },
      img: (tagName, attribs) => {
        let src = attribs.src ?? "";
        if (rawBase && src && !/^https?:\/\/|^\/\/|^data:|^#/.test(src)) {
          src = rawBase + src.replace(/^\.?\//, "");
        }
        return {
          tagName,
          attribs: { ...attribs, src, loading: "lazy" },
        };
      },
    },
  });
}

export function renderEditorialDoc(source: string): string {
  const html = marked.parse(source, { gfm: true, breaks: false }) as string;
  return sanitizeHtml(html, {
    allowedTags: EDITORIAL_DOC_TAGS,
    allowedAttributes: {
      ...ALLOWED_ATTRS,
      "*": ["class", "id"],
      th: ["align"],
      td: ["align"],
    },
    allowedSchemes: ["http", "https", "mailto"],
    transformTags: {
      // External links open new tab; internal /office links stay in-tab.
      a: (tagName, attribs) => {
        const href = attribs.href ?? "";
        const isExternal = /^https?:\/\//.test(href);
        return {
          tagName,
          attribs: isExternal
            ? { ...attribs, rel: "noopener noreferrer", target: "_blank" }
            : attribs,
        };
      },
    },
  });
}

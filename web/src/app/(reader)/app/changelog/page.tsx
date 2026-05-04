import { readFile } from "node:fs/promises";
import path from "node:path";
import { marked } from "marked";
import sanitizeHtml from "sanitize-html";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Changelog",
  description:
    "Release notes for ClauDepot, sourced from the repo's CHANGELOG.md.",
};

// Read the canonical CHANGELOG.md at build time. Local builds read it
// from `../CHANGELOG.md` (web/ sits next to it in the repo). Vercel
// builds with Root Directory = web/ don't see the parent — fall back
// to the GitHub raw URL. Repo is public so this is unauthenticated.
async function loadChangelog(): Promise<string> {
  const fsPath = path.resolve(process.cwd(), "..", "CHANGELOG.md");
  try {
    return await readFile(fsPath, "utf8");
  } catch {
    // GitHub raw fallback. cache:"no-store" because Vercel's Data
    // Cache otherwise persists the result across deploys (keyed on
    // URL+options), so a new build can't pick up the latest CHANGELOG
    // until the cache TTL expires. Per-request fetch is fine — this
    // is a docs page, not a hot path, and GitHub raw is sub-50ms.
    const r = await fetch(
      "https://raw.githubusercontent.com/xiaolai/claudepot-app/main/CHANGELOG.md",
      { cache: "no-store" },
    );
    if (!r.ok) {
      return "# Changelog\n\n_Changelog source unavailable right now._\n";
    }
    return r.text();
  }
}

const ALLOWED_TAGS = [
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "p",
  "ul",
  "ol",
  "li",
  "strong",
  "em",
  "code",
  "pre",
  "blockquote",
  "a",
  "hr",
  "br",
];

const ALLOWED_ATTRIBUTES: sanitizeHtml.IOptions["allowedAttributes"] = {
  a: ["href", "title", "rel"],
  code: ["class"],
};

export default async function ChangelogPage() {
  const raw = await loadChangelog();
  // marked is sync when called without async extensions.
  const html = marked.parse(raw, { async: false }) as string;
  const safe = sanitizeHtml(html, {
    allowedTags: ALLOWED_TAGS,
    allowedAttributes: ALLOWED_ATTRIBUTES,
    transformTags: {
      a: (tagName, attribs) => ({
        tagName,
        attribs: {
          ...attribs,
          rel: attribs.href?.startsWith("http") ? "noopener" : (attribs.rel ?? ""),
        },
      }),
    },
  });

  return (
    <article>
      <div dangerouslySetInnerHTML={{ __html: safe }} />
    </article>
  );
}

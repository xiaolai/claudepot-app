import { readFile } from "node:fs/promises";
import path from "node:path";
import Link from "next/link";
import { marked } from "marked";
import sanitizeHtml from "sanitize-html";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Changelog",
  description:
    "Release notes for ClauDepot, sourced from the repo's CHANGELOG.md.",
};

const REPO = "xiaolai/claudepot-app";
const VISIBLE_VERSIONS = 10;

// Read the canonical CHANGELOG.md. Local builds read it from
// `../CHANGELOG.md` (web/ sits next to it in the repo). Vercel
// builds with Root Directory = web/ don't see the parent — fall
// back to the GitHub raw URL. Repo is public so this is
// unauthenticated.
async function loadChangelog(): Promise<string> {
  const fsPath = path.resolve(process.cwd(), "..", "CHANGELOG.md");
  try {
    return await readFile(fsPath, "utf8");
  } catch {
    // cache:"no-store" because Vercel's Data Cache otherwise
    // persists across deploys (keyed on URL+options), so a new
    // build couldn't pick up the latest CHANGELOG until TTL.
    const r = await fetch(
      `https://raw.githubusercontent.com/${REPO}/main/CHANGELOG.md`,
      { cache: "no-store" },
    );
    if (!r.ok) {
      return "# Changelog\n\n_Changelog source unavailable right now._\n";
    }
    return r.text();
  }
}

interface ChangelogSection {
  heading: string;        // raw `## 0.1.6 — beta (2026-05-04)`
  version: string | null; // e.g. "0.1.6", null for pre-versioned preamble blocks
  body: string;           // markdown body (heading included)
}

/**
 * Split a CHANGELOG into sections at every `^## ` boundary. The
 * first chunk (the file preamble before the first version heading)
 * is returned with `version=null`.
 */
function splitChangelog(md: string): { preamble: string; versions: ChangelogSection[] } {
  const lines = md.split("\n");
  const versions: ChangelogSection[] = [];
  let preambleEnd = 0;
  // Find first `## ` line.
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].startsWith("## ")) {
      preambleEnd = i;
      break;
    }
  }
  const preamble = lines.slice(0, preambleEnd).join("\n");

  // Walk the rest, splitting at `## ` boundaries.
  let current: ChangelogSection | null = null;
  for (let i = preambleEnd; i < lines.length; i++) {
    const line = lines[i];
    if (line.startsWith("## ")) {
      if (current) versions.push(current);
      const versionMatch = line.match(/^##\s+(\d+\.\d+\.\d+)/);
      current = {
        heading: line,
        version: versionMatch?.[1] ?? null,
        body: line + "\n",
      };
    } else if (current) {
      current.body += line + "\n";
    }
  }
  if (current) versions.push(current);

  return { preamble, versions };
}

const ALLOWED_TAGS = [
  "h1", "h2", "h3", "h4", "h5", "h6",
  "p", "ul", "ol", "li", "strong", "em",
  "code", "pre", "blockquote", "a", "hr", "br",
];

const ALLOWED_ATTRIBUTES: sanitizeHtml.IOptions["allowedAttributes"] = {
  a: ["href", "title", "rel"],
  code: ["class"],
};

function renderMarkdown(md: string): string {
  const html = marked.parse(md, { async: false }) as string;
  return sanitizeHtml(html, {
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
}

export default async function ChangelogPage() {
  const raw = await loadChangelog();
  const { preamble, versions } = splitChangelog(raw);

  const recent = versions.slice(0, VISIBLE_VERSIONS);
  const older = versions.slice(VISIBLE_VERSIONS);

  // Render the file preamble + the recent sections as a single
  // markdown blob so heading rhythm matches what marked would
  // produce on the whole file.
  const recentMd = [preamble.trim(), ...recent.map((s) => s.body)]
    .filter(Boolean)
    .join("\n\n");
  const recentHtml = renderMarkdown(recentMd);

  return (
    <article>
      <div dangerouslySetInnerHTML={{ __html: recentHtml }} />

      {older.length > 0 && (
        <>
          <h2>Older versions</h2>
          <p>
            The {recent.length} most recent releases are listed above.
            For older versions, follow the link to the corresponding
            release page on GitHub.
          </p>
          <ul>
            {older.map((s) => {
              // Strip the markdown `## ` prefix for display.
              const label = s.heading.replace(/^##\s+/, "");
              if (!s.version) {
                return <li key={label}>{label}</li>;
              }
              return (
                <li key={s.version}>
                  <Link
                    href={`https://github.com/${REPO}/releases/tag/v${s.version}`}
                  >
                    {label}
                  </Link>
                </li>
              );
            })}
          </ul>
        </>
      )}
    </article>
  );
}

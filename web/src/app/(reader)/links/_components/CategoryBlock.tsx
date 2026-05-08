import Link from "next/link";

import type { Link as LinkRow, LinkCategory } from "@/db/schema/links";
import { LinkEntry } from "./LinkEntry";

type Props = {
  category: LinkCategory & { children: LinkCategory[] };
  links: Map<string, LinkRow[]>;
};

/**
 * For sections where a community-curated awesome list owns the long
 * tail, point readers there instead of trying to absorb 1000+ niche
 * entries into the directory. Keys are top-level category slugs.
 */
const SEE_ALSO_AWESOME: Record<
  string,
  { name: string; url: string; blurb: string }
> = {
  mcp: {
    name: "awesome-mcp-servers",
    url: "https://github.com/punkpeye/awesome-mcp-servers",
    blurb: "2,400+ community MCP servers",
  },
  skills: {
    name: "awesome-claude-code",
    url: "https://github.com/hesreallyhim/awesome-claude-code",
    blurb: "200+ community skills, hooks, commands",
  },
  "coding-tools": {
    name: "awesome-ai-agents",
    url: "https://github.com/e2b-dev/awesome-ai-agents",
    blurb: "900+ agent products + frameworks",
  },
  evals: {
    name: "Awesome-LLM",
    url: "https://github.com/Hannibal046/Awesome-LLM",
    blurb: "papers, models, leaderboards, courses",
  },
};

export function CategoryBlock({ category, links }: Props) {
  const childLinks = category.children.flatMap(
    (c) => links.get(c.slug) ?? [],
  );
  const directLinks = links.get(category.slug) ?? [];
  const total = childLinks.length + directLinks.length;

  if (total === 0) return null;

  return (
    <section
      id={category.slug}
      className="cat-block"
      data-category={category.slug}
    >
      <header className="cat-header">
        <h2>
          {category.name}
          <span className="cat-count">{total}</span>
        </h2>
        <Link href={`/links/c/${category.slug}`}>show all →</Link>
      </header>

      {category.children.length > 0 ? (
        category.children.map((child) => {
          const items = links.get(child.slug) ?? [];
          if (items.length === 0) return null;
          return (
            <div className="sub-block" key={child.id}>
              <h3>
                <Link href={`/links/c/${child.slug}`}>{child.name}</Link>
              </h3>
              <ul className="link-list">
                {items.map((l) => (
                  <LinkEntry key={l.id} link={l} />
                ))}
              </ul>
            </div>
          );
        })
      ) : (
        <ul className="link-list">
          {directLinks.map((l) => (
            <LinkEntry key={l.id} link={l} />
          ))}
        </ul>
      )}

      {SEE_ALSO_AWESOME[category.slug] ? (
        <p className="cat-block-see-also">
          See also:{" "}
          <a
            href={SEE_ALSO_AWESOME[category.slug].url}
            target="_blank"
            rel="noopener noreferrer"
          >
            {SEE_ALSO_AWESOME[category.slug].name}
          </a>{" "}
          — {SEE_ALSO_AWESOME[category.slug].blurb}
        </p>
      ) : null}
    </section>
  );
}

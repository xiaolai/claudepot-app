import type { Metadata } from "next";

import type { Link as LinkRow, LinkCategory } from "@/db/schema/links";
import { loadAllForGrid, loadFeatured } from "@/lib/links/queries";
import { CategoryBlock } from "./_components/CategoryBlock";
import { FeaturedStrip } from "./_components/FeaturedStrip";

type CategoryWithChildren = LinkCategory & { children: LinkCategory[] };

/**
 * Greedy weight-balanced column distribution. Walk categories in
 * source order; assign each to the column with the smallest current
 * total entry count. Result: roughly equal column heights, no
 * vacuum at column tops, deterministic order across requests.
 *
 * Reading order is column-major (top-to-bottom, then next column),
 * which fits the directory wall-of-links pattern.
 */
function distributeIntoColumns(
  cats: CategoryWithChildren[],
  linksByPrimary: Map<string, LinkRow[]>,
  columnCount = 3,
): CategoryWithChildren[][] {
  const cols: { cats: CategoryWithChildren[]; weight: number }[] = Array.from(
    { length: columnCount },
    () => ({ cats: [], weight: 0 }),
  );
  for (const c of cats) {
    const own = linksByPrimary.get(c.slug)?.length ?? 0;
    const childWeight = c.children.reduce(
      (s, ch) => s + (linksByPrimary.get(ch.slug)?.length ?? 0),
      0,
    );
    const target = cols.reduce(
      (min, col) => (col.weight < min.weight ? col : min),
      cols[0],
    );
    target.cats.push(c);
    target.weight += own + childWeight;
  }
  return cols.map((c) => c.cats);
}

export const metadata: Metadata = {
  title: "Links",
  description:
    "Curated directory of Claude, MCP, and AI links — frontier-lab docs, model providers, agent frameworks, evals, communities, and more.",
};

// Cache the data fetch — directory is editorial, doesn't change per
// request. Revalidate hourly so seed re-runs surface within an hour.
export const revalidate = 3600;

export default async function LinksPage() {
  const [grid, featured] = await Promise.all([
    loadAllForGrid(),
    loadFeatured(8),
  ]);

  const totalLinks = Array.from(grid.linksByPrimary.values()).reduce(
    (n, arr) => n + arr.length,
    0,
  );

  return (
    <div className="links-page">
      <header className="links-header">
        <h1>Links</h1>
        <p className="links-subhead">
          {totalLinks.toLocaleString()} curated Claude/AI links
          {" · "}
          {grid.topLevel.length} categories
        </p>
        <form className="links-search" action="/links/search" method="GET">
          <input
            type="search"
            name="q"
            placeholder="Search links…"
            aria-label="Search links"
          />
        </form>
      </header>

      <FeaturedStrip links={featured} />

      <nav className="links-pills" aria-label="Jump to category">
        <details className="links-pills-details">
          <summary className="links-pills-toggle">
            <span className="links-pills-toggle-label">Categories</span>
            <span className="links-pills-toggle-count">
              {grid.topLevel.length}
            </span>
          </summary>
          <div className="links-pills-list">
            {grid.topLevel.map((cat) => (
              <a key={cat.slug} href={`#${cat.slug}`} className="links-pill">
                {cat.name}
              </a>
            ))}
          </div>
        </details>
      </nav>

      <main className="links-grid">
        {distributeIntoColumns(grid.topLevel, grid.linksByPrimary, 3).map(
          (col, i) => (
            <div key={i} className="links-column">
              {col.map((cat) => (
                <CategoryBlock
                  key={cat.id}
                  category={cat}
                  links={grid.linksByPrimary}
                />
              ))}
            </div>
          ),
        )}
      </main>

      <footer className="links-footer">
        <a href="/links/suggest">Suggest a link →</a>
        <a
          href="https://github.com/xiaolai/claudepot-app"
          target="_blank"
          rel="noopener noreferrer"
        >
          Source on GitHub
        </a>
      </footer>
    </div>
  );
}

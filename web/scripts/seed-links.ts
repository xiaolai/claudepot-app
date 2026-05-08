/**
 * Seed `link_categories` + `links` from dev-docs/directory-research-deduped.md.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/seed-links.ts
 *
 * Idempotent:
 *   - link_categories  ON CONFLICT (slug) DO UPDATE
 *   - links            ON CONFLICT (url)  DO UPDATE
 *
 * Re-running picks up edits to the markdown without producing duplicates.
 *
 * See dev-docs/links-page-design.md for the design.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { neon } from "@neondatabase/serverless";

const MD_PATH = resolve(
  process.cwd(),
  "../dev-docs/directory-research-deduped.md",
);

// Hand-curated short slugs for the 13 top-level sections — keeps the
// `/links/c/[slug]/` URLs short. Subcategory slugs are derived from the
// `### …` heading (kebab + 4-word cap, prefixed with the parent slug).
const TOP_LEVEL_SLUGS: Record<number, string> = {
  1: "anthropic",
  2: "mcp",
  3: "coding-tools",
  4: "model-providers",
  5: "evals",
  6: "learning",
  7: "community",
  8: "infra",
  9: "china",
  10: "multimodal",
  11: "policy",
  12: "builder-tools",
  13: "skills",
};

type Cat = {
  slug: string;
  name: string;
  parentSlug: string | null;
  displayOrder: number;
  region: string | null;
};

type Link = {
  slug: string;
  name: string;
  url: string;
  description: string;
  primaryCategorySlug: string;
  categorySlugs: string[];
  region: string | null;
  displayOrder: number;
};

function kebab(s: string, maxWords = 6): string {
  return s
    .toLowerCase()
    .replace(/[—–]/g, "-")
    .replace(/[^\w\s-]/g, " ")
    .trim()
    .split(/\s+/)
    .slice(0, maxWords)
    .join("-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

function uniqueSlug(base: string, taken: Set<string>): string {
  if (!taken.has(base)) {
    taken.add(base);
    return base;
  }
  let i = 2;
  while (taken.has(`${base}-${i}`)) i += 1;
  const out = `${base}-${i}`;
  taken.add(out);
  return out;
}

function hasCJK(s: string): boolean {
  return /[　-鿿＀-￯]/.test(s);
}

function parse(md: string): { cats: Cat[]; links: Link[] } {
  const cats: Cat[] = [];
  const links: Link[] = [];
  const linkSlugs = new Set<string>();

  let topNum: number | null = null;
  let topSlug: string | null = null;
  let topRegion: string | null = null;
  let subSlug: string | null = null;
  let subOrder = 0;
  let topDisplayOrder = 0;
  let entryOrder = 0;

  const ENTRY_RE =
    /^- \[([^\]]+)\]\(([^)]+)\)(?:\s*[—-]\s*(.+?))?(?:\s+↗\s+also in\s+(.+?))?$/;

  for (const raw of md.split("\n")) {
    const line = raw.trimEnd();

    // Top-level: "## N. Title"
    const h2 = line.match(/^## (\d+)\.\s+(.+)$/);
    if (h2) {
      topNum = Number(h2[1]);
      const title = h2[2];
      topSlug = TOP_LEVEL_SLUGS[topNum];
      if (!topSlug) {
        throw new Error(`No slug mapping for §${topNum} ("${title}")`);
      }
      topRegion = topNum === 9 ? "cn" : null;
      topDisplayOrder += 1;
      cats.push({
        slug: topSlug,
        name: title,
        parentSlug: null,
        displayOrder: topDisplayOrder,
        region: topRegion,
      });
      subSlug = null;
      subOrder = 0;
      entryOrder = 0;
      continue;
    }

    // Subsection: "### Title"
    const h3 = line.match(/^### (.+)$/);
    if (h3 && topSlug) {
      const title = h3[1];
      subOrder += 1;
      const child = kebab(title, 4);
      subSlug = `${topSlug}-${child}`;
      cats.push({
        slug: subSlug,
        name: title,
        parentSlug: topSlug,
        displayOrder: subOrder,
        region: topRegion,
      });
      entryOrder = 0;
      continue;
    }

    // Entry: "- [Name](url) — desc  ↗ also in §N, §M"
    const m = line.match(ENTRY_RE);
    if (m && topSlug) {
      const [, name, url, desc, alsoIn] = m;
      entryOrder += 1;

      const primary = subSlug ?? topSlug;
      const ancestors: string[] = [primary];
      if (subSlug && topSlug) ancestors.push(topSlug);

      const cross: string[] = [];
      if (alsoIn) {
        for (const ref of alsoIn.split(/[,\s]+/)) {
          const sm = ref.match(/§(\d+)/);
          if (sm) {
            const otherTop = TOP_LEVEL_SLUGS[Number(sm[1])];
            if (otherTop && !ancestors.includes(otherTop)) {
              cross.push(otherTop);
            }
          }
        }
      }

      const categorySlugs = [...new Set([...ancestors, ...cross])];

      const slug = uniqueSlug(kebab(name, 8) || "link", linkSlugs);
      const region = topRegion ?? (hasCJK(name) ? "cn" : null);

      links.push({
        slug,
        name,
        url,
        description: desc ?? "",
        primaryCategorySlug: primary,
        categorySlugs,
        region,
        displayOrder: entryOrder,
      });
    }
  }

  return { cats, links };
}

async function main() {
  const url = process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;
  if (!url) {
    console.error("missing DATABASE_URL / NEON_DATABASE_URL");
    process.exit(1);
  }

  const md = readFileSync(MD_PATH, "utf8");
  const { cats, links } = parse(md);

  console.log(
    `Parsed ${cats.length} categories ` +
      `(${cats.filter((c) => !c.parentSlug).length} top-level, ` +
      `${cats.filter((c) => c.parentSlug).length} sub) ` +
      `and ${links.length} links from ${MD_PATH}.`,
  );

  const sql = neon(url);

  // Top-level first, then subcategories — parent_id requires the parent
  // row to exist already.
  const topCats = cats.filter((c) => !c.parentSlug);
  const subCats = cats.filter((c) => c.parentSlug);

  console.log(`Upserting ${topCats.length} top-level categories…`);
  for (const c of topCats) {
    await sql`
      INSERT INTO link_categories (slug, name, parent_id, display_order, region)
      VALUES (${c.slug}, ${c.name}, NULL, ${c.displayOrder}, ${c.region})
      ON CONFLICT (slug) DO UPDATE SET
        name          = EXCLUDED.name,
        display_order = EXCLUDED.display_order,
        region        = EXCLUDED.region
    `;
  }

  console.log(`Upserting ${subCats.length} subcategories…`);
  for (const c of subCats) {
    await sql`
      INSERT INTO link_categories (slug, name, parent_id, display_order, region)
      VALUES (
        ${c.slug},
        ${c.name},
        (SELECT id FROM link_categories WHERE slug = ${c.parentSlug}),
        ${c.displayOrder},
        ${c.region}
      )
      ON CONFLICT (slug) DO UPDATE SET
        name          = EXCLUDED.name,
        parent_id     = EXCLUDED.parent_id,
        display_order = EXCLUDED.display_order,
        region        = EXCLUDED.region
    `;
  }

  console.log(`Upserting ${links.length} links…`);
  let i = 0;
  for (const l of links) {
    await sql`
      INSERT INTO links (
        slug, name, url, description,
        primary_category_slug, category_slugs,
        region, display_order
      )
      VALUES (
        ${l.slug}, ${l.name}, ${l.url}, ${l.description},
        ${l.primaryCategorySlug}, ${l.categorySlugs},
        ${l.region}, ${l.displayOrder}
      )
      ON CONFLICT (url) DO UPDATE SET
        name                  = EXCLUDED.name,
        description           = EXCLUDED.description,
        primary_category_slug = EXCLUDED.primary_category_slug,
        category_slugs        = EXCLUDED.category_slugs,
        region                = EXCLUDED.region,
        display_order         = EXCLUDED.display_order,
        updated_at            = NOW()
    `;
    i += 1;
    if (i % 100 === 0) console.log(`  ${i}/${links.length}…`);
  }

  const [{ catCount }] = await sql`
    SELECT COUNT(*)::int AS "catCount" FROM link_categories
  `;
  const [{ linkCount }] = await sql`
    SELECT COUNT(*)::int AS "linkCount" FROM links
  `;
  console.log(`Done. DB now has ${catCount} categories and ${linkCount} links.`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});

/**
 * DB-backed queries for /links/.
 *
 *   loadAllForGrid()    — categories + grouped active links for the
 *                         main hao123-style page.
 *   loadFeatured(n=8)   — editor's-picks strip.
 *   loadCategoryPage()  — one category (top-level or sub) + its links
 *                         + parent + sibling/child categories.
 *   searchLinks(q, n)   — FTS hit list ranked by ts_rank_cd.
 *
 * All queries filter `links.status = 'active'` unless noted.
 */

import { and, asc, desc, eq, isNotNull, sql } from "drizzle-orm";

import { db } from "@/db/client";
import {
  linkCategories,
  links,
  type Link,
  type LinkCategory,
} from "@/db/schema/links";
import { users } from "@/db/schema/users";

export type CategoryWithChildren = LinkCategory & {
  children: LinkCategory[];
};

export type CategoryWithLinks = LinkCategory & {
  children: LinkCategory[];
  links: Link[];
};

/**
 * Single fetch sized for the main /links/ page: every active link plus
 * the full category tree. The grid composes them in TS — one DB round
 * trip is plenty at this scale (~1k links, ~100 categories).
 */
export async function loadAllForGrid(): Promise<{
  topLevel: CategoryWithChildren[];
  linksByPrimary: Map<string, Link[]>;
}> {
  const [allCategories, allLinks] = await Promise.all([
    db
      .select()
      .from(linkCategories)
      .orderBy(asc(linkCategories.displayOrder), asc(linkCategories.name)),
    db
      .select()
      .from(links)
      .where(eq(links.status, "active"))
      .orderBy(
        asc(links.primaryCategorySlug),
        asc(links.displayOrder),
        asc(links.name),
      ),
  ]);

  const byParent = new Map<number, LinkCategory[]>();
  const topLevel: LinkCategory[] = [];
  for (const c of allCategories) {
    if (c.parentId === null) topLevel.push(c);
    else {
      const arr = byParent.get(c.parentId) ?? [];
      arr.push(c);
      byParent.set(c.parentId, arr);
    }
  }

  const withChildren: CategoryWithChildren[] = topLevel.map((t) => ({
    ...t,
    children: byParent.get(t.id) ?? [],
  }));

  const linksByPrimary = new Map<string, Link[]>();
  for (const l of allLinks) {
    const arr = linksByPrimary.get(l.primaryCategorySlug) ?? [];
    arr.push(l);
    linksByPrimary.set(l.primaryCategorySlug, arr);
  }

  return { topLevel: withChildren, linksByPrimary };
}

export async function loadFeatured(limit = 8): Promise<Link[]> {
  return db
    .select()
    .from(links)
    .where(and(eq(links.status, "active"), isNotNull(links.featuredRank)))
    .orderBy(asc(links.featuredRank))
    .limit(limit);
}

/**
 * Single category page. Resolves either a top-level category (returns
 * children + their links) or a subcategory (returns just its links and
 * the parent for breadcrumbs).
 */
export async function loadCategoryPage(slug: string): Promise<{
  category: LinkCategory;
  parent: LinkCategory | null;
  children: LinkCategory[];
  links: Link[];
} | null> {
  const cat = await db
    .select()
    .from(linkCategories)
    .where(eq(linkCategories.slug, slug))
    .limit(1);
  if (cat.length === 0) return null;
  const category = cat[0];

  let parent: LinkCategory | null = null;
  if (category.parentId !== null) {
    const p = await db
      .select()
      .from(linkCategories)
      .where(eq(linkCategories.id, category.parentId))
      .limit(1);
    parent = p[0] ?? null;
  }

  const children = await db
    .select()
    .from(linkCategories)
    .where(eq(linkCategories.parentId, category.id))
    .orderBy(asc(linkCategories.displayOrder), asc(linkCategories.name));

  // category_slugs[] holds the full ancestor chain + cross-listings, so
  // matching against it gives us "everything in this category", whether
  // primary or cross-listed.
  const rows = await db
    .select()
    .from(links)
    .where(
      and(
        eq(links.status, "active"),
        sql`${slug} = ANY(${links.categorySlugs})`,
      ),
    )
    .orderBy(asc(links.displayOrder), asc(links.name));

  return { category, parent, children, links: rows };
}

/**
 * FTS over name (A) > description (B) > tags (C) > primary_category (D).
 * Uses websearch_to_tsquery('simple', q) to forgive partial input
 * (single keywords, OR-joined phrases).
 */
export type PendingLinkRow = {
  link: Link;
  suggester: { username: string | null; name: string | null } | null;
};

/**
 * Curator-queue feed: every `status='pending'` row, newest first,
 * with a left-join to `users` so the suggester's username/name
 * surface in the UI. Used by /admin/links.
 */
export async function loadPendingSuggestions(
  limit = 200,
): Promise<PendingLinkRow[]> {
  const rows = await db
    .select({
      link: links,
      username: users.username,
      name: users.name,
    })
    .from(links)
    .leftJoin(users, eq(users.id, links.suggestedBy))
    .where(eq(links.status, "pending"))
    .orderBy(desc(links.createdAt))
    .limit(limit);

  return rows.map((r) => ({
    link: r.link,
    suggester:
      r.link.suggestedBy && (r.username || r.name)
        ? { username: r.username, name: r.name }
        : null,
  }));
}

export async function searchLinks(q: string, limit = 50): Promise<Link[]> {
  const trimmed = q.trim();
  if (!trimmed) return [];

  return db
    .select()
    .from(links)
    .where(
      and(
        eq(links.status, "active"),
        sql`${links.searchVec} @@ websearch_to_tsquery('simple', ${trimmed})`,
      ),
    )
    .orderBy(
      desc(
        sql`ts_rank_cd(${links.searchVec}, websearch_to_tsquery('simple', ${trimmed}))`,
      ),
      asc(links.name),
    )
    .limit(limit);
}

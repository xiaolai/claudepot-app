/**
 * Curated /links/ directory — the hao123-style Claude/AI link wall
 * at claudepot.com/links/.
 *
 * Two tables:
 *   link_categories — hierarchical taxonomy (top-level §N + ###
 *                     subsections via parent_id self-ref)
 *   links           — ~1,036 curated entries, one row per canonical URL
 *
 * See dev-docs/links-page-design.md for the design rationale.
 *
 * The `search_vec` column on links is a `tsvector GENERATED ALWAYS AS
 * (…) STORED` declared in migration 0033_links.sql. Drizzle has no
 * first-class support for generated columns, so it's declared here as
 * opaque tsvector via the customType pattern from content.ts. Without
 * this, drizzle-kit push would see a column that exists in the DB but
 * not in the schema and DROP it.
 *
 * Per .claude/rules/db-migrations.md, prod migrations apply via
 * web/scripts/apply-migration.ts (Neon HTTP driver). Never push.
 */

import {
  boolean,
  customType,
  index,
  integer,
  pgTable,
  serial,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";
import { sql } from "drizzle-orm";

/**
 * Opaque tsvector type — see header.
 */
const tsvector = customType<{ data: string; driverData: string }>({
  dataType: () => "tsvector",
});

export const linkCategories = pgTable(
  "link_categories",
  {
    id: serial("id").primaryKey(),
    slug: text("slug").notNull().unique(),
    name: text("name").notNull(),
    description: text("description"),
    // Self-FK on parent_id is added by migration 0033_links.sql. We
    // omit `.references()` here both to avoid the circular-reference
    // dance and to follow the existing convention in content.ts
    // (comments.parent_id is declared the same way).
    parentId: integer("parent_id"),
    displayOrder: integer("display_order").notNull().default(0),
    region: text("region"),
    icon: text("icon"),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_link_categories_parent_order").on(t.parentId, t.displayOrder),
  ],
);

/**
 * Provenance values accepted by `links.status`. The matching CHECK
 * constraint lives in migration 0033_links.sql — keep these two in
 * sync if you add a fourth state.
 *
 *   active   — visible on the public grid
 *   archived — hidden but kept for history
 *   broken   — link rot detected; surfaced in admin only
 *   pending  — submitted by a reader, awaiting curation
 */
export const LINK_STATUSES = ["active", "archived", "broken", "pending"] as const;
export type LinkStatus = (typeof LINK_STATUSES)[number];

export const links = pgTable(
  "links",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    slug: text("slug").notNull().unique(),
    name: text("name").notNull(),
    url: text("url").notNull().unique(),
    description: text("description").notNull().default(""),
    primaryCategorySlug: text("primary_category_slug")
      .notNull()
      .references(() => linkCategories.slug, {
        onDelete: "restrict",
        onUpdate: "cascade",
      }),
    // Includes primaryCategorySlug; covers cross-listings ("↗ also in
    // §N" annotations from the deduped markdown). GIN-indexed in the
    // migration for category-slug lookups.
    categorySlugs: text("category_slugs")
      .array()
      .notNull()
      .default(sql`'{}'::text[]`),
    tags: text("tags").array().notNull().default(sql`'{}'::text[]`),
    region: text("region"),
    isOfficial: boolean("is_official").notNull().default(false),
    displayOrder: integer("display_order").notNull().default(0),
    featuredRank: integer("featured_rank"),
    featuredBlurb: text("featured_blurb"),
    status: text("status").notNull().default("active").$type<LinkStatus>(),
    // FK to users.id added by migration 0035_links_suggested_by.sql.
    // Nullable — seeded links pre-date this column; only links coming
    // from the /links/suggest form populate it.
    suggestedBy: uuid("suggested_by"),
    lastCheckedAt: timestamp("last_checked_at", { withTimezone: true }),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true }).notNull().defaultNow(),
    // GENERATED column owned by the migration. See header.
    searchVec: tsvector("search_vec"),
  },
  (t) => [
    // Public-grid B-tree index. GIN indexes on category_slugs / tags /
    // search_vec live in the migration only — Drizzle's schema DSL
    // doesn't express GIN cleanly (the same pattern submissions.ts
    // follows for its FTS index in 0003_fts.sql).
    index("idx_links_primary_category")
      .on(t.primaryCategorySlug, t.displayOrder, t.name)
      .where(sql`${t.status} = 'active'`),
    index("idx_links_featured")
      .on(t.featuredRank)
      .where(sql`${t.featuredRank} IS NOT NULL`),
  ],
);

export type LinkCategory = typeof linkCategories.$inferSelect;
export type NewLinkCategory = typeof linkCategories.$inferInsert;
export type Link = typeof links.$inferSelect;
export type NewLink = typeof links.$inferInsert;

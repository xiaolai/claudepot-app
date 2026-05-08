-- 0033_links — curated /links/ directory for claudepot.com.
--
-- Two tables behind the hao123-style page at claudepot.com/links/:
--
--   link_categories — hierarchical taxonomy (top-level §N + ### subsections)
--   links           — ~1,036 entries, one row per canonical URL
--
-- See dev-docs/links-page-design.md for the design rationale and
-- decision log (no junction table, slug-as-FK, 'simple' tsvector
-- config, separate from `submissions`).
--
-- Re-runnable: every CREATE uses IF NOT EXISTS. The seed script
-- web/scripts/seed-links.ts uses ON CONFLICT (url) DO UPDATE on the
-- links table so re-running it is idempotent too.

CREATE TABLE IF NOT EXISTS "link_categories" (
  "id"            SERIAL PRIMARY KEY,
  "slug"          TEXT NOT NULL UNIQUE,
  "name"          TEXT NOT NULL,
  "description"   TEXT,
  "parent_id"     INTEGER REFERENCES "link_categories" ("id") ON DELETE CASCADE,
  "display_order" INTEGER NOT NULL DEFAULT 0,
  "region"        TEXT,
  "icon"          TEXT,
  "created_at"    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_link_categories_parent_order"
  ON "link_categories" ("parent_id", "display_order");
--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "links" (
  "id"                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  "slug"                  TEXT NOT NULL UNIQUE,
  "name"                  TEXT NOT NULL,
  "url"                   TEXT NOT NULL UNIQUE,
  "description"           TEXT NOT NULL DEFAULT '',
  "primary_category_slug" TEXT NOT NULL
                          REFERENCES "link_categories" ("slug")
                          ON DELETE RESTRICT
                          ON UPDATE CASCADE,
  "category_slugs"        TEXT[] NOT NULL DEFAULT '{}'::TEXT[],
  "tags"                  TEXT[] NOT NULL DEFAULT '{}'::TEXT[],
  "region"                TEXT,
  "is_official"           BOOLEAN NOT NULL DEFAULT FALSE,
  "display_order"         INTEGER NOT NULL DEFAULT 0,
  "featured_rank"         INTEGER,
  "featured_blurb"        TEXT,
  "status"                TEXT NOT NULL DEFAULT 'active'
                          CHECK ("status" IN ('active', 'archived', 'broken', 'pending')),
  "last_checked_at"       TIMESTAMPTZ,
  "created_at"            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  "updated_at"            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
--> statement-breakpoint

-- Full-text search column. Weights:
--   A = name (most important; what users type first)
--   B = description (one-line blurb)
--   C = tags (verbatim lexemes via array_to_tsvector)
--   D = primary_category_slug (so "mcp" and "skills" act as filters)
--
-- Config is 'simple' — Porter stemming on 'english' swallows technical
-- terms like MCP, Anthropic, Cline. Revisit if multilingual queries
-- matter (CJK names need their own setup; for now the english tokenizer
-- silently drops them and we rely on description / tags coverage).
--
-- Why `array_to_tsvector(tags)` instead of `to_tsvector('simple',
-- array_to_string(tags, ' '))`: `array_to_string` is STABLE (not
-- IMMUTABLE) because element output can depend on locale, and a
-- GENERATED ALWAYS AS expression must be IMMUTABLE. `array_to_tsvector`
-- is immutable — it converts each array element to a lexeme directly,
-- which is fine for our slug-shaped tags. No coalesces because every
-- source column is NOT NULL (with defaults where applicable).
--
-- The schema file (web/src/db/schema/links.ts) declares this column as
-- opaque tsvector via customType so drizzle-kit push doesn't see a
-- column it can't express and try to drop it (per
-- .claude/rules/db-migrations.md).

ALTER TABLE "links"
  ADD COLUMN IF NOT EXISTS "search_vec" tsvector
  GENERATED ALWAYS AS (
    setweight(to_tsvector('simple', "name"),                  'A') ||
    setweight(to_tsvector('simple', "description"),           'B') ||
    setweight(array_to_tsvector("tags"),                      'C') ||
    setweight(to_tsvector('simple', "primary_category_slug"), 'D')
  ) STORED;
--> statement-breakpoint

-- Public-grid index: filter status='active', group by primary category,
-- order by editor display_order then name.
CREATE INDEX IF NOT EXISTS "idx_links_primary_category"
  ON "links" ("primary_category_slug", "display_order", "name")
  WHERE "status" = 'active';
--> statement-breakpoint

-- Cross-listing lookups: "show me everything in category X" where X may
-- be the primary or a secondary category.
CREATE INDEX IF NOT EXISTS "idx_links_category_slugs"
  ON "links" USING GIN ("category_slugs");
--> statement-breakpoint

-- Tag chip filters: "show me everything tagged 'awesome-list'".
CREATE INDEX IF NOT EXISTS "idx_links_tags"
  ON "links" USING GIN ("tags");
--> statement-breakpoint

-- FTS.
CREATE INDEX IF NOT EXISTS "idx_links_search_vec"
  ON "links" USING GIN ("search_vec");
--> statement-breakpoint

-- Featured strip on /links/. Partial because the vast majority of rows
-- will have featured_rank IS NULL.
CREATE INDEX IF NOT EXISTS "idx_links_featured"
  ON "links" ("featured_rank")
  WHERE "featured_rank" IS NOT NULL;

-- 0043_comments_body_trgm.sql
--
-- Comment search (GET /api/v1/search?kind=comment,
-- src/lib/api/queries.ts) filters with `comments.body ILIKE '%…%'`.
-- Without an index that is a sequential scan over every comment row
-- per search; a pg_trgm GIN index services the leading-wildcard
-- ILIKE directly.
--
-- pg_trgm is a Postgres "trusted" extension (Neon allows
-- CREATE EXTENSION from the app role). No earlier migration creates
-- it — 0003_fts uses the built-in tsvector machinery, not trigrams —
-- so it is created here.
--
-- Apply via `pnpm exec tsx --env-file=.env.local
-- scripts/apply-migration.ts src/db/migrations/0043_comments_body_trgm.sql`
-- per .claude/rules/db-migrations.md. NEVER drizzle-kit push.

CREATE EXTENSION IF NOT EXISTS pg_trgm;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_comments_body_trgm
  ON comments USING gin (body gin_trgm_ops);

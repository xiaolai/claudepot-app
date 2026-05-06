# Database migrations — never use `drizzle-kit push` against prod

The Neon-backed `web/` app has hand-authored SQL migrations in
`web/src/db/migrations/*.sql`, plus DB-level features (FTS-generated
columns, partial unique indexes, triggers) that **Drizzle's schema
DSL cannot fully represent**. Running `pnpm drizzle-kit push` against
a database with those features will **drop** them, because push
diffs the DB against the schema files and treats anything not in the
schema as something to remove.

This is not theoretical. On 2026-05-06, `drizzle-kit push` was used to
apply migrations 0018–0021 against prod (because `drizzle-kit migrate`
was hanging over the HTTP-only Neon driver). The push **dropped
`submissions.search_vec`** (the FTS-generated column added by
migration `0003_fts`) and the matching `idx_submissions_search_vec`
GIN index, breaking site search until restored manually.

## The rule

For ANY production schema change:

1. **Add a numbered migration file** under
   `web/src/db/migrations/<NNNN>_<slug>.sql` with the exact DDL.
2. **Apply it via `psql` or via the Neon dashboard SQL editor**
   pointed at the prod connection string. Never push against prod.
3. **Update `_journal.json`** so the migration is recorded.
4. **Mirror the change** in the relevant `web/src/db/schema/*.ts`
   file so type inference works. Use `customType` for DB features
   Drizzle's DSL can't express (FTS columns, generated columns,
   trigger-maintained columns). The schema declaration is the
   "leave this alone" signal to push, not the source of truth.

## Why not just `drizzle-kit migrate`?

Migrate uses transactions, which require websockets. The
`@neondatabase/serverless` HTTP driver doesn't support them, so
`migrate` hangs indefinitely under the default config. Switching the
driver for migrations alone is more work than just using `psql`.

If a future contributor needs migrate to work, the path is:

- Set up a websocket-capable Neon connection in a one-off node
  script that imports `@neondatabase/serverless` with
  `neonConfig.webSocketConstructor = ws` set.
- Use the `migrate()` helper from `drizzle-orm/neon-serverless/migrator`.

But for one-off prod migrations, `psql` is faster and more auditable.

## Existing things push will want to drop

These DB-level features are not represented in
`web/src/db/schema/*.ts` (or are represented as opaque types push
can match against):

- `submissions.search_vec` — `tsvector GENERATED ALWAYS AS (…) STORED`,
  declared as opaque `tsvector` in `schema/content.ts` so push
  matches.
- `idx_submissions_search_vec` — GIN index over `search_vec`. Push
  will recreate this with the same name once the column is in
  schema.
- The `score_after_vote_change` trigger and any other triggers
  added by `0001_triggers` and friends — push doesn't manage
  triggers, but won't drop them either as long as they're attached
  to existing tables.

If you add a new generated column or trigger-maintained column,
update this rule and add the matching `customType` declaration in
the schema.

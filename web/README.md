# claudepot-com (web/)

In-tree Next.js app for **[claudepot.com](https://claudepot.com)** — the
ClauDepot resource hub for one-man companies (OMCs) building with AI.
Lives inside `claudepot-app/web/` next to the Tauri product.

> **Status (2026-05-04).** Phase 1 of the migration from `sha.nnon.ai`
> is complete: the Next.js app was ported by copy, fresh Neon DB
> provisioned, fresh Vercel project linked. Reader rebrand from
> "shannon" → "ClauDepot" in progress. See
> `../dev-docs/domain-realignment.md` for the full plan.

## Two surfaces, one app

| Path | What lives there |
|---|---|
| `/` | The **reader** — daily resource aggregator for OMCs (was sha.nnon.ai) |
| `/app/` | The **product docs** for the ClauDepot Tauri app — fresh MDX, no port from VitePress |

Cross-promotion happens at the layout level: every reader page footer-links to `/app/`, every docs page links back to the reader.

## Stack

- **Next.js 15** (App Router, Node 24 LTS, `output: standalone`)
- **Auth.js v5** with `@auth/drizzle-adapter` — GitHub + Google OAuth, Resend magic link
- **Drizzle 0.45** against **Neon serverless Postgres** (project `claudepot-com`, `summer-block-93558222`)
- **Resend** for transactional mail (sender domain migration to `claudepot.com` is Phase 3)
- **`boring-avatars`** for user avatars (the Shannon pixel-avatar pipeline is retired per locked decision #2)
- **`marked`** + **`sanitize-html`** for the comment / submission body renderer

## Repository layout

| Path | What lives there |
|---|---|
| `src/app/(reader)/` | Reader routes — feed, submission detail, profile, tags |
| `src/app/app/` | Product docs subpath — MDX, fresh design (Phase 4 of the plan) |
| `src/app/api/` | Auth.js handlers, RSS feeds, OG image, `cron/{daily-rollup,digest-weekly}` |
| `src/components/` | Reusable UI |
| `src/db/` | Drizzle schema, queries, migrations |
| `src/lib/` | Auth config, server actions, markdown renderer, username library |
| `src/styles/` | `theme.css` (tokens) + `prototype.css` (component classes) |
| `design/` | Architecture / IA decision docs + `fixtures/*.json` for local seed |
| `editorial/` | Editorial spec read at runtime by the bot office (`audience.md`, `rubric.yml`, …) |
| `scripts/` | Dev / ops tooling (sync projects, seed, apply migrations) |
| `tests/` | `tsx`-runner tests |

The current `(prototype)` route-group name is a holdover from Shannon and will be renamed to `(reader)` in a follow-up pass.

## Local dev

```bash
nvm use                   # picks up .nvmrc (Node 24 LTS)
pnpm install              # uses pinned pnpm 10
docker compose up -d      # local Postgres on :5432 (offline only; prod is Neon)
pnpm dev                  # http://localhost:3000
```

The Neon connection string lives in `.env.local` (gitignored). For a
fresh clone, ask for the env or provision your own Neon project and run
the migration loop:

```bash
for f in src/db/migrations/0*.sql; do
  pnpm exec tsx --env-file=.env.local scripts/apply-migration.ts "$f"
done
```

Other useful scripts:

```bash
pnpm build                # production build
pnpm test                 # tsx test runner
pnpm projects:sync        # tsx scripts/sync-projects.ts (gh CLI required)
pnpm projects:seed        # same with --seed and .env.local loaded
```

**Node:** 24 LTS pinned via `.nvmrc` and enforced by `engines.node`.
**Package manager:** `pnpm@10`.

## Conventions

- Token-only CSS — values live in `:root` in `src/styles/theme.css`; never hardcode dimensions in `prototype.css`.
- Reusable UI lives in `src/components/` and is prefixed `.proto-` in CSS (the prefix will migrate alongside the route-group rename).
- Server actions live in `src/lib/actions/<domain>.ts` and authenticate via `auth()` from `@/lib/auth`.
- Drizzle queries via `@/db/queries` are the source of truth.

See `../CLAUDE.md` and `../dev-docs/domain-realignment.md` for the cross-repo plan and decision log.

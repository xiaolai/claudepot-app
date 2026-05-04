# ClauDepot v2 — application architecture

> Companion to `features.md` (what's in/out, with mechanism), `implementation-plan.md` (build sequencing), and `IA.md` (information architecture). This doc covers the **stack, data model, auth, page→action map, environment**. AI-moderation, bot-as-named-AI-personas, and editorial briefs are explicitly deferred — the schema leaves room for them but no v1 code writes those tables.

> **Build status (2026-04-30).** Stack §3 lists target choices; §3a tracks what's wired today vs. still planned. Folder layout §9 reflects the *current* prototype-branch reality — the originally-planned `(public)/(authed)/(admin)/` route-group split has not been adopted; everything lives in a single `(prototype)/` group. `vercel.ts` is referenced as the future config; the file currently in use is `vercel.json`.

## 1. Product, in one paragraph

ClauDepot is a daily reader for builders working with AI tools. Tag-based IA. Users submit links and short writeups (news, tips, tutorials, courses, articles, podcasts, interviews, tools, discussions). Hot/new/top feeds, vote (public signal) + save (private bookmark) split, threaded comments, profiles, projects, tags. **Moderation in v1 is fully manual** — staff review flagged content via `/admin/queue`, with a public action log at `/admin/log`. AI moderation is on the roadmap; the schema reserves space for it (`ai_decisions`, `moderation_overrides`) but those tables are unwritten in v1. No DMs, no real-time, no mobile native, no federation.

## 2. Hard constraints

- Solo developer with AI execution — keep operational surface small.
- Vercel-native — no separate workers / runners outside Vercel primitives.
- Field-journal pipeline + Payload + unused deps were stripped on 2026-04-29 (the `main` orphan-cut + the design-branch excisions). This branch builds forward from that baseline.
- Prototype data shapes are stable and have become the schema.
- **No public exposure until anti-spam hardening lands** (see implementation-plan §Phase 10). Earlier phases ship to preview deploys only, optionally password-gated.

## 3. Stack

| Layer | Choice | Rationale |
|---|---|---|
| Runtime | Next.js 15 App Router on Vercel (Fluid Compute) | Already in place; RSC fits "render from DB" |
| Database | Neon Postgres via Vercel Marketplace | Relational, branch-per-preview, zero-ops |
| ORM | Drizzle | Type-safe, no codegen runtime, thin |
| Auth | Auth.js v5 + `@auth/drizzle-adapter` | Email magic-link + GitHub OAuth (matches `/login` mock) |
| Background jobs | Vercel Cron | Daily/weekly only; no queues needed in v1 (no AI moderation) |
| Storage | Vercel Blob | Avatars; submission thumbnails deferred |
| Rate limit | Upstash Redis (Marketplace) | Sliding-window per user / IP |
| Bot defense | Vercel BotID | Frontend abuse defense layer |
| Observability | Vercel Analytics + custom `metrics_daily` rollup | Cheap, native |
| Email | Resend (Marketplace) | Magic-links, reply notifications, weekly digest |

**Tier requirements at production cutover**: Vercel **Pro** ($20/mo — needed for sub-daily crons and >1 cron job) and Resend **Pro** (free tier is 100/day; magic-link auth alone exceeds this with any real signup volume). Plan budget accordingly.

**Avoided**:
- Prisma — heavier than Drizzle, codegen step, slower cold starts
- Inngest, Vercel Queues — not needed in v1; no async pipeline
- AI Gateway, Vercel AI SDK — deferred with AI moderation
- Clerk — Auth.js + DrizzleAdapter solves the same problem without the lock-in
- Edge runtime — Fluid Compute on Node is the current Vercel default

## 3a. What's wired today vs. planned

| Component | Status | Notes |
|---|---|---|
| Next.js 15 App Router | ✅ wired | `next@15.4.11`, App Router throughout |
| Drizzle ORM | ✅ wired | `drizzle-orm@0.45`, schema in `src/db/schema.ts`, migrations under `src/db/migrations/` |
| Neon Postgres | ✅ wired | `@neondatabase/serverless`; local Postgres via `docker-compose.yml` for offline dev |
| Auth.js v5 | ✅ wired | `next-auth@5.0.0-beta.31` + `@auth/drizzle-adapter`. GitHub OAuth always on; Resend magic-link gated by `RESEND_API_KEY` |
| Resend | ✅ wired | `resend@6`; magic-link + weekly digest |
| Vercel Cron | ✅ wired | `/api/cron/{daily-rollup,digest-weekly}` route handlers exist |
| Boring-avatars, Marked, Mermaid, sanitize-html, Zod | ✅ wired | Runtime deps |
| Upstash Redis (rate limit) | ⏳ planned | Not in package.json yet |
| Vercel Blob (avatar uploads) | ⏳ planned | Not in package.json yet — avatars currently URL-only or boring-avatars |
| Vercel BotID | ⏳ planned | Not yet integrated |
| Vercel Analytics | ⏳ planned | Not yet integrated |
| AI Gateway / Vercel AI SDK | ⏳ deferred | Lands with AI moderation phase |

## 4. Data model

```
users
  id (uuid pk),
  username (citext, unique, not null),
  email (citext, unique, not null),
  email_verified (timestamptz null),       -- Auth.js standard column
  avatar_url (text null),
  bio (text null),
  role (enum user|staff|locked|system, default 'user', not null),
  is_agent (bool default false, not null), -- reserved for later persona phase
  karma (int default 0, not null),         -- denormalized; updated incrementally by trigger
  created_at, updated_at

submissions
  id (uuid pk),
  author_id (fk users not null),
  type (enum news|tip|tutorial|course|article|podcast|interview|tool|discussion, not null),
  title (text not null),
  url (text null),
  text (text null),                         -- xor with url, enforced in Server Action
  state (enum pending|approved|rejected, default 'pending', not null),
  score (int default 0, not null),          -- denormalized vote sum; updated by trigger
  reading_time_min (int null),              -- for type='tutorial'
  podcast_meta (jsonb null),                -- for type='podcast': {host, duration_min}
  tool_meta (jsonb null),                   -- for type='tool': {stars, language, last_commit}
  created_at, published_at (null until approved),
  deleted_at (timestamptz null)             -- soft-delete tombstone

comments
  id (uuid pk),
  author_id (fk users not null),
  submission_id (fk submissions not null),
  parent_id (fk comments null),             -- null = top-level
  body (text not null),
  state (enum pending|approved|rejected, default 'approved', not null),
  score (int default 0, not null),
  created_at,
  deleted_at (timestamptz null)             -- soft-delete tombstone

ai_decisions                                -- IMMUTABLE; reserved for v2 (AI moderation phase)
  id, target_type (submission|comment), target_id,
  decision (approve|reject|escalate), confidence (numeric 3,2),
  model_id, reasoning (text), prompt_hash, cost_usd, created_at

moderation_overrides                        -- one per staff action on AI decision; v2-active
  id, target_type, target_id, ai_decision_id (fk null),
  staff_id (fk users), override (approve|reject), note, created_at

moderation_log                              -- v1: every staff action, public read
  id, staff_id (fk users), action (enum lock|unlist|delete|restore|dismiss_flag|lock_user),
  target_type, target_id, note (text null), created_at

votes
  user_id, submission_id, value (1|-1), created_at
  PRIMARY KEY (user_id, submission_id)

saves
  user_id, submission_id, created_at
  PRIMARY KEY (user_id, submission_id)

tags
  slug (pk), name, tagline (text null), sort_order (int default 0)

submission_tags
  submission_id, tag_slug
  PRIMARY KEY (submission_id, tag_slug)

flags
  id, reporter_id (fk users), target_type, target_id, reason,
  status (enum open|resolved, default 'open'),
  resolved_by (fk users null), created_at, resolved_at (null)

notifications
  id, user_id (fk users), kind (enum comment_reply|submission_reply|moderation|mention),
  payload (jsonb), read_at (null), created_at

user_hidden_submissions                     -- per-user "I've read this, stop showing me"
  user_id, submission_id, hidden_at
  PRIMARY KEY (user_id, submission_id)

user_tag_mutes                              -- per-user mute list
  user_id, tag_slug, muted_at
  PRIMARY KEY (user_id, tag_slug)

user_email_prefs                            -- /settings page writes here
  user_id (pk fk users),
  digest_weekly (bool default true),
  notify_replies (bool default true),
  updated_at

projects
  id, slug, name, blurb, owner_id (fk users), created_at
project_submissions
  project_id, submission_id

metrics_daily                               -- nightly rollup
  day (date pk), submissions_total, comments_total, votes_total, signups_total,
  active_users_24h
```

**Auth.js standard tables** (managed by `@auth/drizzle-adapter`; do not modify their schemas):

```
accounts                                    -- OAuth provider linkage
sessions                                    -- DB-backed session records
verification_tokens                         -- magic-link tokens
```

### Indexes

```
idx_submissions_state_created   on submissions(state, created_at desc) where deleted_at is null
idx_submissions_state_score     on submissions(state, score desc) where deleted_at is null
idx_submission_tags_tag         on submission_tags(tag_slug, submission_id)
idx_comments_submission_created on comments(submission_id, created_at) where deleted_at is null
idx_comments_parent             on comments(parent_id) where deleted_at is null
idx_flags_open                  on flags(target_type, target_id) where status = 'open'
idx_notifications_user_unread   on notifications(user_id, created_at desc) where read_at is null
idx_submissions_fts             GIN on to_tsvector(title || ' ' || coalesce(text,'') || ' ' || coalesce(url,''))
```

### Triggers (raw SQL, not Drizzle-generated)

**`score_after_vote_change`** — on `votes` insert/update/delete:

```sql
-- on INSERT
UPDATE submissions SET score = score + NEW.value WHERE id = NEW.submission_id;

-- on UPDATE (vote flip)
UPDATE submissions SET score = score + (NEW.value - OLD.value) WHERE id = NEW.submission_id;

-- on DELETE
UPDATE submissions SET score = score - OLD.value WHERE id = OLD.submission_id;
```

O(1) per vote.

**`karma_after_score_change`** — on `submissions.score` and `comments.score` updates:

```sql
UPDATE users SET karma = karma + (NEW.score - OLD.score) WHERE id = NEW.author_id;
```

O(1). **Do not** use `SUM(...)` aggregate — that's O(N) per vote and dominates DB CPU.

### Hot rank — query-time

Source: [Ken Shirriff's reverse-engineering](http://www.righto.com/2013/11/how-hacker-news-ranking-really-works.html).

```sql
ORDER BY (
  POWER(GREATEST(score - 1, 0), 0.8) /
  POWER(EXTRACT(EPOCH FROM (NOW() - created_at)) / 3600 + 2, 1.8)
) DESC
```

Computed at query time — no materialized view in v1. **Re-evaluate** when row count > 50k OR hot-feed p95 > 200ms; at that point, materialize as a generated column refreshed by trigger.

## 5. Auth model

- Auth.js v5 with magic-link (Resend) + GitHub OAuth providers.
- DB-backed sessions via `@auth/drizzle-adapter`.
- `auth()` helper in Server Components / Server Actions.
- Roles: `user` (default), `staff` (manual flag in DB), `locked` (banned; sessions revoked on lock), `system` (reserved for agent personas later).
- `isStaff(session)` checks `role === 'staff'` only; `'locked'` users cannot use Server Actions.

### DrizzleAdapter ↔ our `users` table mapping

The adapter expects specific column names. Our extended `users` table has additional fields. Mapping:

| Auth.js field | Our column | Notes |
|---|---|---|
| `id` | `id` (uuid) | Adapter expects string; uuid serializes fine |
| `name` | `username` | Adapter writes `name` on OAuth signup; we expose it as `username` |
| `email` | `email` | Identical |
| `emailVerified` | `email_verified` | Snake-case in DB, camelCase in adapter |
| `image` | `avatar_url` | Adapter writes `image` from OAuth profile |

Additional columns (`role`, `karma`, `is_agent`, `bio`) are not touched by the adapter and have DB-side defaults.

**Username generation on OAuth signup**: GitHub OAuth provides `login`. On signup callback, attempt `INSERT ... ON CONFLICT (username) DO UPDATE SET username = username || '_' || substring(id::text, 1, 6)` — appends a short suffix on collision.

**On `lockUser` action**: in addition to setting `role = 'locked'`, immediately `DELETE FROM sessions WHERE user_id = $1` to invalidate live sessions.

## 6. Pages → actions

| Page | Auth | Reads | Writes |
|---|---|---|---|
| `/` | open | `getHotSubmissions(currentUser?)` | – |
| `/new`, `/top` | open | sorted variants | – |
| `/c`, `/c/[slug]` | open | tag list, tag-filtered feed | – |
| `/post/[id]` | open | submission + comment thread | `vote`, `save`, `submitComment`, `flag`, `editComment`, `deleteComment` |
| `/u/[username]` | open | user profile + their submissions | – |
| `/projects`, `/projects/[slug]` | open | project list, project detail | – |
| `/about` | open | static | – |
| `/login` | anon | – | Auth.js handles |
| `/submit` | authed | – | `submitPost` |
| `/saved` | authed | `getSavedForUser(currentUser)` | – |
| `/notifications` | authed | `getNotifications(currentUser)` | `markRead` |
| `/settings` | authed | `getEmailPrefs(currentUser)` | `updateEmailPrefs`, `requestAccountDeletion`, `requestDataExport` |
| `/admin` | staff | dashboard counts | – |
| `/admin/queue` | staff | open flags + first-submission queue | `moderationAction` |
| `/admin/log` | authed | `getModerationLog()` (last N entries) | – |
| `/admin/users` | staff | `listUsers()` | `setRole`, `lockUser` |
| `/admin/flags` | staff | `listFlags()` + tag vocabulary | `resolveFlag`, tag mutations |
| `/api/rss` | open | site-wide feed | – |
| `/api/rss/c/[slug]` | open | per-tag feed | – |
| `/api/rss/u/[username]` | open | per-user submissions feed | – |
| `/sitemap.xml`, `/robots.txt` | open | static + DB-backed sitemap | – |
| `/api/og/submission/[id]` | open | dynamic OG image (next/og) | – |

Writes are Server Actions (`"use server"`). Cron + auth + OG endpoints are route handlers under `/api`. No public REST in v1.

## 7. Moderation pipeline (v1, manual)

Submissions enter via `submitPost`:

1. Zod-validate; reject malformed input.
2. Duplicate URL check (30-day window); if found, return existing submission for inline display.
3. **Determine state**:
   - If author has `karma >= 50` OR `>= 2` approved past submissions → `state = 'approved'`, `published_at = now()`.
   - Otherwise → `state = 'pending'` (lands in `/admin/queue`'s first-submission lane).
4. Insert row + `submission_tags` rows.
5. If state is `pending`, write a `notifications` row to the author ("your post is being reviewed").

Comments use a similar gate but default to `state = 'approved'` for any user with an approved submission history, else `'pending'`.

**Staff actions** (each writes a `moderation_log` row, public-readable):

| Action | Effect |
|---|---|
| `lock` | Submission accepts no further comments |
| `unlist` | Submission hidden from feeds, still accessible by permalink |
| `delete` | Soft-delete: set `deleted_at`; tombstone preserved (see §8 tombstone format) |
| `restore` | Clear `deleted_at` |
| `dismiss_flag` | Mark flag `resolved`, no content change |
| `lock_user` | Set `users.role = 'locked'`, delete that user's sessions |

The `moderation_log` is **append-only**. Edits and rolebacks happen by adding new rows, never by mutating old ones.

**AI moderation hooks (v2)**: when AI moderation lands, the worker writes `ai_decisions` rows and (depending on confidence) sets `state` directly or routes to `/admin/queue`. `moderation_overrides` records every staff override of an AI decision.

## 8. Tombstone format

When a submission or comment is soft-deleted (because replies/comments exist), the row is preserved but renders differently:

```ts
// On read
if (submission.deleted_at) return {
  ...submission,
  title: '[deleted]',
  url: null,
  text: null,
  // author_id preserved server-side for moderation context;
  // not exposed in the public response
};
```

**Hard-delete only via staff action with explicit `force` flag** (a manual SQL operation, not exposed through the UI). Hard-delete logs as `delete_hard` in `moderation_log`.

## 9. Folder layout

**Current (this branch):** a single `(prototype)/` route group holds every user-facing page; authed and staff pages are gated by `auth()` + the `?as=` shim rather than by separate route groups.

```
src/
  app/
    (prototype)/     all pages — feed, post, user, projects, about, login,
                     submit, saved, notifications, settings, briefs,
                     admin/{queue,audit,log,users,flags}, mod, search
    error.tsx, not-found.tsx
    layout.tsx       root layout (`<html>` + global CSS)
    robots.ts, sitemap.ts
    api/
      auth/[...nextauth]/route.ts
      cron/{daily-rollup,digest-weekly}/route.ts
      og/submission/[id]/route.tsx
      rss/route.ts
      rss/c/[slug]/route.ts
      rss/u/[username]/route.ts
  components/
    prototype/       SubmissionRow, FeedTabs, FeedHeader, VoteButtons, SaveButton,
                     CommentThread, CommentForm, PrototypeNav, AdminTabs,
                     TypeMeta, Logo, Avatar, FlagButton
  db/
    client.ts        Neon serverless client + Drizzle init
    schema.ts        Drizzle schema, single file
    queries.ts       getSubmissionsByHot/New/Top, getSubmissionById,
                     getCommentsForSubmission, getAllTags, getSavedForUser, …
    migrations/      Drizzle-generated + raw SQL trigger migrations
    seed.ts
  lib/
    auth.ts                    Auth.js config + helpers (DrizzleAdapter, GitHub + Resend)
    prototype-fixtures.ts      ?as= shim helpers + remaining fixture loaders
    actions/                   Server Actions: submission, comment, vote, settings,
                               moderation, tag (split intentionally narrow)
    markdown.ts                marked + sanitize-html allowlist
    agent-sprites.ts, card-tint.ts, escape-xml.ts
  styles/
    theme.css                  design tokens (`:root`)
    prototype.css              `.proto-` classes
    font-mono-only.css         font override (mono-only display mode)
docker-compose.yml             local Postgres for offline dev
drizzle.config.ts              drizzle-kit config
vercel.json                    framework, crons, redirects, headers
                               (vercel.ts migration is in §3a "planned")
scripts/
  sync-projects.ts, render-pixel-avatars.ts, reroll-agent.ts,
  generate-agents.ts, seed-agents.ts
design/
  fixtures/*.json, scripts/, IA.md, architecture.md, features.md, …
```

**Planned route-group split (deferred).** The originally-planned `(public)/(authed)/(admin)/` split was not adopted in v1; gating happens inside each page via `auth()`. Revisit if a page count or middleware-segmentation argument materializes.

## 10. Environment

- Vercel project linked to Neon via Marketplace (auto-provisions `DATABASE_URL`, `DATABASE_URL_UNPOOLED`).
- Preview deployments use Neon branch DBs — every PR gets isolated state.
- Secrets via `vercel env` (currently used):
  - `AUTH_SECRET`, `AUTH_GITHUB_ID`, `AUTH_GITHUB_SECRET`
  - `RESEND_API_KEY`, `EMAIL_FROM`
  - `NEXT_PUBLIC_SITE_URL` (used by RSS, sitemap, OG image absolute URLs)
  - `DATABASE_URL` / `NEON_DATABASE_URL`
- Planned (will be added when the corresponding integrations land — see §3a):
  - `UPSTASH_REDIS_REST_URL`, `UPSTASH_REDIS_REST_TOKEN` (rate limiting)
  - `BLOB_READ_WRITE_TOKEN` (avatar uploads)
- Cron schedules + headers + rewrites currently live in `vercel.json`. Migration to `vercel.ts` is planned; see §3a.
- **Tier requirements at cutover**: Vercel Pro (sub-daily crons + multiple cron jobs), Resend Pro (>100 emails/day cap on free tier).

## 11. Open questions worth resolving before phase 2

1. **Canonical tag list.** Admin-only tag creation in v1; the starter set must be defined as a phase 0 deliverable.
2. **First-staff bootstrap.** Document in runbook: SQL `UPDATE users SET role='staff' WHERE username = '<your-handle>'` after production cutover.
3. **Cookie consent.** Vercel Analytics is privacy-respecting (no PII); decide whether to add a banner anyway for EU users.
4. **Karma cold-start policy.** Either accept that downvote is gated until the community grows, or seed founder accounts with starter karma at launch.

## 12. What's deliberately not in v1

- AI moderation pipeline (schema-ready; deferred)
- Bot-as-named-AI-personas (deferred)
- Editorial briefs / longform `/briefs`
- Realtime updates (no websockets, no SSE)
- Mobile native or PWA install
- DMs, follows, social graph
- Search beyond Postgres FTS (later: pgvector for semantic when content volume warrants)
- Multilingual content (English-only v1)
- Federation / ActivityPub
- Public edit history
- Profile pictures (avatars are URL-only, no upload UI)
- Reactions / emoji per item
- Infinite scroll
- Algolia / external search

Each of these can be added without rearchitecting if we keep the schema relational and the actions thin.

---

> **Phase sequencing lives in `implementation-plan.md`** (12 phases, ~22 dev-day budget). This doc and that one share data-model and stack vocabulary; they should not contradict. If they do, the implementation plan wins for sequencing decisions and this doc wins for stack/schema decisions.

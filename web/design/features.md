# ClauDepot v2 — feature inventory

> Companion to `architecture.md` (stack + data model), `implementation-plan.md` (build sequencing), and `IA.md` (information architecture). The canonical "what's in v1, what's deliberately out, and how the load-bearing parts work."

## Reading guide

Tiers are sorted by importance, not phase order. Items in **Tier 1–4** ship in v1; **Tier 5** ships in v1 unchanged from prototype; **Tier 6** is the deliberate skip list (each item lists the mechanism by which skipping wins).

## Tier 1 — required to function

| Feature | Notes |
|---|---|
| Submit | Link or text post; submission `type` distinguishes `news/tip/tutorial/course/article/podcast/interview/tool/discussion` |
| Hot feed (`/`) | HN ranking formula |
| New feed (`/new`) | `ORDER BY created_at DESC` |
| Top feed (`/top`) | Score-only, with ?window=day\|week\|month\|all |
| Vote (▲/▼) | Up always; down karma-gated (≥100) |
| Save (★) | Private bookmark; orthogonal to vote |
| Threaded comments | Recursive CTE fetch |
| Per-user profile (`/u/[username]`) | Submitted / comments / saved tabs |
| Karma | Denormalized via incremental trigger |
| Flag | Community moderation signal |
| Notifications | Pull-based inbox + nav badge |
| Search | Postgres FTS (`tsvector` over title+text+url+tag_slugs) |
| RSS feeds | `/rss`, `/c/[slug]/rss`, `/u/[username]/rss` |

## Tier 2 — HN-specific that matters

| Feature | Spec |
|---|---|
| Karma-gated downvote | Threshold 100 |
| Edit window | 5 min from `created_at`, then locked |
| Delete window | Only while no replies exist; after, soft-delete (tombstone) |
| Comment permalink | `/post/[id]#comment-[comment_id]`, scrolls + highlights |
| Subtree collapse | Client-side only, no DB state |
| Markdown | Strict allowlist: `p i em strong a code pre ul ol li blockquote del`. **No** headings, images, tables. Server-rendered, sanitized via rehype-sanitize. |
| Comment-score visibility | Author sees their own; nobody else (prevents pile-on/groupthink) |
| Pagination | `/more?after=<id>&n=30`. No infinite scroll. |
| Duplicate URL detection | 30-day lookback on submit; show existing submission inline |
| Rate limits | 5 submissions/day for new users (<30d), 20/day after; 1 comment per 60s; 10 signups/IP/day |
| First-submission queue | New accounts (karma <50, no approved past) → `state='pending'`; staff approves; auto-graduates after 2 approved or 50 karma |

## Tier 3 — Lobsters-style additions

| Feature | Mechanism |
|---|---|
| Public moderation log (`/admin/log`) | Every staff action visible to any signed-in user. Removes "shadowbanning" complaint surface. |
| Required tags on submit | Multi-select from `tags` table, validated server-side. |
| Tag descriptions (`tagline`) | Shown on `/c/[slug]` header. |
| Per-user hide-submission | Different from save; "I've read this, stop showing me." |
| Per-user mute-tag | Excludes submissions whose tags overlap with mutes. |

## Tier 4 — vertical features (already in prototype)

| Feature | Source |
|---|---|
| Submission types | 9 enum values; UI renders type-specific row meta |
| Reading time pill | `submissions.reading_time_min` for tutorials |
| Podcast play affordance | `submissions.podcast_meta jsonb` (host, duration_min) |
| Tool meta | `submissions.tool_meta jsonb` (stars, language, last_commit) |
| Discussion preview | First 220 chars of `submissions.text` for `type='discussion'` |
| Projects | Curated collections of submissions; `/projects`, `/projects/[slug]` |

## Tier 5 — deliberate anti-features (skip list)

Each item names the mechanism by which **not** building it wins.

| Skip | Mechanism |
|---|---|
| Images / video in posts | Lobsters and Tildes prove text-only filters out low-effort content; an AI-builder community trades visual richness for signal density. |
| Reactions / emoji per item | One vote axis = one ranking signal. Adding 👍❤️🎉 dilutes the score and invites the LinkedIn-ization of comments. |
| Follows, DMs, social graph | This is a reader, not a network. Following creates filter bubbles + harassment vectors with no ranking value. |
| Public edit history | Removes a perpetual relitigation surface. HN's choice; Lobsters has it and regrets it visibly in moderation threads. |
| Profile pictures | HN/Lobsters don't have them. Removes one axis of identity-bias in ranking. |
| Infinite scroll | `/more` pagination forces a stopping point. Infinite scroll is engagement-bait; this is a daily reader. |
| Public per-comment scores | Author sees their own score; nobody else. Prevents pile-on dynamics. |
| Realtime / SSE / websockets | Adds infra surface for negligible UX win on a daily-cadence product. |
| Algolia / external search | Postgres FTS handles tens of thousands of items at trivial cost. Add only when proven necessary. |
| Federation / ActivityPub | Operational surface for users we don't have. Defer indefinitely. |
| Mobile native | Web-responsive only in v1. PWA install possible later. |

## Load-bearing implementation specs

### Story ranking — HN formula

Standard. Source: [Ken Shirriff's reverse-engineering](http://www.righto.com/2013/11/how-hacker-news-ranking-really-works.html).

```
score_rank = POWER(GREATEST(score - 1, 0), 0.8) /
             POWER(EXTRACT(EPOCH FROM (NOW() - created_at)) / 3600 + 2, 1.8)
```

With controversy penalty for posts with `comments >= 40 AND comments > votes`:

```
final_rank = score_rank * POWER(votes::float / comments, 3)
```

Computed at query time. **Re-evaluate** when row count crosses 50k OR hot-feed p95 latency > 200ms — at that point, materialize as a generated column refreshed on score change.

### Comment ranking

```
comment_rank = score / POWER(EXTRACT(EPOCH FROM (NOW() - created_at)) / 3600 + 2, 0.8)
```

No submitter-reputation factor in v1. Add `users.comment_karma` factor in v2 if quality drops.

### Karma — incremental, NOT aggregate

Trigger fires on `submissions.score` and `comments.score` changes:

```sql
UPDATE users
SET karma = karma + (NEW.score - OLD.score)
WHERE id = NEW.author_id;
```

O(1) per vote. **Do not** recompute via `SUM(...)` — that's O(N) and dominates DB CPU at any real volume.

### Threading

Standard. `comments.parent_id` self-reference, fetched via recursive CTE:

```sql
WITH RECURSIVE thread AS (
  SELECT * FROM comments WHERE submission_id = $1 AND parent_id IS NULL
  UNION ALL
  SELECT c.* FROM comments c JOIN thread t ON c.parent_id = t.id
)
SELECT * FROM thread ORDER BY parent_id NULLS FIRST, comment_rank DESC;
```

Postgres handles this cleanly to ~10k comments per submission. Closure tables / nested sets are faster but unnecessary at v1 volume.

### Tombstone format

When a comment or submission is soft-deleted (because replies exist), the row stays but display becomes:

```
{
  body / title / url: null,
  deleted_at: <timestamp>,
  author_id: <preserved>  -- hidden in public render, kept for moderation context
}
```

UI renders "[deleted]" placeholder. The tombstone preserves thread structure without leaking content.

### Anti-spam stack

Five layers, in order of precedence:

1. **Vercel BotID** on signup form (frontend defense)
2. **Email verification required** before first post (Auth.js handles for magic-link; OAuth users get a verification step via Resend)
3. **First-submission queue**: new accounts → `state='pending'`, staff approves; auto-graduates at 2 approved OR 50 karma
4. **Upstash sliding-window rate limits**: signup 10/IP/day, submission 5/user/day (<30d) or 20/user/day (≥30d), comment 1/60s, vote 50/min sanity cap
5. **Duplicate URL check** on submit (30-day window) — show existing submission inline; force user to comment there

### Search

Postgres FTS in v1:

```sql
CREATE INDEX idx_submissions_fts ON submissions USING GIN (
  to_tsvector('english',
    title || ' ' || COALESCE(text, '') || ' ' || COALESCE(url, '') || ' ' ||
    COALESCE((SELECT string_agg(tag_slug, ' ') FROM submission_tags WHERE submission_id = submissions.id), '')
  )
);
```

Query via `ts_rank` for ordering. **Re-evaluate** for pgvector when log analysis shows >X% search→empty rate driven by vocabulary mismatch.

## Pinned defaults

These decisions are settled and should not be re-litigated without a named mechanism:

| Decision | Pick | Rationale |
|---|---|---|
| Karma threshold for downvote | 100 | HN uses 500; lower threshold compensates for smaller community |
| Edit window | 5 min | HN uses 2h; 5min discourages rewriting after replies arrive |
| Delete cutoff | No replies exist | Preserves thread structure; tombstone after that |
| First-submission queue threshold | 2 approved OR 50 karma | Auto-graduates trustworthy users; manual cost stays bounded |
| Rate limits (new user) | 5 submissions/day, 60s between comments | Calibrated for human pace, frustrating for scripts |
| Dup-URL lookback | 30 days | Same URL after 30 days probably warrants re-discussion |
| Comment thread depth | Unlimited, with subtree collapse | Trees self-prune via collapse |
| Pagination size | 30 items/page | Matches HN |
| Notification delivery | Pull (badge + inbox) v1, weekly email digest v1 | No push; no realtime |
| Email digest cadence | Weekly (Sunday 12:00 UTC) | Daily creates spam-folder fate |
| Markdown allowlist | `p i em strong a code pre ul ol li blockquote del` | No headings, images, tables |
| Karma cold-start | Intentional gate; downvote unlocks as community grows | Or seed founder accounts with starter karma at launch |

## What we still owe before phase 0 starts

- **Canonical tag list** as a phase 0 deliverable (admin-only tag creation in v1; need the starter set).
- **Moderation rulebook** (`design/moderation-rulebook.md`) — only required when AI moderation lands; not a v1 blocker.
- **First-staff bootstrap procedure** documented in the runbook (which user gets `role='staff'` at production cutover).

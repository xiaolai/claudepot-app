/**
 * Drizzle schema for claudepot.com v2 — barrel of the per-domain
 * sub-modules.
 *
 * Single source of truth for the database structure. See
 * design/architecture.md §4 for the spec this implements.
 *
 * Editorial runtime tables (`decision_records`, `override_records`,
 * `scout_runs`) match `editorial/rubric.yml` v0.2.3 + `editorial/audits/
 * README.md`. Bot-side writes come from the `claudepot-office` private
 * repo running on mac-mini-home; reader-side reads come from the public
 * web app (this repo). See `editorial/transparency.md` for the
 * public-vs-private split. Policy moderator (`policy_decisions`) is
 * unrelated to editorial — see dev-docs/policy-moderator-plan.md.
 *
 * Migration 0008_editorial_runtime.sql replaced the v1 `ai_decisions` /
 * `moderation_overrides` scaffolding (no consumers existed) with the
 * richer per-criterion / per-persona tables in editorial.ts.
 *
 * Migration 0018_policy_moderation.sql added the policy moderator
 * substrate in policy.ts.
 *
 * The barrel is what every caller imports as `@/db/schema`. drizzle's
 * `import * as schema from "./schema"` in db/client.ts also resolves
 * here.
 */

export * from "./enums";
export * from "./users";
export * from "./content";
export * from "./moderation";
export * from "./moderation-prompts";
export * from "./moderation-retro";
export * from "./notifications";
export * from "./editorial";
export * from "./policy";
export * from "./projects";
export * from "./metrics";
export * from "./api-tokens";

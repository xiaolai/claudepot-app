/**
 * Reconciliation for Ada-proposed tags on accepted submissions.
 *
 * Three phases:
 *
 *   1. Filter: drop AI tags whose slug duplicates a user-supplied tag
 *      (user wins — they already chose, and Ada agreeing is
 *      redundant) and drop slugs already linked to this submission.
 *   2. Vocab probe: for each surviving slug, decide if the row exists
 *      in the `tags` table. A slug Ada flagged is_new=false but
 *      that doesn't exist gets treated as new (reconcile, not trust).
 *      An existing slug with pending_review=true is also acceptable —
 *      it means staff is reviewing it; another submission using it
 *      is fine.
 *   3. Insert: missing `tags` rows get inserted with
 *      pending_review=true (and a name derived from the slug).
 *      Then `submission_tags` rows are inserted with source='ai'.
 *
 * Cap: total tags per submission stay ≤ 5 (the existing input cap).
 * If user already supplied 5, AI tags are dropped silently — the
 * user's intent wins.
 *
 * Errors here MUST NOT throw out of createSubmission. The submission
 * itself is already inserted; tag application failure is logged and
 * swallowed so a vocab race doesn't undo a valid post.
 */

import { inArray } from "drizzle-orm";

import { db } from "@/db/client";
import { submissionTags, tags as tagsTable } from "@/db/schema";

import type { ModerationTag } from "./types";

const MAX_TAGS_PER_SUBMISSION = 5;

interface ApplyAiTagsParams {
  submissionId: string;
  userTagSlugs: readonly string[];
  aiTags: readonly ModerationTag[];
}

interface ApplyAiTagsResult {
  /** Slugs actually linked to the submission with source='ai'. */
  appliedSlugs: string[];
  /** Slugs newly inserted into `tags` with pending_review=true. */
  newlyCreatedSlugs: string[];
  /** Slugs Ada proposed but were skipped (cap, dup, etc.). */
  skippedSlugs: string[];
}

/**
 * Convert a slug to a human-friendly default name. The result is a
 * placeholder — staff can rename when approving at /admin/tags. We
 * keep the heuristic simple (replace hyphens, capitalize) rather
 * than trying to be clever. "rag" → "Rag", "ai-agents" → "Ai
 * Agents". Staff fixes it on approval.
 */
function defaultNameForSlug(slug: string): string {
  return slug
    .split("-")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export async function applyAiTags(
  params: ApplyAiTagsParams,
): Promise<ApplyAiTagsResult> {
  const { submissionId, userTagSlugs, aiTags } = params;

  const result: ApplyAiTagsResult = {
    appliedSlugs: [],
    newlyCreatedSlugs: [],
    skippedSlugs: [],
  };

  // Budget check first — if the user already saturated the cap we
  // skip entirely. Empty AI tag list is the common path; bail fast.
  const remaining = MAX_TAGS_PER_SUBMISSION - userTagSlugs.length;
  if (aiTags.length === 0 || remaining <= 0) {
    for (const t of aiTags) result.skippedSlugs.push(t.slug);
    return result;
  }

  // Phase 1 — dedup against the user's choices and against each
  // other (in case Ada returned the same slug twice). Order matters:
  // earlier in the array wins so the first occurrence of a slug is
  // the one we keep.
  const userSet = new Set(userTagSlugs);
  const seen = new Set<string>();
  const candidates: ModerationTag[] = [];
  for (const t of aiTags) {
    if (userSet.has(t.slug) || seen.has(t.slug)) {
      result.skippedSlugs.push(t.slug);
      continue;
    }
    seen.add(t.slug);
    candidates.push(t);
  }
  if (candidates.length === 0) return result;

  // Trim candidates down to the remaining budget. AI proposals
  // beyond the cap are dropped, not queued.
  const accepted = candidates.slice(0, remaining);
  for (const t of candidates.slice(remaining)) {
    result.skippedSlugs.push(t.slug);
  }

  // Phases 2 and 3 run in a single transaction. Splitting the new-
  // tag INSERT and the submission_tags INSERT across autocommit
  // boundaries would let a failed link insert leave orphan pending
  // tags behind (a tag row with no submission attached), which is
  // user-invisible but creates noise in /admin/flags. The whole
  // function still doesn't throw out of createSubmission — the
  // caller swallows tx failures so a vocab race never undoes a
  // valid post.
  const candidateSlugs = accepted.map((t) => t.slug);
  await db.transaction(async (tx) => {
    // Find which candidate slugs already exist in `tags`. The set
    // decides which need an INSERT first. We deliberately do NOT
    // trust the model's is_new flag here — a hallucinated
    // is_new=false on a missing slug would FK-violate the
    // submission_tags insert below.
    const existing = await tx
      .select({ slug: tagsTable.slug })
      .from(tagsTable)
      .where(inArray(tagsTable.slug, candidateSlugs));
    const existingSet = new Set(existing.map((r) => r.slug));

    const toCreate = accepted.filter((t) => !existingSet.has(t.slug));
    if (toCreate.length > 0) {
      // Brand-new tags land with pending_review=true. The slug is
      // also the placeholder name — staff renames when approving.
      await tx
        .insert(tagsTable)
        .values(
          toCreate.map((t) => ({
            slug: t.slug,
            name: defaultNameForSlug(t.slug),
            tagline: null,
            sortOrder: 0,
            pendingReview: true,
          })),
        )
        // A concurrent moderation pass on a different submission
        // may have inserted the same slug between our SELECT and
        // INSERT. onConflictDoNothing keeps us idempotent — the
        // existing row wins (pending_review state stays whatever
        // staff left it).
        .onConflictDoNothing({ target: tagsTable.slug });
      for (const t of toCreate) result.newlyCreatedSlugs.push(t.slug);
    }

    // Link the submission to every accepted slug. source='ai'
    // marks provenance so /admin/log and future analytics can
    // split AI vs user tagging. onConflictDoNothing covers a
    // parallel call that already created the same (submission,
    // tag) pair.
    await tx
      .insert(submissionTags)
      .values(
        accepted.map((t) => ({
          submissionId,
          tagSlug: t.slug,
          source: "ai" as const,
        })),
      )
      .onConflictDoNothing({
        target: [submissionTags.submissionId, submissionTags.tagSlug],
      });
    for (const t of accepted) result.appliedSlugs.push(t.slug);
  });

  return result;
}

/** Exported for tests: the per-submission tag cap is shared with
 *  the input schema (lib/submissions/schema.ts max(5)). */
export const TAG_CAP_PER_SUBMISSION = MAX_TAGS_PER_SUBMISSION;

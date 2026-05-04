/**
 * Office-specific read queries — drive the /office/ public window per
 * editorial/transparency.md. These read decision_records / override_records
 * / scout_runs and shape them for the UI.
 *
 * Bot-side WRITES happen from the claudepot-office private repo on
 * mac-mini-home; this file only reads.
 */

import { and, desc, eq, sql } from "drizzle-orm";

import { db } from "./client";
import {
  decisionRecords,
  overrideRecords,
  submissions,
  users,
} from "./schema";

export interface OfficeDecision {
  id: string;
  submissionId: string;
  submissionTitle: string;
  submissionUrl: string | null;
  submissionType: string;
  appliedPersona: string;
  perCriterionScores: Record<string, number>;
  weightedTotal: number;
  hardRejectsHit: string[];
  inclusionGates: Record<string, boolean>;
  typeInferred: string;
  subSegmentInferred: string;
  confidence: "high" | "low";
  oneLineWhy: string;
  finalDecision: "accept" | "reject" | "borderline_to_human_queue";
  routing: "feed" | "firehose" | "human_queue";
  rubricVersion: string;
  audienceDocVersion: string;
  modelId: string;
  scoredAt: Date;
}

const OFFICE_DECISION_SELECT = {
  id: decisionRecords.id,
  submissionId: decisionRecords.submissionId,
  submissionTitle: submissions.title,
  submissionUrl: submissions.url,
  submissionType: sql<string>`${submissions.type}::text`,
  appliedPersona: decisionRecords.appliedPersona,
  perCriterionScores: decisionRecords.perCriterionScores,
  weightedTotal: decisionRecords.weightedTotal,
  hardRejectsHit: decisionRecords.hardRejectsHit,
  inclusionGates: decisionRecords.inclusionGates,
  typeInferred: sql<string>`${decisionRecords.typeInferred}::text`,
  subSegmentInferred: decisionRecords.subSegmentInferred,
  confidence: decisionRecords.confidence,
  oneLineWhy: decisionRecords.oneLineWhy,
  finalDecision: decisionRecords.finalDecision,
  routing: decisionRecords.routing,
  rubricVersion: decisionRecords.rubricVersion,
  audienceDocVersion: decisionRecords.audienceDocVersion,
  modelId: decisionRecords.modelId,
  scoredAt: decisionRecords.scoredAt,
};

function mapRow(r: {
  id: string;
  submissionId: string;
  submissionTitle: string;
  submissionUrl: string | null;
  submissionType: string;
  appliedPersona: string;
  perCriterionScores: unknown;
  weightedTotal: string; // numeric → string
  hardRejectsHit: unknown;
  inclusionGates: unknown;
  typeInferred: string;
  subSegmentInferred: string;
  confidence: "high" | "low";
  oneLineWhy: string;
  finalDecision: "accept" | "reject" | "borderline_to_human_queue";
  routing: "feed" | "firehose" | "human_queue";
  rubricVersion: string;
  audienceDocVersion: string;
  modelId: string;
  scoredAt: Date;
}): OfficeDecision {
  return {
    ...r,
    weightedTotal: Number.parseFloat(r.weightedTotal),
    perCriterionScores: r.perCriterionScores as Record<string, number>,
    hardRejectsHit: r.hardRejectsHit as string[],
    inclusionGates: r.inclusionGates as Record<string, boolean>,
  };
}

/** Recent decisions across all routings (default: accepted only). */
export async function getRecentDecisions(opts: {
  routing?: "feed" | "firehose" | "human_queue";
  persona?: string;
  limit?: number;
} = {}): Promise<OfficeDecision[]> {
  const limit = Math.min(opts.limit ?? 30, 200);
  const filters = [];
  if (opts.routing) filters.push(eq(decisionRecords.routing, opts.routing));
  if (opts.persona) filters.push(eq(decisionRecords.appliedPersona, opts.persona));

  const rows = await db
    .select(OFFICE_DECISION_SELECT)
    .from(decisionRecords)
    .innerJoin(submissions, eq(decisionRecords.submissionId, submissions.id))
    .where(filters.length > 0 ? and(...filters) : undefined)
    .orderBy(desc(decisionRecords.scoredAt))
    .limit(limit);

  return rows.map(mapRow);
}

export async function getOfficeDecisionById(
  id: string
): Promise<OfficeDecision | null> {
  const rows = await db
    .select(OFFICE_DECISION_SELECT)
    .from(decisionRecords)
    .innerJoin(submissions, eq(decisionRecords.submissionId, submissions.id))
    .where(eq(decisionRecords.id, id))
    .limit(1);
  if (rows.length === 0) return null;
  return mapRow(rows[0]);
}

/** Per-persona stats: total decisions, accept rate, avg score on accepts. */
export interface PersonaStats {
  persona: string;
  total: number;
  accepted: number;
  rejected: number;
  borderline: number;
  avgWeightedTotalAccepted: number;
}

export async function getPersonaStats(persona: string): Promise<PersonaStats> {
  const rows = await db
    .select({
      total: sql<number>`COUNT(*)::int`,
      accepted: sql<number>`COUNT(*) FILTER (WHERE ${decisionRecords.finalDecision} = 'accept')::int`,
      rejected: sql<number>`COUNT(*) FILTER (WHERE ${decisionRecords.finalDecision} = 'reject')::int`,
      borderline: sql<number>`COUNT(*) FILTER (WHERE ${decisionRecords.finalDecision} = 'borderline_to_human_queue')::int`,
      avgWeightedTotalAccepted: sql<number>`COALESCE(AVG(CASE WHEN ${decisionRecords.finalDecision} = 'accept' THEN ${decisionRecords.weightedTotal} END), 0)::float`,
    })
    .from(decisionRecords)
    .where(eq(decisionRecords.appliedPersona, persona));
  const r = rows[0] ?? { total: 0, accepted: 0, rejected: 0, borderline: 0, avgWeightedTotalAccepted: 0 };
  return { persona, ...r };
}

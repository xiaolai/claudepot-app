/**
 * Loads the editorial spec from this repo's editorial/ directory for
 * rendering on /office/. Distinct from src/lib/editorial/ (the scoring
 * runtime); this file is purely a read-and-render helper used by the
 * public /office/{transparency,voice,rubric} pages.
 *
 * Path resolution uses process.cwd() — Next.js sets cwd to the project
 * root for both `next dev` and the Vercel build, so editorial/* is
 * always reachable.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import yaml from "js-yaml";

const EDITORIAL_DIR = "editorial";

export function readAudienceMd(): string {
  return readFileSync(resolve(process.cwd(), EDITORIAL_DIR, "audience.md"), "utf-8");
}

export function readTransparencyMd(): string {
  return readFileSync(resolve(process.cwd(), EDITORIAL_DIR, "transparency.md"), "utf-8");
}

export interface PublicRubricView {
  version: string;
  values: Record<string, string>;
  hard_rejects: { id: string; why: string }[];
  inclusion_gates: { id: string; check: string }[];
  recency_windows: Record<string, number>;
  quality_criteria: { id: string; rubric: string }[]; // weights INTENTIONALLY OMITTED
  routing_destinations: Record<string, string>;       // thresholds INTENTIONALLY OMITTED
  format_extensions: Record<string, string[]>;
  persona_descriptions: { id: string; description: string }[]; // multipliers OMITTED
}

/**
 * Reads rubric.yml and returns ONLY the public-safe slices per
 * editorial/transparency.md §3:
 *   - Weights, thresholds, persona multipliers stay private (math
 *     adversaries could optimize against).
 *   - Criterion names + descriptions, hard_rejects ids + whys,
 *     gate names + checks, recency windows, format extension
 *     fields, persona descriptions = all public-safe.
 */
export function readPublicRubricView(): PublicRubricView {
  const raw = readFileSync(
    resolve(process.cwd(), EDITORIAL_DIR, "rubric.yml"),
    "utf-8"
  );
  const r = yaml.load(raw) as Record<string, unknown>;

  const qualityScore = (r.quality_score ?? {}) as Record<string, { rubric: string }>;
  const personaOverlays = (r.persona_overlays ?? {}) as Record<
    string,
    { description: string }
  >;
  const routing = (r.routing ?? {}) as { destinations?: Record<string, string> };

  return {
    version: r.version as string,
    values: r.values as Record<string, string>,
    hard_rejects: r.hard_rejects as { id: string; why: string }[],
    inclusion_gates: r.inclusion_gates as { id: string; check: string }[],
    recency_windows: r.recency_windows as Record<string, number>,
    quality_criteria: Object.entries(qualityScore).map(([id, v]) => ({
      id,
      rubric: v.rubric,
    })),
    routing_destinations: routing.destinations ?? {},
    format_extensions: r.extensions as Record<string, string[]>,
    persona_descriptions: Object.entries(personaOverlays).map(([id, v]) => ({
      id,
      description: v.description,
    })),
  };
}

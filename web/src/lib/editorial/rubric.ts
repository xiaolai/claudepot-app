import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import yaml from "js-yaml";
import type { Rubric } from "./types";

const RUBRIC_PATH = "editorial/rubric.yml";
const AUDIENCE_PATH = "editorial/audience.md";

export function loadRubric(): Rubric {
  const path = resolve(process.cwd(), RUBRIC_PATH);
  const raw = readFileSync(path, "utf-8");
  return yaml.load(raw) as Rubric;
}

export function loadAudienceDoc(): string {
  const path = resolve(process.cwd(), AUDIENCE_PATH);
  return readFileSync(path, "utf-8");
}

/**
 * Returns a pruned YAML view of the rubric for inclusion in the model prompt.
 * Strips blocks that are NOT used during scoring and that would confuse the
 * model into treating them as the response schema:
 *
 *   - decision_record       (output schema we fill in from scoring + routing — not scored)
 *   - engagement_loop_inputs (audit-system spec — not scored)
 *   - calibration_notes     (meta — not scored)
 *   - guardrails            (operational rules; the model gets the relevant ones in the prompt)
 *   - not_in_scope          (ingestion routing — happens before scoring)
 *
 * Keeps: version, audience, values, hard_rejects, inclusion_gates,
 * recency_windows, quality_score, persona_overlays, extensions, routing.
 */
export function loadRubricForPrompt(): string {
  const full = loadRubric() as unknown as Record<string, unknown>;
  const pruned: Record<string, unknown> = {};
  const keep = [
    "version",
    "audience",
    "values",
    "hard_rejects",
    "inclusion_gates",
    "recency_windows",
    "quality_score",
    "routing",
    "extensions",
    "persona_overlays",
  ];
  for (const key of keep) {
    if (key in full) pruned[key] = full[key];
  }
  return yaml.dump(pruned, { lineWidth: 100 });
}

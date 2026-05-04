/**
 * Score a submission against the editorial rubric using Claude.
 *
 *   pnpm editorial:score \
 *     --title "..." \
 *     --url "..." \
 *     [--body "..." | --body-file path/to.md] \
 *     [--type tool|paper|tutorial|...] \
 *     [--persona ada|historian|scout] \
 *     [--json]
 *
 * Without --json, prints a human-readable verdict + per-criterion table.
 */

import { parseArgs } from "node:util";
import { readFileSync, existsSync } from "node:fs";
import { score } from "@/lib/editorial/score";
import type { Persona, SubmissionType } from "@/lib/editorial/types";

const VALID_PERSONAS: Persona[] = ["base", "ada", "historian", "scout"];
const VALID_TYPES: SubmissionType[] = [
  "news",
  "release",
  "tool",
  "podcast",
  "tutorial",
  "paper",
  "interview",
  "discussion",
  "workflow",
  "case_study",
  "prompt_pattern",
];

function fail(msg: string, code = 2): never {
  console.error(`✗ ${msg}`);
  process.exit(code);
}

const { values } = parseArgs({
  options: {
    title: { type: "string", short: "t" },
    body: { type: "string", short: "b" },
    "body-file": { type: "string", short: "f" },
    url: { type: "string", short: "u" },
    type: { type: "string" },
    persona: { type: "string", short: "p", default: "base" },
    json: { type: "boolean", default: false },
  },
});

if (!values.title) fail("--title required");
if (!values.url) fail("--url required");

let body: string;
if (values["body-file"]) {
  if (!existsSync(values["body-file"])) fail(`body-file not found: ${values["body-file"]}`);
  body = readFileSync(values["body-file"], "utf-8");
} else if (values.body) {
  body = values.body;
} else {
  fail("--body or --body-file required");
}

const persona = values.persona as Persona;
if (!VALID_PERSONAS.includes(persona)) {
  fail(`unknown persona "${persona}". Valid: ${VALID_PERSONAS.join(", ")}`);
}

let type: SubmissionType | undefined;
if (values.type) {
  if (!VALID_TYPES.includes(values.type as SubmissionType)) {
    fail(`unknown type "${values.type}". Valid: ${VALID_TYPES.join(", ")}`);
  }
  type = values.type as SubmissionType;
}

const decision = await score(
  { title: values.title!, body, source_url: values.url!, type },
  { persona }
);

if (values.json) {
  console.log(JSON.stringify(decision, null, 2));
  process.exit(0);
}

const verdictGlyph =
  decision.final_decision === "accept" ? "✓ accept" :
  decision.final_decision === "reject" ? "✗ reject" :
  "? human-queue";

console.log(
  `${verdictGlyph} → ${decision.routing}  (score: ${decision.weighted_total.toFixed(1)}, persona: ${decision.applied_persona}, confidence: ${decision.confidence})`
);
console.log(
  `  type: ${decision.type_inferred}  sub-segment: ${decision.sub_segment_inferred}`
);
if (decision.hard_rejects_hit.length > 0) {
  console.log(`  hard rejects hit: ${decision.hard_rejects_hit.join(", ")}`);
}
const failedGates = Object.entries(decision.inclusion_gates)
  .filter(([, v]) => !v)
  .map(([k]) => k);
if (failedGates.length > 0) {
  console.log(`  inclusion gates failed: ${failedGates.join(", ")}`);
}
console.log(`  why: ${decision.one_line_why}`);
console.log();
console.log("  per-criterion scores:");
for (const [k, v] of Object.entries(decision.per_criterion_scores)) {
  console.log(`    ${k.padEnd(25)} ${v}`);
}
console.log();
console.log(`  rubric: v${decision.rubric_version}  audience: v${decision.audience_doc_version}  scored: ${decision.scored_at}`);

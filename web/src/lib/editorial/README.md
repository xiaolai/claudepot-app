# src/lib/editorial/

The runtime layer for the editorial team. Reads the spec in `editorial/`, calls Claude to score submissions, returns a `DecisionRecord`.

This is one half of the two-layer editorial system. The other half — the spec — lives at the repo root in `editorial/`. The spec answers *what to score for*; this layer answers *what does this submission score?*.

## Files

| Path | Purpose |
|---|---|
| `types.ts` | `SubmissionInput`, `DecisionRecord`, `Persona`, `Rubric`, `ScoreResponse`, criterion + sub-segment + type enums. |
| `rubric.ts` | Reads `editorial/rubric.yml` (parsed YAML and raw string) and `editorial/audience.md`. Pure file I/O. |
| `routing.ts` | Pure functions: `computeWeightedTotal()` and `decideRouting()`. No API calls; tested in `tests/editorial-routing.test.ts`. |
| `prompt.ts` | Assembles the system + user prompts. The system prompt embeds the full audience constitution and rubric — meant to be cached. |
| `schema.ts` | Single source of truth for the Claude response shape. Zod schema; `claude.ts` derives a JSON Schema from it for the SDK; `score.ts` uses it for runtime validation. |
| `claude.ts` | Claude Agent SDK wrapper. Authenticates via the local Claude Code CLI's OAuth session (no API key needed). Forces structured output via `outputFormat: { type: "json_schema", schema }`. Sandboxed: `allowedTools: []`, `settingSources: []`, `persistSession: false`. |
| `score.ts` | The orchestrator. `score(submission, options)` → `DecisionRecord`. Validates Claude's response with the shared Zod schema before computing routing. |

## Authentication

No API key. The runtime uses [`@anthropic-ai/claude-agent-sdk`](https://github.com/anthropics/claude-agent-sdk-typescript), which authenticates through the local Claude Code CLI's OAuth session.

**Prerequisites:**

1. Claude Code CLI installed (`npm i -g @anthropic-ai/claude-code` or via Homebrew)
2. Logged in (`claude /login` once in your shell)

If you've ever run `claude` interactively on this machine, you're already set up. The SDK reuses that session.

**Trade-offs vs direct API key:**

- ✓ No key management; calls bill against the user's Claude subscription rather than per-call API charges.
- ✗ Requires Claude Code CLI on the host. Production deployments (e.g., Vercel) without the CLI installed will need a different runtime path — out of scope for v0.

## Programmatic usage

```ts
import { score } from "@/lib/editorial/score";

const decision = await score(
  {
    title: "Comparing four prompt strategies for legal document review",
    body: "Over two weeks we tested four prompt strategies on 1,200 contract clauses...",
    source_url: "https://example.com/article",
    type: "case_study",     // optional; agent infers if omitted
  },
  { persona: "ada" }        // optional; defaults to "base"
);

console.log(decision.final_decision);  // "accept" | "reject" | "borderline_to_human_queue"
console.log(decision.routing);          // "feed" | "firehose" | "human_queue"
console.log(decision.weighted_total);
console.log(decision.one_line_why);
```

## CLI

```bash
pnpm editorial:score \
  --title "..." \
  --url "https://..." \
  --body "..." \
  [--type case_study] \
  [--persona ada] \
  [--json]

# or read body from a file:
pnpm editorial:score --title "..." --url "..." --body-file path/to/article.md
```

Without `--json`, prints a human-readable verdict + per-criterion table. With `--json`, prints the full `DecisionRecord` for piping or storage.

## Architecture choices worth knowing

### Structured output via JSON Schema, not free-text JSON

The Agent SDK's `outputFormat: { type: "json_schema", schema }` option forces the model to return validated structured data — no fragile JSON-in-markdown parsing. The schema is derived from a single Zod schema in `schema.ts` via `z.toJSONSchema()`, so the SDK enforcement and our runtime validation can never drift.

### Sandboxed scoring call

The `query()` call sets `allowedTools: []` (no file/bash/web access), `settingSources: []` (no skills, hooks, plugins), and `persistSession: false` (no session disk pollution). The agent's only job is to emit one structured-output response.

### OAuth session caching

Because the SDK uses Claude Code's OAuth session, prompt caching is handled by the CLI's session layer rather than per-call `cache_control` headers. As long as the system prompt (audience + rubric) is identical across calls, the underlying Claude infrastructure caches it and bills accordingly.

### Routing math is pure

`decideRouting()` and `computeWeightedTotal()` do not make API calls and don't read files — they take a `ScoreResponse` + `Rubric` + `Persona` and return a `RoutingResult`. This is deliberately separable so the routing logic can be tested without spending API quota and so future changes to scoring (different model, different schema) don't have to touch routing.

### Per-persona scores are not directly comparable

ada multiplies `evidence_quality` × 1.5; her evidence-multiplier raises her effective max on that criterion above the base. The `feed_threshold` (37) is fixed across personas — by design. ada accepts more evidence-heavy content; historian and scout each have their own multiplier shape. All personas pass the same numeric bar but earn it differently.

### Validation at the API boundary

Zod parses the Claude response into a typed `ScoreResponse` before any routing logic runs. If Claude returns malformed scores (out of range, missing fields, unknown enum values), the parse throws and the caller decides what to do. We do not silently coerce or default-fill.

## What this layer is NOT

- **Not the spec.** Audience definition, voice, criteria, weights, hard_rejects all live in `editorial/`. This layer reads them.
- **Not an ingestion layer.** It takes a pre-extracted `(title, body, source_url)`. Fetching, HTML stripping, type detection from URL — separate.
- **Not a scheduler or queue.** No retries, no backoff, no rate-limit handling. Caller orchestrates.
- **Not the audit layer.** The `decision_record` shape returned here matches the schema in `rubric.yml` decision_record block, so it slots into the audit substrates — but writing to the substrate is a separate concern.
- **Not the comment-quality scorer.** That's a future `commentScore()` against a different (planned) rubric.

## Future shape

Once `decision_record` and `override_record` substrates exist:

```ts
// in some scoring pipeline:
import { score } from "@/lib/editorial/score";
import { saveDecisionRecord } from "@/db/queries";
import { publish } from "@/lib/social/publish";

for (const submission of pendingSubmissions) {
  const decision = await score(submission);
  await saveDecisionRecord(decision);

  if (decision.routing === "feed") {
    const post = formatForSocial(submission, decision);
    await publish(post, { platforms: ["bluesky", "x"] });
  }
}
```

That `formatForSocial` function applies `audience.md` voice rules to produce a publication-ready post — the link between the editorial layer and the social layer.

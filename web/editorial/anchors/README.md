# editorial/anchors/

Hand-curated exemplars used as fixed reference points for the rubric. Anchors are the only thing in `editorial/` that is *content*, not *spec*.

## What anchors are used for

| Use | Why anchors are the right reference |
|---|---|
| **5/2 receipts bootstrap** | Until `decision_record` history exists (≥ 100 logged decisions), receipts come from anchors. |
| **Goodhart-watch comparison** | When a guardrail says "compare against editorial gut," anchors are the concrete external reference. |
| **Anchor-drift quarterly audit** | Re-evaluate every anchor under the current `rubric_version`. If accept-anchors stop scoring above `routing.feed_threshold`, that's drift. |
| **New-persona calibration** | Run a new persona over the accept and reject sets; verify scores match `expected_verdict`. |
| **Cross-version regression test** | When bumping `rubric.yml` minor or major, score the full anchor set under old and new versions; diff. |

A self-auditing rubric without anchors has no reference points. Treat anchor coverage as a release blocker for any major rubric version.

## Files

| Path | Purpose |
|---|---|
| `accept.example.json` | Skeleton + 1 worked example. Copy when adding real anchors to `accept.json`. |
| `reject.example.json` | Same for rejects. |
| `accept.json` *(planned — populate to ≥ 30 across all sub-segments)* | "Post like this." |
| `reject.json` *(planned — populate to ≥ 30 across hard_rejects + constitutional nevers)* | "Never post like this." |
| `archive/` *(planned)* | Anchors retired due to staleness (world drift, not rubric drift). Retained for historical audits. |

## Schema

Each anchor is a JSON object:

```json
{
  "id": "anchor-<verdict>-<short-name>",
  "title": "the actual title (or a synthetic one for hand-written examples)",
  "url": "the actual URL, or 'synthetic' for hand-written",
  "body_excerpt": "first ~500 chars of body so anchor evaluation doesn't depend on live URL fetch",
  "type": "tool | paper | tutorial | podcast | workflow | case_study | prompt_pattern | discussion | news | release | interview",
  "sub_segment": "knowledge_workers | engineers | operators | learners | cross_cutting",
  "expected_verdict": "accept | reject",
  "expected_reason": "one-sentence why this is the canonical example. References specific rubric criteria or hard_rejects.",
  "expected_score_range": [lower, upper],
  "added": "YYYY-MM-DD",
  "added_by": "<handle>",
  "rubric_version_at_addition": "0.X.Y",
  "audience_version_at_addition": "0.X.Y"
}
```

`expected_score_range` is required for `expected_verdict: accept`, optional for rejects (rejects fail at the gate or hard_reject layer; score doesn't matter).

`body_excerpt` is required because anchors must be evaluable offline. Live URL fetch is too brittle for a quarterly regression test.

## Coverage targets

| Set | Target count | Distribution |
|---|---|---|
| `accept.json` | ≥ 30 | At least 5 per sub-segment (knowledge_workers / engineers / operators / learners) + cross_cutting |
| `reject.json` | ≥ 30 | At least 1 per hard_reject id + at least 1 per constitutional never (audience.md §3.4) |

Coverage below the target → the anchor-drift audit loop runs at reduced confidence and the 5/2 bootstrap is partially broken.

## Edit policy

- **Adding an anchor** is a normal commit (no special process). Bump no version.
- **Removing an anchor** requires noting *why* in the commit message. Anchors should rarely be removed; usually they're moved to `archive/`.
- **An accept-anchor that no longer scores ≥ `routing.feed_threshold`** under the current rubric is a signal of drift. It does not get silently retired — it goes to the next anchor-drift audit, attribution-tagged as `rubric drift` or `world drift`.
- **A reject-anchor that no longer rejects** is a hole in the hard_reject layer. Same: audit it, don't silently update.

## Why anchors live in `editorial/`, not `design/fixtures/`

`design/fixtures/` is seed data for the application — submissions to display in dev. Anchors are reference data for the rubric — they don't appear in any feed. Different lifecycle, different consumer, different ownership.

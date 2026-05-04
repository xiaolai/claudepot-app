---
type: <override | engagement | persona | audit-agent | audience-realization | anchor | ad-hoc>
period_start: YYYY-MM-DD
period_end: YYYY-MM-DD
rubric_version: 0.X.Y
audience_version: 0.X.Y
loop: <overrides | engagement | persona | audit-agent | audience-realization | anchors | ad-hoc>
auditor: <human handle or agent id>
status: <draft | reviewed | merged | rejected>
---

# Audit: <type> — <YYYY-MM>

## Summary

One sentence in plain English. What was found, what's proposed.

## Method

- **Data source:** <override_record | engagement_record | persona_scores | audit_agent_log | analytics | anchor_evaluation>
- **Sample size:** N
- **Analysis:** <correlation | aggregation | re-evaluation | comparison>
- **Tools used:** <scripts, queries, external services>

## Findings

### Finding 1 — <short name>

- **Severity:** high | medium | low
- **Failure type:** wrong-weight | ambiguous-criterion | procedural-error | world-drift | ingestion-bug
- **Attribution:** rubric drift | world drift | ingestion bug | unclear
- **Evidence:** <numbers, sample IDs, score deltas — be specific>

<Free-form description of what was observed and why it matters.>

### Finding 2 — <short name>

...

## Proposed patches

> Each patch must carry **5 receipts** (decisions it would change) and **2 sanity checks** (decisions it must not change). If a patch can't supply receipts, it is not ready — return to "Findings" with more data.

### Patch 1 — <short name>

- **Target file:** `rubric.yml` | `audience.md` | `personas/<name>.yml`
- **Change:** <one-line description of the edit>
- **Source finding:** Finding N above
- **Predicted version bump:** patch | minor | major

#### Receipts — 5 historical decisions this patch would change

| ID  | Title (truncated)         | Old verdict | New predicted verdict | Why this is the right change |
| --- | ------------------------- | ----------- | --------------------- | ---------------------------- |
|     |                           |             |                       |                              |
|     |                           |             |                       |                              |
|     |                           |             |                       |                              |
|     |                           |             |                       |                              |
|     |                           |             |                       |                              |

#### Sanity — 2 historical decisions this patch should NOT change

| ID  | Title (truncated)         | Verdict (unchanged) | Why this should still hold   |
| --- | ------------------------- | ------------------- | ---------------------------- |
|     |                           |                     |                              |
|     |                           |                     |                              |

#### Surprise budget

- **Items reviewed:** 100 (most recent published)
- **Items reclassified by this patch:** N
- **Threshold:** 15% triggers extra review; 5% requires explicit acceptance
- **This patch:** under | at | over → normal | accepted-surprise | extra-review | split-recommended

### Patch 2 — <short name>

...

## Outstanding questions

- <Things this audit could not resolve. Goes to the next loop or to a follow-up audit.>

## Decision

<For the reviewer to fill in. One row per proposed patch.>

Reviewer roles (allowed values for the Reviewer column):

| Role               | Who                                                              | Authority                                                            |
| ------------------ | ---------------------------------------------------------------- | -------------------------------------------------------------------- |
| `owner`            | editorial owner (xiaolai)                                        | Approves all patches.                                                |
| `external`         | rotating verified-human practitioner (pool TBD; see `TODO.md`)   | Co-reviews engagement loop monthly; flags blockers on other loops.   |
| `auditor-agent`    | the auditor agent process                                        | Files draft audits; never approves merges.                           |
| `community`        | community contributor via PR                                     | Files draft audits; merge requires `owner` co-sign.                  |

| Patch | Decision                                  | Reviewer (role)     | Date       | Notes |
| ----- | ----------------------------------------- | ------------------- | ---------- | ----- |
| 1     | approved \| rejected \| deferred          | owner               | YYYY-MM-DD |       |
| 2     | approved \| rejected \| deferred          | owner               | YYYY-MM-DD |       |

## Follow-ups

- **If approved:** PR link, version bumps applied, downstream files touched.
- **If deferred:** when to revisit, what additional data is needed.
- **If rejected:** why the rubric is correct as-is; record so the same finding doesn't get re-litigated next loop.

<!-- Optional. Only present if the merged report received copy-level corrections.
     Anything substantive (changing a finding, decision, verdict, or receipt count)
     requires a new audit, not an amendment. -->

## Amendments *(optional — only after status: merged)*

| Date       | Editor   | What changed                                              |
| ---------- | -------- | --------------------------------------------------------- |
| YYYY-MM-DD |          | Fixed broken link in Receipts table row 3.                |

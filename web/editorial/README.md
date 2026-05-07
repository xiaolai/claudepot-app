# editorial/

The editorial team's source of truth. Specs, not runtime — code that applies these specs lives in `src/lib/editorial/` (when it lands).

## Files

| Path                    | Purpose                                                                                                                                                                            |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `audience.md`           | The constitution. Audience definition, voice principles, style, glossary, constitutional nevers, in-voice and not-in-voice examples. Every other file here references this one.    |
| `rubric.yml`            | The taste rubric every agent applies before posting or surfacing a link. Versioned. References `audience.md` for sub-segment IDs and voice grounding.                              |
| `transparency.md`       | The transparency policy. Defines the single public window into the agentic team's machinery (`/office/` on claudepot.com) — what fits, what doesn't, why. Constitutional alongside `audience.md` and `rubric.yml`. |
| `audits/`               | Audit reports — the closed loop that turns rubric mistakes into rubric fixes. Four continuous substrates feed six periodic loops (weekly: `overrides`, `persona`; monthly: `engagement`; quarterly: `audit-agent`, `audience-realization`, `anchors`); each audit can drive one atomic PR (which may touch multiple target files when changes are coordinated). See `audits/README.md`. |
| `anchors/`              | Hand-curated exemplars used as fixed reference points: 5/2 receipts bootstrap, Goodhart-watch comparison, anchor-drift quarterly audit, new-persona calibration. Currently scaffolded with `accept.example.json` + `reject.example.json`; populating `accept.json` / `reject.json` is the first build-order blocker (see `TODO.md`). |
| `personas/` *(to be split out)* | Per-persona weight overlays. Currently inline in `rubric.yml` `persona_overlays:` (ada, ClauDepot, historian, scout). Split into per-file `personas/*.yml` when any single persona spec exceeds 15 lines or when a 5th persona is added. |
| `TODO.md`               | Open work — referenced from other editorial docs but not yet built or specified. Build-order dependency graph for the audit-loop substrates lives here. Reviewed alongside `rubric.yml` at the monthly engagement-loop review. |

## Edit policy

- `audience.md` is the constitution. Other files reference it; never restate it.
- `rubric.yml` is reviewed like code. Bump `version` on every meaningful change.
- Old posts retain the `rubric_version` and `audience.md` version they were scored under, so historical drift stays interpretable.
- Public-facing readable subsets are published at `/about/rubric` and `/about/voice`. Generated from these files; do not maintain parallel copies.

## Versioning

`rubric.yml` and `audience.md` each carry their own version and have separate bump policies. The two versioning schemes are independent on purpose — copy edits to the voice doc shouldn't force a rubric bump, and a weight tweak in the rubric shouldn't touch the voice version.

### `rubric.yml` versioning

- Patch (`0.X.y`): copy edits, weight tweaks ≤ 20%, new exemplars, structural reorganization without semantic change.
- Minor (`0.x.0`): added/removed criteria, weight changes > 20%, new format extension, new persona, new hard_reject.
- Major (`X.0.0`): structural change to how the rubric is applied (e.g., persona overlay model changes, routing semantics changes).

### `audience.md` versioning

See `audience.md` §6. Briefly: patch = glossary / examples / copy edits; minor = voice or style principles added or removed, sub-segment definitions change; major = audience primary definition or binding traits change.

### Cross-file pinning

`rubric.yml` carries `audience.doc_version_pinned`. When `audience.md` minor- or major-bumps, the rubric should bump its pin in the same PR (or in a coordinated follow-up). A patch bump on `audience.md` does *not* require updating the pin.

## Audits + the 5/2 receipts rule

The rubric is not a static spec. Four continuous data substrates (decisions, overrides, engagement, audit-agent divergences) feed six periodic loops that produce structured reports under `editorial/audits/`. Each report can drive at most one PR.

| Loop                    | Cadence    | Detects                                       |
| ----------------------- | ---------- | --------------------------------------------- |
| `overrides`             | weekly     | Ambiguous criteria, procedural errors         |
| `persona`               | weekly     | Ambiguous criteria, persona drift             |
| `engagement`            | monthly    | Wrong weights                                 |
| `audit-agent`           | quarterly  | Procedural application errors                 |
| `audience-realization`  | quarterly  | World drift in audience composition           |
| `anchors`               | quarterly  | World drift in taste                          |

Substrates run continuously (records). Loops run on a schedule (reports). The `audit-agent` loop is unique in that its substrate (an auditor agent re-scoring every Nth decision) requires a separate process; the other substrates are populated as a side effect of normal operation.

**Every PR that changes `rubric.yml`, `audience.md`, or `personas/*.yml` must carry receipts:**

- **5 historical decisions the patch would change** — old vs. predicted new outcome, plus a one-line "why this is the right change."
- **2 historical decisions the patch should NOT change** — sanity check against over-aggressive tuning.
- **Surprise-budget number** — how many of the last 100 published items move between `/feed` and `/firehose`. Above 15% triggers extra review.

If a patch can't supply receipts, it is not ready. Vibes-driven edits don't ship.

See `editorial/audits/README.md` for the full policy and `audits/_template.md` for the report shape.

## Review cadence

Driven by the six audit loops above. AI tooling moves fast — quarterly is too slow for engagement; weekly is too noisy for audience-realization. The cadence per loop is set so the slowest signal still moves before its next reading.

The monthly engagement-loop review rotates between editorial owner (xiaolai) and one external practitioner. **External practitioner pool is TBD** — working model is a rotating volunteer drawn from invited verified-human commenters who have demonstrated taste alignment over ≥ 3 months. Until the pool exists, monthly reviews are owner-only. See `TODO.md`.

All cadences are UTC. *Weekly* = ISO week (Monday–Sunday). *Monthly* = calendar month. *Quarterly* = calendar quarter (Q1: Jan–Mar, etc.).

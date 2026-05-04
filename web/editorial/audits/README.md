# editorial/audits/

Audit reports — the closed loop that turns rubric mistakes into rubric fixes.

Each report drives at most one PR against `editorial/rubric.yml`, `editorial/audience.md`, or `editorial/personas/*.yml`. Reports without resulting patches are still kept — they are evidence the loops ran and that nothing was found to need changing.

## Naming convention

| Pattern                     | When                                                                |
| --------------------------- | ------------------------------------------------------------------- |
| `YYYY-MM-{loop}.md`         | Weekly or monthly periodic loops                                    |
| `YYYY-QN-{loop}.md`         | Quarterly periodic loops (e.g., `2026-Q2-anchors.md`)               |
| `YYYY-MM-DD-{name}.md`      | Ad-hoc audits driven by a specific event or question                |

## Data substrate (always running)

Four record types feed the periodic loops. All are populated continuously as events happen — they are *records*, not *reports*.

| Record type                     | What populates it                                                                                                       |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `decision_record`               | every agent decision (schema in `rubric.yml`)                                                                           |
| `override_record`               | every human override of an agent decision in `/admin/queue`                                                             |
| `engagement_record`             | per-post analytics — votes, comments, saves, dwell-time-median, return-reads, age                                       |
| `audit_agent_divergence_record` | independent re-scoring of every Nth published decision by a separate auditor agent; logs criteria where divergence > threshold |

Three of these (decision, override, engagement) record events that happen during normal operation. The fourth (`audit_agent_divergence_record`) requires a separate auditor agent to run continuously — it is the only substrate that costs ongoing compute to populate, and the only one whose existence is a deliberate audit-system choice rather than a side effect.

## Periodic loops (run on a schedule, produce a report)

Each loop runs on its cadence, aggregates substrate records, and produces one report file under `editorial/audits/`. Each audit drives one atomic PR (which may touch multiple target files when changes are coordinated).

| Loop                    | Cadence    | Detects                                       | Reads                                                                                            | Run by                                              |
| ----------------------- | ---------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------------------------------------- |
| `overrides`             | weekly     | Ambiguous criteria, procedural errors         | `override_record` + correlated `audit_agent_divergence_record`                                   | automated (script — not yet built; see `TODO.md`)   |
| `persona`               | weekly     | Ambiguous criteria, persona drift             | `decision_record` grouped by persona; cross-persona variance per criterion                       | automated (planned)                                 |
| `engagement`            | monthly    | Wrong weights                                 | `engagement_loop_inputs` triggers (in `rubric.yml`) + `engagement_record` × `decision_record` per-criterion correlation | automated (planned) + reviewer interpretation       |
| `audit-agent`           | quarterly  | Procedural application errors                 | `audit_agent_divergence_record` aggregated by criterion and rubric_version                       | semi-automated (auditor agent + manual review)      |
| `audience-realization`  | quarterly  | World drift in audience composition           | analytics-inferred sub-segment distribution vs. declared `target_mix_per_7d`                     | **manual** (blocked until sub-segment inference exists — see `TODO.md`) |
| `anchors`               | quarterly  | World drift in taste                          | anchor exemplars in `editorial/anchors/` re-evaluated under current rubric                       | semi-automated (script re-scores; reviewer interprets) |

Weekly loops are continuous-improvement (fast feedback). Monthly is the engagement reckoning. Quarterlies are checkpoints — slower-moving signal that doesn't reward more frequent reading.

The `audit-agent` loop is the only one whose substrate has no other consumer — it is paired one-to-one with its continuous re-scoring substrate. The other five loops read from substrates that also serve other purposes (analytics, moderation review, regular feed operation).

**Cadence anchors.** All cadences are UTC. *Weekly* = ISO week (Monday–Sunday). *Monthly* = calendar month. *Quarterly* = calendar quarter (Q1: Jan–Mar, Q2: Apr–Jun, Q3: Jul–Sep, Q4: Oct–Dec).

**On engagement scoring.** The engagement loop runs *per-signal* — one correlation per criterion × per engagement axis (votes, comments, saves, return-reads). Reports surface the strongest correlations across axes. A composite `engagement_score` formula is deferred until enough corpus data exists to calibrate the weights — see `TODO.md`. Premature compositing would Goodhart the rubric.

## Where to start

Don't try to land all six loops at once. Minimum viable system, in order:

1. **Populate `editorial/anchors/`** to ≥ 30 accept anchors and ≥ 30 reject anchors. Without anchors the 5/2 receipts rule cannot bootstrap (next step depends on this).
2. Wire `decision_record` and `override_record` substrates into the data layer.
3. Weekly `overrides` loop + the 5/2 receipts rule below.
4. Monthly `engagement` loop with `engagement_record` substrate.

The other three loops and the audit-agent substrate add depth, but value-per-effort drops sharply after the first four.

## The 5/2 receipts rule

**Every PR that changes `rubric.yml`, `audience.md`, or `personas/*.yml` must include receipts.**

For each proposed patch:

- **5 historical decisions the patch would change**, with old vs. new predicted verdicts and a one-line "why this is the right change."
- **2 historical decisions the patch should NOT change**, with a one-line "why this should still hold."

If a patch cannot supply receipts, it is not ready. This rules out vibes-driven edits and the "feels right, bump the weight" failure mode.

The receipts live inside the audit report (see `_template.md`). The PR description either repeats them or links to them.

### Bootstrap — what to do before decision history exists

Until `decision_record` has accumulated **≥ 100 logged decisions**, the 5/2 receipts come from **anchor exemplars in `editorial/anchors/`** instead of historical decisions. Format is the same — old vs. new predicted verdicts under the proposed rubric — just sourced from anchors.

This is why `editorial/anchors/` is a hard dependency for the audit system. Without anchors, early rubric edits have no defensible test. See `editorial/anchors/README.md`.

### Receipts shape per target file

| Target | What "5 decisions / 2 sanity" means |
|---|---|
| `rubric.yml` (criterion change, weight change, gate change) | 5 past decisions whose final score or verdict would change; 2 that should not. Bootstrap: 5 anchors. |
| `rubric.yml` (routing change) | 5 past decisions whose destination (`/feed`, `/firehose`, `/human_queue`) would change; 2 that should not. Bootstrap: 5 anchors. |
| `audience.md` (voice principle, glossary, constitutional never) | 5 examples of past editorial commentary or anchor `expected_reason` text the change would re-classify; 2 that should not. Bootstrap: drawn from anchors and from prior commits. |
| `personas/*.yml` (overlay multipliers, voice register) | 5 past decisions where the persona's verdict or score would change; 2 that should not. Bootstrap: 5 anchors. |

## Surprise budget

Each weight-changing or threshold-changing patch must be tested against the **last 100 published items**. Report how many would have been **reclassified** by the patch.

**"Reclassified" means: would have been routed to a different destination** (`/feed`, `/firehose`, or `/human_queue`). Score-within-band changes don't count — only band-boundary crossings.

| Reclassified | Treatment                                                                       |
| ------------ | ------------------------------------------------------------------------------- |
| < 5%         | normal review                                                                   |
| 5–15%        | reviewer must explicitly accept the surprise                                    |
| > 15%        | second reviewer required; consider splitting into smaller patches               |

Catches over-aggressive tuning before it ships.

**Bootstrap.** Until 100 published items exist, run the surprise budget against the anchor set instead — score every anchor under old rubric, score every anchor under new rubric, count destination changes. Over-tuning still gets caught; the population is just smaller.

## Filing policy

1. An audit produces a report. The report alone does nothing.
2. The report's `Proposed patches` section drives **one atomic PR per audit**. The PR may touch multiple target files (`rubric.yml`, `audience.md`, `personas/*.yml`, `anchors/`) when changes are coordinated — atomic means they ship together.
3. Every PR references its source audit and carries the 5/2 receipts inline (or links to them).
4. Approved/rejected/deferred patches are recorded in the report's `Decision` section.
5. **Findings, decisions, and receipts are immutable once `status: merged`.** Substantive new findings produce a new audit, never an amendment to a merged one. Trivial copy-level corrections to merged reports are allowed; mark them in an `## Amendments` section appended at the bottom of the report listing what was changed and when. Anything that would change a finding, a decision, a verdict, or a receipt count is *not* a copy edit and requires a new audit.

## Drift attribution — the load-bearing distinction

Every finding must answer: **is the rubric mis-calibrated, or did the world change?**

| Attribution     | Example                                                                                  | Remediation              |
| --------------- | ---------------------------------------------------------------------------------------- | ------------------------ |
| `rubric drift`  | A criterion that worked at v0.2 produces noise at v0.3 because the criterion text is now ambiguous | Edit `rubric.yml`        |
| `world drift`   | The set of valid AI tools doubled; `practitioner_fit` interpretation needs widening      | Edit `audience.md` or `rubric.yml` |
| `ingestion bug` | Hard-rejects firing on title alone because body wasn't passed to the agent               | Fix the ingestion layer; do not patch the rubric |
| `unclear`       | Signal exists but cause is undetermined                                                  | Defer; collect more data |

Confusing the four causes leads to patching the rubric for problems that are not rubric problems. The `not_in_scope` block in `rubric.yml` is the canonical example of this lesson learned.

## Public

This folder is published at `/about/audits`. Audit reports are part of the platform's editorial accountability — they show readers how decisions were made and how mistakes are caught and fixed. Most aggregators are black boxes about their editorial decisions; we are not.

## Files

| Path                       | Purpose                                                                                                  |
| -------------------------- | -------------------------------------------------------------------------------------------------------- |
| `_template.md`             | The audit-report template. Underscore prefix means "not a real audit." Copy when starting a new one.     |
| `YYYY-MM-{loop}.md`        | Actual audit reports, one per loop run.                                                                  |
| `YYYY-MM-DD-{name}.md`     | Ad-hoc audits triggered outside the regular loop cadence.                                                |

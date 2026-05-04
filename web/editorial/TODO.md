# editorial/TODO.md

Open work. Each item lists status, why it matters, and what unblocks it.

Items here come from the v0.2.3 / v0.1.2 audit (see commit history for the audit notes). They are the "loose ends" — referenced from other editorial docs but not yet built or specified.

---

## Build-order dependencies (the part that matters)

Several items below have dependency chains. The audit loops in `editorial/audits/` cannot fully run until upstream items land. Updated build order (incorporates the `claudepot-office` private repo on mac-mini-home + the `/office/` public window):

```
0. claudepot-office repo scaffold (private, on mac-mini-home)  ►  home for scouts, scoring loop, audit loops
1. Neon migration: scout_runs, decision_records, override_records, submitter_kind ►  the integration spine
2. anchors/accept.json + anchors/reject.json  ────────────►  unblocks 5/2 receipts bootstrap
3. scout v0 in claudepot-office (RSS + GitHub atom + body extract → Neon) ►  candidates start flowing
4. scoring loop in claudepot-office (polls Neon for pending → DecisionRecord) ►  decisions start landing
5. /office/ page on claudepot.com (renders public-safe subset per transparency.md) ►  visible bot activity
6. /admin/queue UI (human reviews borderline) ►  override_records substrate
7. Vercel cron for social publish (top accepted-but-not-yet-posted → Bluesky+X)
8. weekly overrides loop in claudepot-office (first audit substrate goes live)
9. monthly engagement loop with engagement_record substrate
10. sub-segment inference layer (blocks audience-realization loop)
11. auditor agent (blocks audit-agent loop)
12. personas/*.yml split-out (when 5th persona lands or any persona exceeds 15 lines)
```

Don't build downstream loops before their substrate exists. Empty loops produce empty reports, which atrophy.

---

## Anchor coverage *(blocks 5/2 receipts bootstrap)*

- [ ] Populate `editorial/anchors/accept.json` to ≥ 30 anchors, ≥ 5 per sub-segment.
- [ ] Populate `editorial/anchors/reject.json` to ≥ 30 anchors, ≥ 1 per `hard_reject` id, ≥ 1 per constitutional never (audience.md §3.4).
- [ ] Define an `anchor-evaluator` script that takes the rubric.yml version + the anchor set and outputs a per-anchor score table.

**Why it matters.** Without anchors, the 5/2 receipts rule cannot bootstrap (no decision history to draw from), the Goodhart-watch guardrail has nothing concrete to compare against, and the anchor-drift quarterly audit has nothing to evaluate. Three audit loops are partially broken until anchors land.

**Suggested first batch:** 10 anchors per sub-segment over 2 weeks of curation. Worked example in `accept.example.json` shows the shape.

---

## Sub-segment inference *(blocks audience-realization loop)*

- [ ] Decide between (a) self-declared sub-segment (user picks at signup), (b) behavioral inference (tag affinity, dwell patterns, save patterns), (c) hybrid.
- [ ] Build the inference layer.
- [ ] Wire it into the analytics dashboard so the quarterly `audience-realization` audit can read realized-distribution numbers.

**Why it matters.** Without sub-segment inference, the `audience-realization` audit is blocked. We can't tell whether the rubric's `target_mix_per_7d` (30/30/20/15/5) is being hit or missed. Until this lands, run the loop as a manual quarterly review of analytics: engineer eyeballs the dashboard, compares against gut-feel for each sub-segment.

**Decision needed:** which option (a/b/c). Hybrid is probably right but adds complexity.

---

## Engagement-score composite formula *(refines engagement loop)*

- [ ] After ≥ 100 published items have engagement data, regress per-engagement-axis on per-criterion scores. Find the axis weights that produce the strongest signal.
- [ ] Define `engagement_score = α·upvotes + β·comments + γ·saves + δ·return_reads − ε·downvotes` with calibrated coefficients.

**Why it matters.** Currently the engagement loop runs per-signal (one correlation per criterion × per axis). That's correct but verbose. A composite score makes the monthly report cleaner. Calibrating it before there's data is bad practice — strawman weights would Goodhart the rubric.

**Blocked on:** corpus + engagement data. Until then, per-signal correlations.

---

## Personas: split out from `rubric.yml` to `personas/*.yml`

- [ ] Create `editorial/personas/{ada,ClauDepot,historian,scout}.yml`, one file each.
- [ ] Each persona file: stance, multipliers, voice register delta from base voice, 3 example "why we picked this" lines in-character.
- [ ] Update `rubric.yml` to reference `personas/*.yml` instead of the inline `persona_overlays:` block.
- [ ] Update `audience.md` §5 once personas/ exists.

**Why it matters.** Inline persona definitions are fine for v0.2 but don't scale. Each persona will eventually have 30+ lines (overlay + voice notes + examples + retirement criteria). Inline becomes unreadable at that point.

**Not urgent** — current inline overlays work. Split when adding the 5th persona, or when a single persona's spec exceeds 15 lines.

---

## Auditor agent *(blocks audit-agent loop + substrate)*

- [ ] Define the auditor agent's spec: re-scoring methodology, divergence thresholds, what gets logged in `audit_agent_divergence_record`.
- [ ] Decide sampling rate (every Nth published decision, where N depends on volume).
- [ ] Build the agent.

**Why it matters.** The audit-agent loop and its substrate require this. Lower priority than the override and engagement loops because procedural-error detection is a smaller failure mode than wrong-weights or ambiguous-criteria.

**Defer until:** override loop and engagement loop are running and surfacing findings. If those two loops catch most issues, the audit-agent's marginal value is small. Re-evaluate quarterly.

---

## External practitioner reviewer pool *(supports engagement loop quality)*

- [ ] Decide recruitment model: invited verified-human commenters with ≥ 3 months of taste-aligned engagement; rotating pool of 5–10.
- [ ] Decide compensation: platform credits, verified-human badge upgrade, modest cash, or volunteer.
- [ ] Document the rotation cadence.

**Why it matters.** The engagement-loop monthly review is supposed to rotate between editorial owner (xiaolai) and one external practitioner. Without a defined pool, "external practitioner" stays an aspiration.

**Blocked on:** sufficient verified-human commenter base to draw from. Until then, monthly engagement reviews are owner-only.

---

## `/office/` — the single public window *(per `transparency.md`)*

- [ ] `/office` — landing page: today's picks, persona profiles, source list, recent overrides, audit reports archive. One tabbed page or a small set of sub-routes.
- [ ] `/office/persona/[name]` — per-persona profile: bot badge, multipliers' criteria interpretation in plain language, recent picks, recent comments. ada / ClauDepot / historian / scout.
- [ ] `/office/decision/[submission_id]` — per-decision page: routing verdict, applied persona, per-criterion scores, one_line_why, hard_rejects hit, gates failed.
- [ ] `/office/sources` — source list (names + last successful pull + items kept/dropped counts; **not** per-source rules).
- [ ] `/office/audits` — index of `editorial/audits/*.md` reports with `status: merged`. Renders the 5/2 receipts tables.
- [ ] `/office/rubric` — readable summary from `editorial/rubric.yml` `values:` block + criterion descriptions. Weights + thresholds NOT shown (per transparency.md §3).
- [ ] `/office/voice` — readable summary from `editorial/audience.md` §1 + §2 + §3.4.
- [ ] `/office/overrides` — log of human overrides with reasons, redacted of reviewer PII.

**Why it matters.** This is the platform's editorial accountability surface — the only place agentic activity is rendered to the public. Per `transparency.md`, every other detail of the bot machinery stays in the private `claudepot-office` repo on mac-mini-home.

**Blocked on:** at least one populated `decision_records` row (otherwise the pages are empty and undermine the message). Which means: scout v0 → scoring loop → first decisions land → then `/office/` ships.

**Replaces** the previous `/about/{rubric,voice,audits}` plan with one consolidated `/office/` namespace. The `/about` namespace is freed for non-bot project info.

---

## Comment guidelines for editorial agents

- [ ] Document when an editorial-team agent comments curatorially vs. takes a position.
- [ ] Define escalation: when does an agent comment trigger human review?
- [ ] Reference `audience.md` §2.5 (curatorial-not-opinionated) — the editorial *frame* doesn't take positions; agent comments in *threads* may.

**Why it matters.** Agents will comment on threads. Without guidelines, they'll either be inert or inappropriately opinionated. Rubric-style spec is needed.

**Blocked on:** at least one persona file exists (so per-persona comment register can be derived).

---

## `src/lib/editorial/` runtime layer

- [ ] Decide the runtime shape: a single `score(submission)` function? A pipeline of stages (gates → score → routing)? Per-persona invocation API?
- [ ] Wire it to `decision_record` substrate.
- [ ] Make it consume `rubric.yml` directly (no parallel TypeScript spec — single source of truth).

**Why it matters.** Right now `editorial/` is pure spec. Eventually agents need to execute it. The runtime layer is where YAML becomes behavior.

**Blocked on:** decision on whether agents call the runtime directly (in-process) or via an API (out-of-process). Probably in-process for simplicity, but at scale could be either.

---

## Migration tooling *(version-bump downstream effects)*

- [ ] When `audience.md` version bumps, what touches downstream? (`rubric.yml`'s `audience.doc_version_pinned` at minimum.) Script to verify.
- [ ] When `rubric.yml` version bumps, what historic `decision_record` rows reference the prior version? Migration policy: re-score on bump? Lazy re-score? Never re-score?
- [ ] Lint: every PR that changes `rubric.yml` must verify the audience.md `doc_version_pinned` is current.

**Why it matters.** Version pinning between files is currently manual. Drift is silent.

**Defer until:** there's enough version churn to warrant tooling. For now, the lint check (manual via PR review) is sufficient.

---

## Calibration items *(deferred until corpus exists)*

These are not bugs. They are settings that need real data to calibrate. Until corpus exists, they stay at strawman values. Documented in `rubric.yml` `calibration_notes:` block; surfaced here for visibility.

| Item | Current | Suspected better | Blocked on |
|---|---|---|---|
| `recency_bonus` decay shape | linear | exponential or piecewise | engagement loop output |
| `routing.feed_threshold` value | 37 (~36% of max 103) | TBD | ≥ 100 real scored items |
| `target_mix_per_7d` | 30/30/20/15/5 | TBD | audience-realization loop (itself blocked) |
| Persona multipliers | hand-set | derive from data | ≥ 100 decisions per persona |

---

## Edit policy for this file

- New items go to the appropriate section (or open a new section if none fits).
- Items that ship get removed (or moved to a `## Done` log if the history matters).
- This file is reviewed alongside `rubric.yml` and `audience.md` at the monthly engagement-loop review.
- Don't let this become a wishlist. Every item should have a clear "what unblocks it" line. If you can't write that line, the item isn't ready to be tracked.

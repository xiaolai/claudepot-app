# claudepot.com — audience, voice, style

The constitution for who we serve and how we sound. Single source of truth.
Every downstream artifact — `rubric.yml`, `personas/*.yml`, marketing copy, `/about` pages, comment guidelines, digest emails — should *reference* this doc, not restate it.

Version: 0.1.3
Updated: 2026-05-01

---

## 1. Audience

### Primary
Anyone who uses AI tools for real work and wants to use them better. Tech or non-tech, code or non-code, IC or team lead.

### Binding traits
The audience is defined by attitude, not role. Three traits, all required:

1. **Actually uses AI tools daily.** Past curiosity, past experimentation. Claude / ChatGPT / Cursor / Aider / Copilot / Windsurf / etc. are open right now.
2. **Cares about efficiency.** The unifying interest across all sub-segments — how real work gets done better, faster, cleaner.
3. **Willing to learn cross-domain.** Tolerates technique even when it's not from one's own field. A writer reads a coder's prompt. An engineer reads a teacher's workflow.

### Sub-segments
Four domains under the same attitude.

| Sub-segment | Examples | What they want from a daily reader |
|---|---|---|
| **knowledge_workers** | writers, lawyers, teachers, marketers, researchers | Workflow patterns, prompt techniques, lived-experience reports, "this saved 4 hours per draft — here's the prompt" |
| **engineers** | software engineers, AI engineers, infra builders | Mechanism with enough detail to reproduce, evals, agentic patterns, model selection, dev velocity, working code |
| **operators** | founders, consultants, indie operators | Tools that compounded over months, GTM applications, "replaced 3 tools with 1 prompt" with the prompt shown |
| **learners** | students, career-switchers, AI-curious-and-committed | Mental models, primers, "what's worth learning next," the first-30-days path |

### Not for
| Not for | Why exclude |
|---|---|
| Casuals who want shortcuts without effort | "10 ChatGPT prompts to..." — wrong product |
| AI skeptics looking for ammunition | A different conversation, valid but not this one |
| Pure consumers of plain-English explainers | They want digest; we want technique |
| Hardcore arxiv researchers | Different vehicle (arxiv-sanity, conferences); we'd dilute both |
| AI-hype influencers | Their content is the slop the rubric is designed to reject |

### Reader needs, by sub-segment
A piece that serves all four well is rare. Most pieces serve one or two. The rubric's `domain_legibility` criterion exists so readers can self-route.

- **knowledge_workers** want a technique applicable to tomorrow's draft / case / class / campaign / paper. They want the actual prompt. They don't want code unless learning to read it.
- **engineers** want mechanism reproducible from the description. Evals to trust. Code or pseudocode. What broke and how it was fixed.
- **operators** want what compounded over time. What didn't. Time saved with specifics. Tools that survived three months of real use.
- **learners** want where to start, what to ignore, the mental models that make the rest legible.

---

## 2. Voice

No reference voice. We observe established voices to learn what works, then derive our own from first principles. Six principles, in priority order.

### 2.1 Plural, not singular
"We" is the editorial voice. Readers are implicitly part of "we" by virtue of using the platform — not a target audience to be addressed. We don't say "you should" — we say "we ship," "we observe," "the work benefits," "the technique applies when…".

When "you" feels unavoidable, the sentence is wrong. Restructure:
- *Avoid:* "You should always include the failure modes when documenting a workflow."
- *Prefer:* "Workflows are more useful when failure modes are documented alongside them."

The pronoun choice positions claudepot.com as a community of practice, not a teacher with students.

### 2.2 Compressed, not decorated
Short sentences earn their length. No throat-clearing. Adjectives only when load-bearing. No transition phrases. No restating the obvious. A sentence that adds no information doesn't ship.

### 2.3 Mechanism, not adjective
Describe what something does. Don't describe how exciting it is. "A 2-step retrieval with reranking" carries more than "a powerful new retrieval approach." "Cuts eval latency from 4s to 600ms" carries more than "dramatically faster."

### 2.4 Confidence calibrated, not hedged or absolute
State what's verified flatly. State what's uncertain plainly. Avoid both extremes:
- *Avoid:* "Maybe this works, who knows." *(false modesty)*
- *Avoid:* "Obviously this is the right approach." *(false certainty)*
- *Prefer:* "We tested this for 2 weeks. Works on X, breaks on Y."
- *Prefer:* "We haven't verified this, but the mechanism described is plausible."

### 2.5 Curatorial, not opinionated
The editorial voice describes what's there: what the piece argues, what evidence backs it, who it's for. The platform doesn't take positions on contested AI questions — comments do. When a controversial release happens, the editorial line is *"here's the primary source, here's what's claimed, here's what's evidenced."* Persuasion happens in threads, not in the editorial frame.

### 2.6 Humor lives in personas, not the base voice
The institutional voice is humor-neutral. Each editorial persona (`ada`, `ClauDepot`, `historian`, `scout`) carries its own register — dry, warm, wry, curious. The base voice that wraps the platform is neutral so that personas can vary without dissonance.

---

## 3. Style

### 3.1 Sentence patterns
- Short over long. Three short sentences usually beat one long compound.
- Verbs over nouns. "We ship" beats "shipping is what we do."
- Concrete over abstract. "Saves 2 hours per draft" beats "improves writing efficiency."
- One clause that earns its existence. If a clause adds no new information, cut it.

### 3.2 Formatting
- Markdown tables for any structured comparison.
- Lists for parallel items only. If items aren't parallel, prose carries them better.
- No emoji as emphasis. (Lucide icons for UI per `.claude/rules/icons.md`; prose stays plain.)
- No exclamation points. No ALL CAPS for emphasis. Italics or asterisks for emphasis, sparingly.

### 3.3 Glossary — preferred
| Use | Not |
|---|---|
| AI tools | AI tooling |
| with AI tools | with Claude *(more inclusive)* |
| ship | build *(when delivery is the point)* |
| technique | trick, hack, magic |
| pattern | recipe *(when the structure is the point)* |
| we observe / we think / we doubt | I think *(in editorial commentary)* |
| works | solves *(rarely solves anything definitively)* |

### 3.4 The constitutional nevers

**Scope.** These patterns govern *our output* — editorial commentary ("why we picked this"), comments from agents, marketing copy, digest emails, `/about` pages. They are **not** auto-rejects against submitted content based on its title or surface framing.

A submitted link whose title contains "unlock" or "10x" is still evaluated by `rubric.yml`'s `hard_rejects` and `inclusion_gates` — *against the body, not the title*. The constitutional nevers determine how *we* sound when describing or framing the piece; they do not decide whether *they* get in. A primary-source release post with hype in its title can still ship; the editorial commentary just has to reframe it without inheriting the hype.

Eight patterns that, if they appear in our output, mean the voice is broken. The draft fails the audience.md test and gets *rewritten*, not edited.

| # | Pattern | Examples |
|---|---|---|
| 1 | **Hype vocabulary** | "unlock," "supercharge," "10x," "game-changer," "revolutionize," "the future of," "ushering in," "next era of" |
| 2 | **Second-person imperative** | "you should," "you need to," "you must" — restructure to "we" or declarative |
| 3 | **LLM-slop connectives** | "it's worth noting that," "furthermore," "in conclusion," "let's dive into," "buckle up"; em-dash density that signals AI authorship |
| 4 | **Claim without evidence** | "obviously," "clearly," "everyone knows," "it's clear that" — presume a consensus that may not exist |
| 5 | **Insider snark / tribal signaling** | "as we all know," cliquey AI-Twitter humor, in-group references that exclude non-techs from a cross-domain audience |
| 6 | **Condescension / lecture posture** | "here's what most people get wrong," "most people don't realize," "the thing about [topic] is…" — talking down |
| 7 | **Filler openers** | "I've been thinking a lot about," "three things I want to share," storytime intros that delay the substance |
| 8 | **Manufactured excitement** | exclamation points, ALL CAPS, 🚀 emoji as emphasis, "mind = blown," cliffhanger sentence breaks |

These are *constitutional* in the sense that they bind the voice's identity. Other style preferences (Oxford comma, en-dash vs em-dash, etc.) are non-constitutional — they can drift without breaking who the platform is. These eight cannot.

---

## 4. Examples

### 4.1 In-voice (annotated)

**Editorial "why we picked this" line on a submission:**
> A 2-week eval comparing four prompt strategies on Anthropic's API for legal document review. Numbers on each, including where the "best" strategy fails. Useful for tool-shippers building any retrieval-heavy workflow, not just lawyers.

*Why this works:* states what the piece is, names the evidence, names the failure modes the piece itself surfaces, routes the reader with a cross-domain bridge in the last sentence. No adjectives. No "you."

**Digest email opener:**
> Three picks this week. Two are workflows we hadn't seen before; the third is a paper that contradicts the consensus on long-context evaluation. Ranked by likely-Monday-impact.

*Why this works:* states the contents, names the angle (consensus contradiction), frames the ordering by reader value. Plural pronoun. No filler. Note: "ranked" instead of "we've ranked" — even shorter, still no "you."

**Editorial comment in a thread:**
> The benchmark in the post uses 3-shot prompting; the baseline it compares against uses 0-shot. The numbers move ~12pp when both use the same setup. Worth holding the conclusion lightly until that's controlled for.

*Why this works:* curatorial — names a methodological issue without taking a position on the conclusion. Specific number. "Worth holding the conclusion lightly" instead of "you should be skeptical" — restructured away from second-person.

### 4.2 Not in-voice (annotated)

**Hype slop:**
> 🚀 This INCREDIBLE new prompting technique is going to UNLOCK a whole new era of AI productivity! You won't believe how much faster you'll ship.

*Fails:* hype vocabulary ("incredible," "unlock," "new era"), ALL CAPS, emoji-as-emphasis, second-person, manufactured excitement, vibes-only claim with no evidence. Hits constitutional nevers 1, 2, 4, 8.

**Substack-essay opener:**
> I've been thinking a lot about how AI agents are changing the way we work. Three thoughts I want to share with you.

*Fails:* filler opener ("I've been thinking a lot about"), "I" instead of "we" (singular voice instead of editorial collective), "you" used reflexively, "three thoughts" listicle frame, no actual content yet — pure throat-clearing. Hits nevers 2, 7.

**LinkedIn-influencer:**
> The thing about most AI tools? They're missing the point.
>
> Here's what most people don't realize:

*Fails:* cliffhanger sentence break, condescension ("most people don't realize"), manufactured tension, no substance arrives, "you" implied via "most people," lecture posture. Hits nevers 6, 8.

---

## 5. How this doc is used

| Surface | How it consumes audience.md |
|---|---|
| `editorial/rubric.yml` | `audience.doc: editorial/audience.md`. Sub-segment IDs imported; criteria reference voice principles. |
| `editorial/rubric.yml` `persona_overlays:` *(inline today)* | Persona definitions live inline in `rubric.yml` for now. They will split out to `editorial/personas/*.yml` when any persona spec exceeds 15 lines or a 5th persona is added. The base voice stays here either way. |
| Marketing copy (bio, pinned posts, /about) | Drafted from §2 and §3, validated against §3.4. |
| Comment guidelines for editorial agents *(planned — see `TODO.md`)* | Will reference §2.5 — agents are curatorial in editorial mode, may take positions in comment mode. |
| Digest email templates | Voice from §2; structure follows §4.1 example 2. |
| `/about/rubric` and `/about/voice` *(planned site pages — see `TODO.md`)* | Generated from this doc. Don't hand-maintain a parallel copy. |
| `editorial/anchors/` exemplars | Each anchor's `expected_reason` field uses voice from §2 + criterion vocabulary from `rubric.yml`. |
| `editorial/transparency.md` | The transparency policy. Defines what becomes public via `/office/` and what stays private in the `claudepot-office` private repo. References this doc's voice rules for the rendered text on `/office/`. |
| `/office/` page on claudepot.com | The single public window into the editorial team's machinery. Renders agent activity, persona profiles, decision pages, source list, audit reports, override log. Voice on `/office/` is this doc's voice — plural, curatorial, no humor at base level. |

## 6. Versioning

Reviewed monthly alongside `rubric.yml`. Versioning policy in `editorial/README.md`. Bump:

| Bump | When |
|---|---|
| Patch | Glossary entries, examples, copy edits |
| Minor | Voice or style principles added/removed; sub-segment definitions change |
| Major | Audience primary definition or binding traits change |

Old artifacts (rubric versions, persona definitions, marketing copy) record the `audience.md` version they were written against, so historical drift is interpretable.

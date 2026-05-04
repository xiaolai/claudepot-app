---
description: When to use icon-only buttons vs labeled buttons vs plain text buttons
when_applicable:
  - Adding a new button anywhere in `src/`
  - Refactoring a row of action buttons
  - Reviewing a PR that introduces button UI
---

# Icon button system — three tiers

Two primitives carry every button:
- `<IconButton glyph={NF.x} title="…" aria-label="…" />` — square, 22/28/32 px,
  icon-only with hover tooltip. Tooltip carries the verb; aria-label
  matches.
- `<Button variant="solid|ghost|subtle|outline|accent" glyph={NF.x}>Label</Button>`
  — labeled with optional leading glyph. The whole label is read by
  assistive tech; no separate aria-label needed.

The choice is **about the user reading the surface**, not about saving
pixels. If the user can scan the surface and know what each button
does without reading, icon-only is correct. Otherwise label it.

## Tier 1 — Icon-only (`<IconButton>`)

Use when **all four** hold:

1. **Universal verb mapping**: refresh / copy / close / search / trash
   (in lists) / edit / run / settings / chevron-toggle / ellipsis-menu.
   The icon means one thing across the web.
2. **Repeated or scattered**: the button appears in a list row, a
   toolbar row alongside other icon controls, or the table header
   chrome. Pure icons compose into rhythm.
3. **Secondary-action prominence**: never the screen's primary action.
4. **Compact context**: the surrounding column is dense (≤ 32 px tall
   per row, or a chrome strip with multiple sibling buttons).

Tooltip text is mandatory. Aria-label matches the tooltip.

## Tier 2 — Icon + label (`<Button glyph={…}>Label</Button>`)

Use when:

- The action is the **only one of its kind on the surface** (e.g. a
  single "Refresh" in an empty toolbar — context can't carry the
  verb).
- The button sits at primary-action prominence (`variant="solid"`,
  empty-state CTAs, onboarding actions). Discoverability beats
  density.
- The icon is recognizable but the action's effect deserves
  reinforcement (e.g. "Verify all accounts" with shield glyph — the
  shield could mean a dozen things; the label disambiguates).

## Tier 3 — Text only (`<Button>Label</Button>`)

Use when:

- **No clear icon** maps to the verb: "Sync from current", "From
  template…", "Adopt", "Skip this version", "Show again", "Move to
  main".
- **High-stakes confirmation** in a modal footer: "Cancel", "Move",
  "Empty trash — confirm". Text reduces accidental commitment;
  destructive icons in modals invite misclicks.
- **Dynamic / state-bearing label**: "Restoring…", "Starting…", "Move
  to <project>", "Adopting…". Replacing the text with an icon hides
  the state.
- **Toggle states encoded in the label**: "Show runs / Hide runs",
  "Disable / Enable". The label IS the state; an icon would need a
  legend.

## Anti-patterns

- **Icon-only for primary actions in onboarding**: "Add account" in an
  empty Accounts list must be Tier 2 — discoverability > density.
- **Icon-only for destructive actions outside dense lists**: a lone
  trash icon as the only visible cue is misclick-bait. Pair with a
  confirm dialog; label if standalone.
- **Tooltips for required disclosure**: per `design.md`'s "disabled
  buttons state a reason inline", a tooltip is *not* a substitute for
  an inline reason next to a disabled button. Tooltips name the verb;
  inline notes carry the why.
- **Ambiguous icons (gear, shield, sparkle) without label**: too many
  meanings; promote to Tier 2.
- **Icon-only inside a high-density modal footer**: modals tear the
  user's attention away; text in the footer reduces friction. Always
  Tier 3.

## Migration checklist (existing button → icon-only)

1. Confirm Tier 1 criteria. If any fail, stop.
2. Replace `<Button glyph={NF.x}>Label</Button>` with
   `<IconButton glyph={NF.x} title="Label" aria-label="Label" />`.
3. If the surrounding `aria-pressed` / `aria-expanded` / data
   attributes were on the `<Button>`, forward them — `IconButton`
   accepts the same aria props.
4. Drop any `variant`, `size`, `danger` props that don't map.
   `IconButton` size is `"sm" | "md" | "lg"`; default is `md` (28 px).
5. Test on hover: tooltip must appear within 250 ms and read the same
   verb the label carried.

## Surfaces already correct (do not touch in routine refactors)

- Modal footers (Cancel / Confirm / Move to / Empty trash — confirm).
- Empty-state CTAs ("Add account", "Add automation", "Verify all").
- Toggle-state buttons in cards (Show runs / Hide runs, Enable /
  Disable).
- Domain-specific actions without icon mapping ("Sync from current",
  "Adopt", "From template…").

If the audit flags one of these, re-read the rules above before
converting — they're flagged on purpose.

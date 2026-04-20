---
description: Three-pass review of UI changes — principles, patterns, and tokens. Requires a screenshot or running build for render-time checks.
---

# Design Review

You are reviewing UI changes in a Tauri 2 + React + plain CSS project
that uses the **paper-mono** design system: JetBrains Mono Nerd Font
everywhere, OKLCH palette with a single terracotta accent, custom
`WindowChrome` + 240px `AppSidebar` + content + `AppStatusBar`,
NF-glyph iconography via the `Glyph` primitive.

Run **three passes** in order. Later passes do not excuse earlier
failures. Do not skip any pass.

Some Pass C checks are render-time properties a diff cannot see.
**Pass C requires a screenshot or a running `pnpm tauri dev` window.**
If neither is provided, ask for one before continuing. Do not approve
Pass C from code alone.

## Step 1 · Identify changed UI files

```bash
git diff --name-only -- 'src/**/*.tsx' 'src/**/*.css' 'src/**/*.ts' 'src/styles/**'
git diff --cached --name-only -- 'src/**/*.tsx' 'src/**/*.css' 'src/**/*.ts' 'src/styles/**'
```

Combine and deduplicate. If no UI files changed, report
"No UI files modified" and stop.

## Step 2 · Load the design system

Read in this order:

1. `.claude/rules/design-principles.md` — why
2. `.claude/rules/design-patterns.md` — composition recipes + the
   feedback ladder (selection table for state→surface)
3. `.claude/rules/ui-design-system.md` — tokens and defaults
4. `.claude/rules/no-raw-values.md` — BLOCK-level rule: every color,
   size, shadow, duration, and dimension in UI code must come from
   `src/styles/tokens.css`. No raw hex / rgb / oklch / numeric
   fontSize / numeric padding / numeric dimension.
5. `.claude/rules/accessibility.md` — a11y floor
6. `.claude/rules/react-components.md` — file-shape conventions

Legacy native-macOS rules sit under `.claude/rules/_legacy/` for
archaeology. Do **not** evaluate new work against them.

## Step 3 · Read the diffs

`git diff HEAD -- <file>` for each changed UI file. Read the full
diff, not summaries.

---

# Pass A · Principles (the hardest gate)

Check each changed file against `design-principles.md`. A violation
here is always **BLOCK** — tokens and recipes do not rescue a
principle failure.

For each principle, answer yes or no with a specific line reference:

- **§1 Implementation detail never outranks user identity.** Does
  this change put any DB key, slug, UUID, or internal path on a
  primary surface? Internal IDs only behind `DevBadge` (visible only
  in Developer mode) or in a context-menu "Copy ID".
- **§2 Selected object is unmistakable.** In each view touched, can a
  cold user tell which item is selected in under one second? The
  paper-mono signature is `--bg-active` background + 2–3px accent
  left border + weight 600.
- **§3 Destructive actions state consequence inline.** Does every new
  destructive action carry (a) a verb-and-count label, (b) an inline
  consequence hint, (c) friction proportional to reversibility?
- **§4 State legibility beats chrome restraint.** Is any consequential
  state hidden behind hover, a tiny icon, or a tooltip?
- **§5 One signal per surface.** Does any new event fire in more than
  one surface? Cross-check against the feedback ladder in
  `design-patterns.md`.
- **§6 Reversibility shapes friction.** Does friction match blast
  radius? Never too much, never too little.
- **§7 Keychain and credential state are first-class.** If the change
  touches credentials, is state persistent chrome (banner, status
  strip, AnomalyBanner) and not buried in a settings panel? Any
  secret at risk of being logged or rendered?
- **§8 Five-Second Test.** Cold-eyes test:
  1. What is selected?
  2. What state is it in?
  3. What will the biggest button do?
- **§9 Dark-first parity.** Does every new color token define both
  light and dark values? Are screenshots from both themes?
- **§10 No emoji, no icon library.** Every glyph is an NF codepoint
  rendered via `Glyph` (or, for legacy code paths, `Icon`). Any
  imported `lucide-react`, `heroicons`, `@phosphor-icons`, or emoji
  is BLOCK.

Principle pass report format:

| Principle | File | Status | Evidence |
|---|---|---|---|
| §1 | … | PASS / BLOCK | line ref + what's wrong |

---

# Pass B · Patterns and surfaces

Check the diff against the recipes and feedback ladder in
`design-patterns.md`. Most checks are render-time-ish — use the
screenshot or running build to confirm.

### Hierarchy

- [ ] One primary navigation surface (the `AppSidebar`). No fixed
  third nav layer.
- [ ] One `Button variant="solid"` visible per view (the primary
  action).
- [ ] One accent-colored item at rest.

### Window shell

- [ ] Top region is the `WindowChrome` (38px, breadcrumb + ⌘K +
  theme toggle). Do not hand-roll a substitute.
- [ ] Sidebar is the `AppSidebar` (240px, swap targets + nav + tree
  + sync strip).
- [ ] Bottom is the `AppStatusBar` (24px). Stats are filtered for
  zero values.
- [ ] Section bodies render through a `ScreenHeader` at the top.

### List rows

- [ ] Selectable rows follow the listbox recipe (`<li role="option"
  aria-selected tabIndex={0}>`), not a bare `<button>` or `<div>`.
- [ ] Metadata has zero-values filtered out. Never `0 sessions · …`.
- [ ] Max 2-3 metadata fields. More belongs in the detail pane.

### Detail pane / cards

- [ ] Every visible field is user-meaningful. No DB slugs, UUIDs.
  Internals only via `DevBadge`.
- [ ] Zero-state rows dropped.

### Controls

- [ ] Segmented controls carry text labels a cold user understands
  on first read. Counts on the active segment when load-bearing.
- [ ] Buttons use one of the five variants (`solid · ghost · subtle ·
  outline · accent`); `danger` is a modifier, not a variant.
- [ ] `IconButton` carries an `aria-label`.
- [ ] Disabled buttons carry an inline reason (not just a `title`).

### Feedback surfaces

For each new state change in the diff, name the surface used and
compare to the feedback ladder selection table in
`design-patterns.md`.

- [ ] One surface per state (no toast + banner echoes).
- [ ] Long ops own their status via `RunningOpStrip` alone (no
  toast-while-running).
- [ ] Persistent state uses a banner, not a toast.
- [ ] Modals are for blocking; completion uses a toast.

### Window chrome (screenshot required)

- [ ] `WindowChrome` renders the breadcrumb + ⌘K hint + theme toggle
  cleanly with no overlap with the OS traffic lights (left padding
  is set to 80px to leave room).
- [ ] First list row not clipped by sticky filter bar.
- [ ] Vertical separators run uninterrupted top-to-bottom.
- [ ] Dark mode parity: same screenshot, dark theme, looks coherent.

### Iconography

- [ ] Every new glyph is an NF codepoint, drawn via `Glyph` (or, for
  legacy code paths, `Icon`).
- [ ] No SVG icon library imported. No emoji.
- [ ] If a new glyph was needed, it was added to the `NF` map in
  `src/icons.ts` with a comment and used by name.

---

# Pass C · Tokens and a11y

Mechanical. A linter could do this — run it consistently.

### BLOCK-level

- [ ] **No raw values anywhere in paper-mono code.** Per
  `no-raw-values.md`, every color, font size, spacing, dimension,
  shadow, radius, duration, easing, opacity, border width, line
  height, letter spacing, and z-index comes from a token in
  `src/styles/tokens.css`. Run the audit greps in that rule file
  before approving:
  ```
  rg -nE "(#[0-9a-fA-F]{3,8}\b|(rgba?|hsla?|oklch)\s*\()" \
    --glob '!src/styles/tokens.css' --glob '!src/App.css' \
    --glob '!src/components/*.tsx' src/
  rg -nE "(fontSize|padding|margin|gap|width|height|borderRadius|lineHeight|zIndex|opacity):\s*['\"]?[1-9]" \
    --glob '!src/styles/tokens.css' --glob '!src/App.css' \
    --glob '!src/components/*.tsx' src/ | rg -v "var\(--"
  ```
  Anything surfaced that is not on the allow-list in
  `no-raw-values.md` (0, keywords, percentages, fr, font weights) is
  BLOCK. Legacy unported components under `src/components/*.tsx`
  and `src/App.css` are currently exempt and thus excluded above.
- [ ] Every new color token has both light and dark values defined
  in `src/styles/tokens.css` (principle §9).
- [ ] New color tokens occupy a defined role (background / surface /
  text / line / accent / semantic). Never a token without a role.
- [ ] No `window.confirm/alert/prompt`.
- [ ] Modals: `role="dialog"`, `aria-modal`, `aria-labelledby`,
  Escape handler, focus trap (use `Modal` + `useFocusTrap`).
- [ ] Icon-only buttons: `aria-label` present.
- [ ] Keyboard-reachable (Tab + Enter/Space on custom interactives).
- [ ] `:focus-visible` outline present on new interactives, scoped
  with `outline-offset: -1px` if the element has its own border.
- [ ] Color is never the sole indicator of state — pair with text or
  icon (use `Tag` rather than rolling your own).

### Buttons (BLOCK)

- [ ] New buttons use one of the five `Button` variants (`solid ·
  ghost · subtle · outline · accent`). `danger` is a modifier.
- [ ] Exactly one `solid` button visible per view.
- [ ] Destructive irreversible-global actions use `Button
  variant="solid" danger` with consequence copy inline.

### WARN-level

- [ ] Font sizes from the `--fs-xs · sm · base · md · lg · xl · 2xl`
  scale.
- [ ] Spacing on the 4px grid (`--s-1 … --s-16`).
- [ ] Border radius matches element type (`--r-1` badges · `--r-2`
  buttons/rows · `--r-3` modals/cards · `--r-pill` pills).
- [ ] Animations only `opacity` / `transform`, never layout
  properties.
- [ ] No `::-webkit-scrollbar` overrides outside `tokens.css`.
- [ ] No box shadows on list items, list rows, chrome, or detail
  rows. Elevation reserved for popovers, dropdowns, modals.
- [ ] New animations wrapped in `prefers-reduced-motion: no-preference`.
- [ ] New border/separator tokens have `prefers-contrast: more`
  variant.
- [ ] New translucent surfaces have `prefers-reduced-transparency`
  fallback.

### Component shape (WARN)

- [ ] One component per file; helpers co-located only when scoped to
  the public component.
- [ ] Props interface inline above the component; no `any`.
- [ ] Named export (the only default export is `App.tsx`).
- [ ] Icons via `Glyph` from `src/components/primitives/Glyph.tsx`
  (paper-mono) or `Icon` from `src/components/Icon.tsx` (legacy
  surfaces being kept). Never lucide / heroicons / SVGs / emoji.
- [ ] Data via `src/api.ts`, not direct `invoke()`.
- [ ] State across `await`: functional updater
  (`setState(prev => …)`).
- [ ] Inline styles drawn from token vars; new class-based CSS in
  `src/App.css` only when extending an unported legacy component.

---

## Step 4 · Report

Output **three tables**, one per pass. Do not collapse.

### Pass A · Principles

| Principle | File | Status | Evidence |
|---|---|---|---|

### Pass B · Patterns & surfaces

| File | Issue | What to do instead | Reference |
|---|---|---|---|

### Pass C · Tokens & a11y

| File | Rule | Line | Severity |
|---|---|---|---|

Severity model:

- **BLOCK** — any Pass A failure; any Pass B named anti-pattern (the
  ten in `design-patterns.md`); Pass C BLOCK-level rules above.
- **WARN** — Pass C WARN-level rules; minor Pass B deviations not
  on the named anti-pattern list.
- **NOTE** — optional improvements.

If all three tables are empty:

> Three-pass review clean. Changes comply with principles, patterns,
> and tokens.

If Pass A passes but later passes fail:

> Principles clean. Later passes found issues — see below.

If Pass A fails:

> **Pass A BLOCK — principle violation.** Do not merge. Fix before
> continuing Pass B or C.

## Note on Pass C alone

If a PR is token-compliant but fails Pass A or B, the PR still fails.
Token compliance does not mean the design is right. This is the
entire reason for the three-pass structure.

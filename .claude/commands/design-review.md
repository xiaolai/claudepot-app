---
description: Three-pass review of UI changes — principles, patterns, and tokens. Requires a screenshot or running build for render-time checks.
---

# Design Review

You are reviewing UI changes in a Tauri 2 + React + plain CSS project.
Run **three passes** in order. Later passes do not excuse earlier
failures. Do not skip any pass.

Some Pass C checks are render-time properties a diff cannot see.
**Pass C requires a screenshot or a running `pnpm tauri dev` window.**
If neither is provided, ask for one before continuing. Do not approve
Pass C from code alone.

## Step 1 · Identify changed UI files

```bash
git diff --name-only -- 'src/**/*.tsx' 'src/**/*.css' 'src/**/*.ts'
git diff --cached --name-only -- 'src/**/*.tsx' 'src/**/*.css' 'src/**/*.ts'
```

Combine and deduplicate. If no UI files changed, report
"No UI files modified" and stop.

## Step 2 · Load the design system

Read in this order:

1. `.claude/rules/design-principles.md` — why
2. `.claude/rules/design-references.md` — what to imitate
3. `.claude/rules/feedback-ladder.md` — which surface for which state
4. `.claude/rules/design-patterns.md` — composition recipes
5. `.claude/rules/ui-design-system.md` — tokens and defaults
6. `.claude/rules/accessibility.md` — a11y floor
7. `.claude/rules/react-components.md` — component conventions

## Step 3 · Read the diffs

`git diff HEAD -- <file>` for each changed UI file. Read the full
diff, not summaries.

---

# Pass A · Principles (the hardest gate)

Check each changed file against `design-principles.md`. A violation
here is always **BLOCK** — tokens and references do not rescue a
principle failure.

For each principle, answer yes or no with a specific line reference:

- **§1 Implementation detail never outranks user identity.** Does this
  change put any DB key, slug, UUID, or internal path on a primary
  surface?
- **§2 Selected object is unmistakable.** In each view touched, can a
  cold user tell which item is selected? Does selection survive
  `prefers-contrast: more`?
- **§3 Destructive actions state consequence inline.** Does every new
  destructive action have (a) a verb-and-count label, (b) an inline
  consequence hint, (c) friction proportional to reversibility?
- **§4 State legibility beats chrome restraint.** Is any consequential
  state hidden behind hover, a tiny icon, or a tooltip?
- **§5 One signal per surface.** Does any new event fire in more than
  one feedback surface? Check against `feedback-ladder.md`.
- **§6 Reversibility shapes friction.** Does friction match blast
  radius? Never-too-much, never-too-little.
- **§7 Keychain and credential state are first-class.** If the change
  touches credentials, is state persistent chrome? Any secret at risk
  of being logged or rendered?
- **§8 Five-Second Test.** Can a cautious developer, seeing this view
  cold, answer in 5 seconds:
  1. What is selected?
  2. What state is it in?
  3. What will the biggest button do?

Principle pass report format:

| Principle | File | Status | Evidence |
|---|---|---|---|
| §1 | … | PASS / BLOCK | line ref + what's wrong |

---

# Pass B · Patterns and surfaces

Check the diff against `design-patterns.md` and `feedback-ladder.md`.
Most of these are render-time-ish — use the screenshot or running
build to confirm.

### Hierarchy

- [ ] One primary navigation surface (nav rail). No fixed third nav
  layer.
- [ ] One `.btn.primary` visible at a time.
- [ ] One accent-colored item at rest.

### List rows

- [ ] Selectable rows follow the listbox recipe (`<li role="option"
  aria-selected tabIndex={0}>`), not a bare `<button>`.
- [ ] Metadata has zero-values filtered out. Never `0 sessions · …`.
- [ ] Max 2-3 metadata fields. More belongs in the detail pane.

### Detail pane

- [ ] Every visible field is user-meaningful. No DB slugs, UUIDs.
- [ ] Zero-state rows dropped.
- [ ] Right-aligned label column, left-aligned values.

### Controls

- [ ] Segmented controls carry text labels a cold user understands
  on first read.
- [ ] Counts appear on the active segment only.
- [ ] Disabled buttons carry an inline reason (not just a `title`).

### Feedback surfaces

For each new state change in the diff, name the surface used and
compare to the selection table in `feedback-ladder.md`.

- [ ] One surface per state.
- [ ] Long ops own their status via `RunningOpStrip` alone (no
  toast-while-running).
- [ ] Persistent state uses a banner, not a toast.
- [ ] Modals are for blocking; completion uses a toast.

### Window chrome (screenshot required)

- [ ] Title bar unified with nav rail. Traffic lights float on rail
  vibrancy. No horizontal separator in the rail's top zone.
- [ ] First list row not clipped by sticky filter bar.
- [ ] Vertical separators run uninterrupted top-to-bottom.

### Reference app match

Pick the most relevant reference from `design-references.md` and
answer: if this view were dropped into that app, would it look like
it belonged?

- Trust-critical state? Compare to **Keychain Access**.
- List-detail layout? Compare to **1Password 8** or **Xcode**.
- Segmented control? Compare to **Xcode**.
- Detail pane? Compare to **System Settings**.
- Destructive maintenance flow? Compare to **Disk Utility**.

Name the reference. State yes/no with a sentence.

---

# Pass C · Tokens and a11y

Mechanical. A linter could do this — run it consistently.

### BLOCK-level

- [ ] No raw hex/rgb/hsl colors in component CSS; every color via `var(--token)`
- [ ] No `window.confirm/alert/prompt`
- [ ] Modals: `role="dialog"`, `aria-modal`, `aria-labelledby`, Escape handler, focus trap
- [ ] Icon-only buttons: `aria-label` present
- [ ] Keyboard-reachable (Tab + Enter/Space on custom interactives)
- [ ] `:focus-visible` outline present on new interactives
- [ ] Color is never the sole indicator of state

### WARN-level

- [ ] Font sizes from the scale in `ui-design-system.md`
- [ ] Spacing on the 4 px grid
- [ ] Border radius matches element type (6 / 8 / 10 / 12 / 999)
- [ ] Transitions: 0.12 s ease; animates only `opacity` / `transform`
- [ ] No `cursor: pointer` outside `<a>` tags
- [ ] No `::-webkit-scrollbar` overrides
- [ ] No box shadows on list items
- [ ] Dark-mode variants present for any new token
- [ ] New animations wrapped in `prefers-reduced-motion: no-preference`
- [ ] New border/separator tokens have `prefers-contrast: more` variant
- [ ] New translucent surfaces have `prefers-reduced-transparency` fallback

### Component shape (WARN)

- [ ] One component per file, under 120 lines
- [ ] Props interface inline above the component
- [ ] Named export
- [ ] Icons from `lucide-react` only
- [ ] Data via `src/api.ts`, not direct `invoke()`
- [ ] State across `await`: functional updater

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

- **BLOCK** — any Pass A failure; any Pass B named-anti-pattern
  (the ten in `design-patterns.md`); Pass C BLOCK-level rules above.
- **WARN** — Pass C WARN-level rules; minor Pass B deviations not on
  the named-anti-pattern list.
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
Token compliance does not mean the design is right. This is the entire
reason for the three-pass structure.

---
description: Review UI changes against the paper-mono design rules. Requires a screenshot or running `pnpm tauri dev` for render-time checks.
---

# Design Review

Audit the current UI diff against the paper-mono rules in
`.claude/rules/design.md`. Some checks are render-time properties a
diff cannot see — **ask for a screenshot or a running build before
approving anything in that category.**

## Step 1 · Identify changed UI files

```bash
git diff --name-only -- 'src/**/*.tsx' 'src/**/*.css' 'src/**/*.ts' 'src/styles/**'
git diff --cached --name-only -- 'src/**/*.tsx' 'src/**/*.css' 'src/**/*.ts' 'src/styles/**'
```

If no UI files changed, report "No UI files modified" and stop.

## Step 2 · Load the rules

Read `.claude/rules/design.md` and `src/styles/tokens.css`. That's
the full rule set — don't look anywhere else.

## Step 3 · Read each diff

`git diff HEAD -- <file>` for each changed file. Read the full diff.

## Step 4 · Check against the rules

Score every finding as **BLOCK** or **WARN**.

### BLOCK

- **Raw CSS values.** Any hex/rgb/hsl/oklch literal, numeric
  fontSize/padding/margin/gap/width/height/borderRadius/zIndex/
  opacity, raw shadow, raw duration — in any file under `src/`
  outside `tokens.css` and `App.css` component classes. Grep:
  ```bash
  rg -nE "(#[0-9a-fA-F]{3,8}\b|(rgba?|hsla?|oklch)\s*\()" \
    --glob '!src/styles/tokens.css' --glob '!src/App.css' src/
  rg -nE "(fontSize|padding|margin|gap|width|height|borderRadius|lineHeight|zIndex|opacity):\s*['\"]?[1-9]" \
    --glob '!src/styles/tokens.css' --glob '!src/App.css' src/ | rg -v "var\(--"
  ```
  Allowed literals: `0`, percentages, keywords (`auto`, `inherit`,
  `transparent`, `currentColor`), `fr` / `calc()` / `minmax()`, font
  weights (`400`/`500`/`600`/`700`).
- **Extra `:root { }` block.** Tokens are declared exactly once, in
  `tokens.css`. Grep must return zero:
  ```bash
  rg -n ":root\s*\{" src/ --glob '!src/styles/tokens.css'
  ```
- **Token re-declaration in `App.css`.** Grep must return zero:
  ```bash
  rg -n "^\s*--(accent|bg|surface|border|text|ok|bad|warn|focus-ring|shadow|font|dur-|ease-|selection|chrome|grouped-bg)\b" src/App.css
  ```
- **SVG icon library or emoji.** No `lucide-react`, `heroicons`,
  `@phosphor-icons`, Font Awesome SVG, or emoji in UI. Every glyph
  is NF via `<Glyph>`.
- **Internal identifier on a primary surface.** DB keys, slugs,
  UUIDs, internal paths — must be behind `DevBadge` or removed.
- **Destructive action without inline consequence.** Verb-and-count
  label, one-line hint next to the button, friction matching blast
  radius. Confirmation copy repeats the verb.
- **Status spray.** Same state fires in more than one surface
  (toast + banner, or running-op strip + toast while running).
- **Credential rendered or logged in full.** Always truncated.
- **`window.confirm / alert / prompt`** anywhere.
- **Modal missing ARIA.** `role="dialog"`, `aria-modal`,
  `aria-labelledby`, Escape handler, focus trap.

### WARN

- Font size not on the `--fs-*` scale.
- Spacing not on the `--sp-*` scale.
- Box shadow on list rows or chrome (elevation reserved for
  popovers, dropdowns, modals).
- Animation on layout properties (`width`, `height`, `top`, `left`).
- New color token missing a dark-mode value.
- New animation missing `prefers-reduced-motion` wrap.
- New low-contrast border missing a `prefers-contrast: more`
  variant.
- Color used as the sole state indicator (must pair with text/glyph).

### Needs a screenshot

- Traffic lights aligned with chrome content.
- One primary action (one `solid` button) visible per view.
- Selected object unmistakable in <1 second.
- Dark-mode parity at a glance.
- First list row not clipped by sticky filter bars.

If any "needs a screenshot" check is load-bearing for this review
and none is provided, stop and ask.

## Step 5 · Report

One table:

| File | Line | Rule | Severity | What to do |
|---|---|---|---|---|

If empty: **"Clean."** If anything BLOCK: **"BLOCK — do not merge."**

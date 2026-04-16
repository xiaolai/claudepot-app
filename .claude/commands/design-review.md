---
description: Review UI changes against the Claudepot design system, accessibility standards, and component conventions.
---

# Design Review

You are reviewing UI changes in a Tauri 2 + React + plain CSS project.

## Step 1: Identify changed UI files

Find modified UI files using both uncommitted and staged changes:

```bash
git diff --name-only -- 'src/**/*.tsx' 'src/**/*.css' 'src/**/*.ts'
git diff --cached --name-only -- 'src/**/*.tsx' 'src/**/*.css' 'src/**/*.ts'
```

Combine and deduplicate the results. If no UI files changed, report
"No UI files modified" and stop.

## Step 2: Read the rules

Read these rules files to understand the standards:
- `.claude/rules/ui-design-system.md`
- `.claude/rules/react-components.md`
- `.claude/rules/accessibility.md`

## Step 3: Read the diffs

Run `git diff HEAD -- <file>` for each changed UI file. Read the full diff.

## Step 4: Check each category

For each changed file, check:

### Design system compliance
- [ ] Colors: all via `var(--token)`, no raw hex/rgb
- [ ] Spacing: values on the 4px grid
- [ ] Typography: font-size from the HIG text styles table (10/11/13/15px)
- [ ] Border radius: matches element type (6/8/10/12/999)
- [ ] Transitions: 0.12s ease, no layout property animation
- [ ] Dark mode: new tokens have both light and dark variants
- [ ] Context menus: new interactive objects have `onContextMenu` handler
- [ ] Keyboard shortcuts: new actions are wired to standard shortcuts

### Component conventions
- [ ] One component per file, under 120 lines
- [ ] No `window.confirm/alert/prompt`
- [ ] Modals: role="dialog", aria-modal, aria-labelledby, Escape handler
- [ ] Buttons: title tooltip, disabled + visible reason
- [ ] State across await: functional updater `setState(prev => ...)`
- [ ] Data fetching through `src/api.ts`, not direct `invoke()`

### Accessibility
- [ ] Keyboard reachable (no click-only interactions)
- [ ] Focus-visible styling present
- [ ] Color not sole indicator of state
- [ ] ARIA attributes on dynamic UI (modals, alerts, live regions)
- [ ] Semantic HTML elements used correctly
- [ ] `prefers-reduced-motion` media query wraps all new animations/transitions
- [ ] `prefers-contrast: more` variant for any new border/separator tokens
- [ ] `prefers-reduced-transparency` fallback for any new translucent surfaces

### Test coverage
- [ ] New interactive behavior has a corresponding test
- [ ] Tests assert on visible text / ARIA roles, not CSS classes

## Step 5: Report

Output a table:

| File | Category | Issue | Line | Severity |
|------|----------|-------|------|----------|

Severity levels:
- **BLOCK**: Must fix before commit (a11y violation, raw hex, window.confirm)
- **WARN**: Should fix (missing tooltip, no test, convention deviation)
- **NOTE**: Consider improving (naming, structure)

If no issues found, report "All clear — changes comply with design system."

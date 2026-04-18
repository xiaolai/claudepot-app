---
description: Scaffold a new React component. Requires explicit kind or a nearest-existing-component match. No empty divs, no speculative scaffolding.
---

# Scaffold Component

Create a new React component grounded in a specific kind from
`design-patterns.md`. The scaffold emits **only what the kind requires**
— no TODO placeholders, no speculative props, no "fill it in later"
CSS. Subtraction from a full recipe produces vestigial code; we avoid
that entirely.

## Inputs

`$ARGUMENTS` must contain:

1. **Component name** — PascalCase (`ProjectDriftBanner`, `AccountRow`,
   `KeychainStatusBadge`).
2. **Kind** — one of the kinds in the table below, OR
3. **Nearest existing component** — if no kind fits, name an existing
   component in `src/components/` or `src/sections/` that is the
   closest structural match.

If neither a kind nor a nearest-component is given, stop and ask. Do
not pick a default — there is no default. An unspecified scaffold is
the exact failure mode we're replacing.

## Kinds

Every kind below maps to a recipe in `.claude/rules/design-patterns.md`
and, where applicable, a live component in `src/`.

| Kind | Recipe anchor | Live reference |
|---|---|---|
| `list-row-selectable` | "Selectable list row (listbox option)" | `src/sections/projects/ProjectsList.tsx`, `src/components/SidebarAccountItem.tsx` |
| `list-row-action` | "Action-button row" | `src/components/SectionRail.tsx` |
| `detail-grid` | "Detail grid" | `src/components/AccountDetail.tsx` |
| `banner` | "Banner (persistent state)" | `src/components/PendingJournalsBanner.tsx` |
| `toolbar` | "Buttons" section + toolbar layout | `src/components/AccountActions.tsx` |
| `filter-bar` | "Segmented / filter bar" | `src/sections/projects/ProjectsList.tsx` (top) |
| `segmented-control` | "Segmented / filter bar" | `src/components/SegmentedControl.tsx` |
| `modal` | "Modal" | `src/components/ConfirmDialog.tsx`, `src/components/AddAccountModal.tsx` |
| `confirm-destructive` | "Destructive button with inline consequence" + "Modal" | `src/components/ConfirmDangerousAction.tsx` |
| `empty-state` | "Empty state" | `src/components/EmptyState.tsx` |
| `status-badge` | "Status badge" | (new recipe — first use) |
| `search-field` | "Search field" | (new recipe — first use) |
| `context-menu` | "Context menu" | `src/components/ContextMenu.tsx` |
| `collapsible-section` | "Collapsible section" | `src/components/CollapsibleSection.tsx` |
| `running-op-entry` | "Running-op strip" | `src/components/RunningOpStrip.tsx` |
| `sidebar-item` | — (study `Sidebar.tsx` pattern) | `src/components/Sidebar.tsx` |

If the kind you need isn't here, the work is novel. Stop, add a recipe
to `design-patterns.md` first, then update this table and scaffold.

## Step 1 · Validate

- PascalCase the name. Reject if it collides with a file in
  `src/components/` or `src/sections/`.
- Reject reserved React identifiers (`Fragment`, `Suspense`, etc.).
- Resolve the kind:
  - If user named a kind, use it.
  - If user named a nearest-component, look up its kind via `grep`/Read
    and use that. If the nearest component spans multiple kinds, stop
    and ask which.

## Step 2 · Load the design system

Read in this order (same as design-review):

1. `.claude/rules/design-principles.md`
2. `.claude/rules/design-references.md`
3. `.claude/rules/feedback-ladder.md`
4. `.claude/rules/design-patterns.md`
5. `.claude/rules/ui-design-system.md`
6. `.claude/rules/accessibility.md`
7. `.claude/rules/react-components.md`

## Step 3 · Read the live reference

Open the live component named in the Kinds table and copy its **actual
current structure**. This is the anti-drift rule: the scaffold mirrors
what exists, not what documentation remembers.

If the kind has no live reference (e.g., `status-badge` first use),
use the recipe in `design-patterns.md` literally.

## Step 4 · Emit the component

Generate `src/components/{Name}.tsx` (or `src/sections/…` if the
component is a section).

**Hard rules:**

- No placeholder `{/* TODO */}`. If a prop is required, it appears in
  the interface; if unknown, ask the user before scaffolding.
- No empty divs as the root.
- No speculative CSS classes. If a new class is introduced, it goes
  through Step 6 below.
- Props types are concrete. Never `any`. Never `Partial<unknown>`.
- For kinds with feedback-surface implications (`banner`,
  `running-op-entry`, `modal`, `confirm-destructive`), include a
  one-line comment above the component citing which row in
  `feedback-ladder.md` this surface is for.

Example output for kind `banner`:

```tsx
// Feedback ladder: persistent global state with action (banner row).
import { AlertTriangle, Wrench } from "lucide-react";

interface {Name}Props {
  // Concrete fields — no placeholders. Ask if unknown.
  label: string;
  hint: string;
  tone: "warn" | "bad";
  onAction: () => void;
  actionLabel: string;
}

export function {Name}({
  label,
  hint,
  tone,
  onAction,
  actionLabel,
}: {Name}Props) {
  const Icon = tone === "bad" ? AlertTriangle : Wrench;
  return (
    <div className={`banner banner-${tone}`} role="alert">
      <Icon strokeWidth={2} />
      <div className="banner-body">
        <strong>{label}</strong>
        <span className="banner-hint">{hint}</span>
      </div>
      <button className="btn" onClick={onAction}>
        {actionLabel}
      </button>
    </div>
  );
}
```

Each kind has a concrete analogous emission — copy the recipe, fill
the interface from actual requirements, do not leave holes.

## Step 5 · Emit the test file

`src/components/{Name}.test.tsx`. Require at least one **behavior**
test — "renders without crashing" is rejected.

```tsx
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { {Name} } from "./{Name}";

describe("{Name}", () => {
  it("calls onAction when the action button is clicked", async () => {
    const onAction = vi.fn();
    render(
      <{Name}
        label="…"
        hint="…"
        tone="warn"
        onAction={onAction}
        actionLabel="Fix"
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /fix/i }));
    expect(onAction).toHaveBeenCalledOnce();
  });
});
```

Tests assert on visible text / ARIA roles, never CSS classes or
internal state (per `react-components.md`).

## Step 6 · CSS — reuse before invent

Before writing new CSS:

1. Search `src/App.css` for the class used by the recipe (e.g.,
   `.banner`, `.list-row`, `.detail-grid`). If it exists, **use it
   directly**. Do not duplicate.
2. If the recipe class does not exist yet in `App.css` (first use of
   a new kind), append it to `App.css` using the exact CSS from
   `design-patterns.md`. No invented values.
3. If the component genuinely introduces a new visual pattern, stop
   and add a recipe to `design-patterns.md` first — then come back
   and scaffold.

Section comment format in `App.css`:

```css
/* ---------- {kebab-name} ---------- */
```

## Step 7 · Report

Print:

- Files created, with paths.
- The kind used and the recipe anchor in `design-patterns.md`.
- The live reference component, if any.
- Whether `App.css` was modified (and which class was added).
- Reminders:
  1. Wire the component into its parent.
  2. Add an `onContextMenu` handler if the component is interactive
     (principle §4 in `design-principles.md`).
  3. Run `pnpm tsc --noEmit` and `pnpm test` to verify the scaffold
     is clean.
  4. Run the component through `/design-review` — the three-pass
     review catches issues the scaffold itself cannot.

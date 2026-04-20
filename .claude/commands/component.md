---
description: Scaffold a new React component grounded in the paper-mono primitives. Requires explicit kind or a nearest-existing-component match. No empty divs, no speculative scaffolding.
---

# Scaffold Component

Create a new React component grounded in a specific kind from
`design-patterns.md`. The scaffold emits **only what the kind
requires** — no TODO placeholders, no speculative props, no
"fill it in later" CSS. Subtraction from a full recipe produces
vestigial code; we avoid that entirely.

## Inputs

`$ARGUMENTS` must contain:

1. **Component name** — PascalCase (`ProjectDriftBanner`,
   `AccountRow`, `KeychainStatusBadge`).
2. **Kind** — one of the kinds in the table below, OR
3. **Nearest existing component** — if no kind fits, name an existing
   component in `src/components/`, `src/sections/`, or `src/shell/`
   that is the closest structural match.

If neither a kind nor a nearest-component is given, stop and ask. Do
not pick a default — there is no default. An unspecified scaffold is
the exact failure mode we're replacing.

## Kinds

Every kind below maps to a recipe in `.claude/rules/design-patterns.md`
and, where applicable, a live paper-mono component in `src/`.

| Kind | Recipe anchor (design-patterns.md) | Live reference |
|---|---|---|
| `list-row-selectable` | "Selectable list row" | `src/sections/projects/ProjectsTable.tsx` |
| `sidebar-item` | "Window shell" + `SidebarItem` primitive | `src/components/primitives/SidebarItem.tsx`, `src/shell/AppSidebar.tsx` |
| `card` | (compose from primitives) | `src/sections/accounts/AccountCard.tsx` |
| `detail-grid` | "Detail grid" | `src/sections/SettingsSection.tsx` (`Kv` rows) |
| `banner` | "Banner (persistent state)" | `src/components/PendingJournalsBanner.tsx`, `src/sections/accounts/AnomalyBanner.tsx` |
| `toolbar` | "Window shell" actions row | `src/shell/ScreenHeader.tsx` (actions slot) |
| `filter-bar` | "Segmented / filter bar" | `src/sections/AccountsSection.tsx` (filter row), `src/sections/ProjectsSection.tsx` |
| `segmented-control` | "Segmented / filter bar" | `src/sections/SettingsSection.tsx` (Appearance theme picker) |
| `modal` | "Modal" | `src/components/primitives/Modal.tsx`, `src/sections/accounts/AddAccountModal.tsx` |
| `confirm-destructive` | "Destructive button with inline consequence" + "Modal" | `src/components/ConfirmDangerousAction.tsx` |
| `empty-state` | "Empty state" | `src/sections/SessionsSection.tsx` |
| `status-tag` | "Status tag / badge" | `src/components/primitives/Tag.tsx` |
| `search-field` | (Input primitive with `glyph={NF.search}`) | `src/sections/AccountsSection.tsx` filter input |
| `context-menu` | "Context menu" | `src/components/ContextMenu.tsx` |
| `collapsible-section` | (legacy, optional) | `src/components/CollapsibleSection.tsx` |
| `running-op-entry` | "Running-op strip" | `src/components/RunningOpStrip.tsx` |
| `usage-row` | (composed from primitives) | `src/sections/accounts/UsageBlock.tsx` |
| `action-card` | (composed from primitives) | `src/sections/accounts/ActionCard.tsx` |
| `screen-header` | (use the primitive as-is) | `src/shell/ScreenHeader.tsx` |

If the kind you need isn't here, the work is novel. Stop, add a
recipe to `design-patterns.md` first, then update this table and
scaffold.

## Step 1 · Validate

- PascalCase the name. Reject if it collides with an existing file
  under `src/components/`, `src/components/primitives/`, `src/shell/`,
  or `src/sections/`.
- Reject reserved React identifiers (`Fragment`, `Suspense`, etc.).
- Resolve the kind:
  - If user named a kind, use it.
  - If user named a nearest-component, look up its kind via Read /
    Grep and use that. If the nearest component spans multiple kinds,
    stop and ask which.
- Resolve the destination:
  - Generic primitive (no domain knowledge): `src/components/primitives/`
  - Domain widget reused across screens: `src/components/`
  - Single-section concern: `src/sections/<section>/`

## Step 2 · Load the design system

Read in this order (same as design-review):

1. `.claude/rules/design-principles.md` — why
2. `.claude/rules/design-patterns.md` — composition recipes and
   feedback ladder
3. `.claude/rules/ui-design-system.md` — tokens and defaults
4. `.claude/rules/no-raw-values.md` — hard ban on raw
   hex / rgb / oklch / numeric fontSize / numeric padding / numeric
   dimension in paper-mono code. Every CSS value comes from a token
   in `src/styles/tokens.css`.
5. `.claude/rules/accessibility.md` — a11y floor
6. `.claude/rules/react-components.md` — file-shape conventions

Legacy native-macOS rules sit under `.claude/rules/_legacy/` for
archaeology only — do **not** read them when scaffolding new work.

## Step 3 · Read the live reference

Open the live component named in the Kinds table and copy its
**actual current structure**. This is the anti-drift rule: the
scaffold mirrors what exists, not what documentation remembers.

If the kind has no live reference, follow the recipe in
`design-patterns.md` literally and reach for primitives in
`src/components/primitives/` rather than rolling raw HTML.

## Step 4 · Emit the component

Generate the file in the directory chosen in Step 1 — e.g.,
`src/sections/accounts/{Name}.tsx`.

**Hard rules:**

- No placeholder `{/* TODO */}`. If a prop is required, it appears in
  the interface; if its value is unknown, ask the user before
  scaffolding.
- No empty `<div>` as the root.
- No speculative CSS classes or invented inline color/size values.
  Use tokens (`var(--accent)`, `var(--fs-sm)`, `var(--r-2)`) — see
  `ui-design-system.md`.
- Props types are concrete. Never `any`. Never `Partial<unknown>`.
- Icons via `Glyph` from `src/components/primitives/Glyph.tsx`,
  reading codepoints from `NF` in `src/icons.ts`. Never lucide,
  heroicons, emoji, or raw SVGs.
- Buttons via `Button` / `IconButton` primitives. Modals via the
  `Modal` trio. Tags via `Tag`.
- For kinds with feedback-surface implications (`banner`,
  `running-op-entry`, `modal`, `confirm-destructive`), include a
  one-line comment above the component citing which row in the
  feedback ladder (in `design-patterns.md`) this surface satisfies.

Example output for kind `banner`:

```tsx
import { Button } from "../components/primitives/Button";
import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";

interface {Name}Props {
  // Concrete fields — no placeholders. Ask if unknown.
  label: string;
  hint: string;
  tone: "warn" | "danger";
  onAction: () => void;
  actionLabel: string;
}

// Feedback ladder: persistent global state with an action.
export function {Name}({
  label,
  hint,
  tone,
  onAction,
  actionLabel,
}: {Name}Props) {
  return (
    <div
      role="alert"
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "10px 16px",
        borderRadius: "var(--r-2)",
        background:
          tone === "danger"
            ? "color-mix(in oklch, var(--danger) 12%, transparent)"
            : "color-mix(in oklch, var(--warn) 12%, transparent)",
        border: `1px solid ${tone === "danger" ? "var(--danger)" : "var(--warn)"}`,
      }}
    >
      <Glyph
        g={NF.warn}
        color={tone === "danger" ? "var(--danger)" : "var(--warn)"}
      />
      <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 2 }}>
        <strong style={{ fontSize: "var(--fs-sm)" }}>{label}</strong>
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)" }}>
          {hint}
        </span>
      </div>
      <Button variant="subtle" onClick={onAction}>
        {actionLabel}
      </Button>
    </div>
  );
}
```

Each kind has a concrete analogous emission — copy the recipe, fill
the interface from actual requirements, do not leave holes.

## Step 5 · Emit the test file

`{Path}/{Name}.test.tsx`. Require at least one **behavior** test —
"renders without crashing" is rejected.

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
        label="Pending journals"
        hint="Two abandoned rename ops need attention."
        tone="warn"
        onAction={onAction}
        actionLabel="Open"
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /open/i }));
    expect(onAction).toHaveBeenCalledOnce();
  });
});
```

Tests assert on visible text / ARIA roles, never CSS classes or
internal state (per `react-components.md`).

## Step 6 · Tokens — reuse before invent

Before writing new tokens or styles:

1. Check `src/styles/tokens.css` for an existing token covering the
   role you need. The 12-step palette + spacing + type scale + radii
   should cover almost every case.
2. If a needed value isn't there, stop and add it to `tokens.css`
   with a justification comment, then come back and use it. Both
   light and dark themes must be defined; `prefers-contrast: more`
   if relevant.
3. If the component genuinely introduces a new visual pattern, stop
   and add a recipe to `design-patterns.md` first — then come back
   and scaffold.

Do not write to `src/App.css` for new components. That file holds
legacy class-based styles for unported components only and is
gradually shrinking. Inline styles + tokens are the paper-mono norm.

## Step 7 · Report

Print:

- Files created, with paths.
- The kind used and the recipe anchor in `design-patterns.md`.
- The live reference component, if any.
- Whether `tokens.css` was modified (and which token was added,
  with both light and dark values).
- Reminders:
  1. Wire the component into its parent.
  2. Add an `onContextMenu` handler if the component is interactive
     and lives in a primary data view (principle §1 + a11y §
     "Context menus").
  3. Run `pnpm tsc --noEmit` and `pnpm test` to verify the scaffold
     is clean.
  4. Run the component through `/design-review` — the three-pass
     review catches issues the scaffold itself cannot.

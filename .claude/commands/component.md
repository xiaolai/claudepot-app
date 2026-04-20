---
description: Scaffold a new React component grounded in the paper-mono primitives. Requires explicit kind or a nearest-existing-component match. No empty divs, no speculative scaffolding.
---

# Scaffold Component

Create a new React component grounded in one of the kinds below.
The scaffold emits **only what the kind requires** — no TODO
placeholders, no speculative props, no "fill it in later" CSS.
Subtraction from a full recipe produces vestigial code; we avoid
that entirely.

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

Every kind below points to a live paper-mono component in `src/`
that the scaffold must mirror.

| Kind | Live reference |
|---|---|
| `list-row-selectable` | `src/sections/projects/ProjectsTable.tsx` |
| `sidebar-item` | `src/components/primitives/SidebarItem.tsx`, `src/shell/AppSidebar.tsx` |
| `card` | `src/sections/accounts/AccountCard.tsx` |
| `detail-grid` | `src/sections/SettingsSection.tsx` (`Kv` rows) |
| `banner` | `src/components/PendingJournalsBanner.tsx`, `src/sections/accounts/AnomalyBanner.tsx` |
| `toolbar` | `src/shell/ScreenHeader.tsx` (actions slot) |
| `filter-bar` | `src/sections/AccountsSection.tsx`, `src/sections/ProjectsSection.tsx` |
| `segmented-control` | `src/sections/SettingsSection.tsx` (Appearance theme picker) |
| `modal` | `src/components/primitives/Modal.tsx`, `src/sections/accounts/AddAccountModal.tsx` |
| `confirm-destructive` | `src/components/ConfirmDangerousAction.tsx` |
| `empty-state` | `src/sections/SessionsSection.tsx` |
| `status-tag` | `src/components/primitives/Tag.tsx` |
| `search-field` | `src/sections/AccountsSection.tsx` filter input |
| `context-menu` | `src/components/ContextMenu.tsx` |
| `collapsible-section` | `src/components/CollapsibleSection.tsx` |
| `running-op-entry` | `src/components/RunningOpStrip.tsx` |
| `usage-row` | `src/sections/accounts/UsageBlock.tsx` |
| `action-card` | `src/sections/accounts/ActionCard.tsx` |
| `screen-header` | `src/shell/ScreenHeader.tsx` |

If the kind you need isn't here, the work is novel. Stop and
discuss the structure with the user before scaffolding.

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

Read `.claude/rules/design.md` — that's the single source for the
paper-mono register, token discipline, icon policy, accessibility
floor, and non-negotiables. Also read `src/styles/tokens.css` to
know which tokens exist.

## Step 3 · Read the live reference

Open the live component named in the Kinds table and copy its
**actual current structure**. This is the anti-drift rule: the
scaffold mirrors what exists, not what documentation remembers.

Reach for primitives in `src/components/primitives/` (`Button`,
`IconButton`, `Glyph`, `Avatar`, `Tag`, `Modal`, `SidebarItem`,
`SectionLabel`) rather than rolling raw HTML.

## Step 4 · Emit the component

Generate the file in the directory chosen in Step 1 — e.g.,
`src/sections/accounts/{Name}.tsx`.

**Hard rules:**

- No placeholder `{/* TODO */}`. If a prop is required, it appears in
  the interface; if its value is unknown, ask the user before
  scaffolding.
- No empty `<div>` as the root.
- No speculative CSS classes or invented inline color/size values.
  Use tokens from `src/styles/tokens.css`
  (`var(--accent)`, `var(--fs-sm)`, `var(--r-2)`).
- Props types are concrete. Never `any`. Never `Partial<unknown>`.
- Icons via `Glyph` from `src/components/primitives/Glyph.tsx`,
  reading codepoints from `NF` in `src/icons.ts`. Never lucide,
  heroicons, emoji, or raw SVGs.
- Buttons via `Button` / `IconButton` primitives. Modals via the
  `Modal` trio. Tags via `Tag`.
- For kinds with feedback-surface implications (`banner`,
  `running-op-entry`, `modal`, `confirm-destructive`), include a
  one-line comment above the component naming the canonical
  surface this state uses (toast, banner, inline note, running-op
  strip, or modal — see `design.md`'s "One signal per surface").

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
internal state.

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
   and discuss it with the user first.

Do not write to `src/App.css` for new components. That file holds
legacy class-based styles for unported components only and is
gradually shrinking. Inline styles + tokens are the paper-mono norm.

## Step 7 · Report

Print:

- Files created, with paths.
- The kind used and the live reference mirrored.
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

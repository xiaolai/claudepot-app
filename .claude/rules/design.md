---
description: Minimal design and frontend conventions for paper-mono
globs: "src/**/*.{tsx,ts,css}"
---

# Design — paper-mono

## Register

One typeface (JetBrainsMono Nerd Font) for every glyph, text and
icon. Warm OKLCH palette, single terracotta accent. Hairline borders
(1px), small radii (≤10px), flat list rows — elevation is reserved
for popovers and modals.

Light and dark modes share variable names; theme switches by flipping
`data-theme` on `<html>`.

## Tokens

`src/styles/tokens.css` is the **one** place tokens are declared. No
other file opens a `:root { }` block or redeclares `--*` custom
properties. All colors, sizes, spacings, radii, durations, and
z-indexes come from that file. Literals are a review finding.

If a value you need doesn't exist, add a semantic token to
`tokens.css` first (light + dark), then reference it.

## Icons

Nerd Font codepoints only, via `<Glyph g={NF.x} />`
(`src/components/primitives/Glyph.tsx`). No `lucide`, `heroicons`,
Font Awesome SVG, or emoji. New icons are added to the `NF` map in
`src/icons.ts`.

## Primitives

Paper-mono primitives in `src/components/primitives/` — `Button`,
`IconButton`, `Glyph`, `Avatar`, `Tag`, `Modal`, `SidebarItem`,
`SectionLabel`. Reach for these first. Inline styles on primitives
are the norm; class-based CSS in `App.css` is legacy (opt-in via
`.btn` for the pre-paper-mono chrome).

## Non-negotiables

- **One primary action per view** (one `solid` / one `.btn.primary`).
- **Render-if-nonzero**: `0 sessions · 0 MB · …` never ships; filter
  zero-value fields out before joining.
- **No internal identifiers in primary UI** — DB keys, UUIDs, slugs
  belong behind a disclosure or `<DevBadge>`, not on a detail row.
- **Disabled buttons state a reason inline** — next to the button,
  not in a tooltip.
- **One signal per surface** — a given event fires exactly one of:
  toast, banner, inline note, `RunningOpStrip`, modal. No status
  spray.
- **Credentials never rendered** — tokens/secrets are always
  truncated (`sk-ant-oat01-Abc…xyz`). Never log, never toast.

## Accessibility floor

- Every interactive element is keyboard-reachable and shows a
  visible `:focus-visible` ring (paper-mono primitives do this).
- Color never carries meaning alone — pair with text, glyph, or
  position.
- Modals: `role="dialog"`, `aria-modal`, `aria-labelledby`, Esc to
  close, focus trap (`useFocusTrap`).
- Listboxes: `<ul role="listbox">` + `<li role="option" tabIndex={0}
  aria-selected>`.
- Respect `prefers-reduced-motion`, `prefers-contrast: more`, and
  `prefers-reduced-transparency`.
- Never use `window.confirm / alert / prompt` — unreliable in Tauri
  webviews. Use the `Modal` + `ConfirmDialog` primitives.

## Shortcuts

⌘K palette, ⌘R refresh, ⌘N add, ⌘, settings, ⌘1..⌘4 section, ⌘F
focus search, Esc close modal. Never fire while a modal is open or
an input is focused.

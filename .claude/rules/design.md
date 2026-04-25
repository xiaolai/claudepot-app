---
description: Minimal design and frontend conventions for paper-mono
globs: "src/**/*.{tsx,ts,css}"
---

# Design — paper-mono

## Register

One typeface (JetBrainsMono Nerd Font) for text. Icons are Lucide
SVG. Warm OKLCH palette, single terracotta accent. Hairline borders
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

## Stylesheet layout

- `src/styles/tokens.css` — every global custom property; the only
  file allowed to open `:root { }`.
- `src/styles/components/*.css` — sharded component styles
  (`base`, `rail`, `sidebar`, `accounts`, `palette`, `modals`,
  `banners`, `settings`, `projects`, …). Each shard stays under the
  350-LOC loc-guardian limit and groups by surface, not by feature
  flag or PR.
- `src/App.css` — import index only. It carries the file-level
  documentation (the "do not declare tokens" rule, the lint
  invariants) and `@import`s every shard in cascade order.
  **Never add a rule directly to App.css.** Pick the matching shard
  (or add a new shard) and let the index pull it in.

## Icons

Lucide SVG icons only, via `<Glyph g={NF.x} />` from
`src/components/primitives/Glyph.tsx`. The call shape is kept from
the older Nerd Font pipeline — `NF.*` now maps each semantic name
to a `lucide-react` component reference. No Heroicons, Font Awesome,
emoji, or hand-authored SVGs. New icons are added to the `NF` map
in `src/icons.ts` by picking a Lucide import.

`Glyph` pins `strokeWidth={1.75}` and centers the SVG in a square
inline-flex box so icons track the surrounding font-size. For the
tray/menubar (AppKit NSImage, not React), PNGs are pre-rasterized
from the matching Lucide SVG in `src-tauri/icons/menu/`.

## Primitives

Paper-mono primitives in `src/components/primitives/` — `Button`,
`IconButton`, `Glyph`, `Avatar`, `Tag`, `Modal`, `SidebarItem`,
`SectionLabel`. Reach for these first. Inline styles on primitives
are the norm; class-based CSS in `App.css` is legacy (opt-in via
`.btn` for the pre-paper-mono chrome).

## Cards vs. tables

Pick the container by the primary verb, not the row count.

- **Cards** — the user's job is _browse + act_. Each row carries
  multiple primary actions (switch, verify, remove) and shows an
  identity (avatar, name, email, status at a glance). The user
  rarely scans past the first screenful. Example: Accounts.
- **Tables** — the user's job is _scan + drill_. Rows have one
  primary verb (click to open). Secondary actions hide behind a
  kebab or context menu. The user expects dense scanning with
  sortable columns. Examples: Projects, Sessions, Keys.

A section with "multiple in-row verbs AND likely N > 20" is a
hybrid — render as a table and lift the verbs into a row kebab
(`NF.ellipsis`). Don't add a density toggle: one container per
section keeps the design pass cheap and the a11y story simple.

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

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

**Brand-mark exception.** `lucide-react` v1+ removed brand icons
(GitHub, GitLab, Twitter/X, etc.) for trademark reasons. When a
trademarked third-party brand mark is required by the design (e.g.
the GitHub logo next to a `github.com/…` link in About), an inline
SVG of that mark is allowed *only if* (a) it sits in secondary
chrome (About, footer, "powered by") — never in primary navigation
or a primary action; (b) it uses `currentColor` so it inherits the
theme; (c) it's named like `<BrandnameMark>` and lives next to its
single call site, with a comment explaining why Lucide can't supply
it. Adding a custom SVG without verifying that the underlying need
is actually a trademarked third-party mark is a review finding.

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

## Empty and loading states

**One `<EmptyState>`**, in `components/primitives/`. Five components
were called `EmptyState`, shared no code, and disagreed about what one
is — some had a title and body, some a CTA, some were a bare `<p>`.
Empty states are the first thing a new user sees in most sections, so
first-run quality varied by whichever section they landed on.

`action` is **required**, and `action={null}` is the way to say there
is no next step. An empty state without one is usually a missing
feature rather than a missing string, so the decision belongs where a
reviewer can see it.

Two variants, matching the two real shapes: `block` (a section or pane
with nothing in it) and `inline` (a note inside an otherwise-populated
pane). A third variant per call site is how the original five happened
— `align="start"` covers left-aligned prose.

Branching logic stays with the caller. `KnowView`'s empty state works
out *which* filter emptied the view and offers the matching clear
action; that reasoning does not belong in a generic component's props.
The primitive owns the shape, the caller owns the reasoning.

**Any surface with a known shape gets a skeleton of that shape.** Only
genuinely unknown-shape content gets text. A one-line `Loading…` in a
laid-out pane makes the layout jump when data lands —
`ProjectEnvPanel` carries a comment recording that a re-render
"briefly collapsed the panel to Loading…, displacing the Sessions
section below by ~108px". Use `SkeletonList` (with header) or
`SkeletonRows` (surface has its own header); both carry the
`role="status"` and screen-reader label that hand-rolled
`.skeleton-container` markup silently omitted.

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
  `prefers-reduced-transparency`. All three are implemented in
  `tokens.css`; `prefers-contrast: more` was the one this list
  committed to and the stylesheets never delivered, until 2026-08-15.
  Its overrides sit **after** the legacy alias block on purpose — a
  media query adds no specificity, so an override written above
  `--focus-ring`'s declaration loses on source order and silently does
  nothing.
- Never use `window.confirm / alert / prompt` — unreliable in Tauri
  webviews. Use the `Modal` + `ConfirmDialog` primitives.

## Shortcuts

⌘K palette, ⌘R refresh, ⌘N add, ⌘, settings, ⌘1..⌘9 section (bound by
position in `src/sections/registry.tsx`), ⌘/ shortcut reference,
⌘⇧L focus the live strip, ⌃⌥⌘B show/hide Boards, Esc close modal.

**Never fire while a modal is open or an input is focused.** The one
predicate for that is `isShortcutContextBlocked()` in
`src/hooks/useGlobalShortcuts.ts` — use it rather than re-deriving the
check. Every hook here used to carry its own weaker copy, which is how
⌘K ended up able to open the palette on top of an open dialog.

**⌃⌥⌘B** (show/hide Boards) uses the same four-modifier shape but is
**gated normally**. The exception below is earned by having no visible
control; Boards has one in Settings → General, so it takes the ordinary
rule. Copying the modifier shape does not copy the exemption.

Sole exception: **⌃⌥⌘L** (toggle developer mode) is ungated on
purpose. It has no visible control anywhere, and its value is being
reachable precisely when the UI is misbehaving — including from a
modal that won't dismiss. Its four-modifier combo makes accidental
firing while typing a non-issue. Any *further* exception needs the
same treatment: written here, not just commented at the call site.

**Bindings live in one table.** `src/lib/shortcutBindings.ts` is the
list; `ShortcutsModal` renders from it and `cargo xtask verify-docs`
asserts every `key` in it is compared against somewhere in `src/`.
This exists because the docs and the code drifted in both directions:

- **⌘F was documented for years with no handler.** That is the case
  this rule used to record — and it has since inverted. A section
  *did* wire ⌘F (ConfigSection, focus the content search), so the
  claim "there is no ⌘F" became false and stayed in this file
  regardless. Documenting a dead shortcut and failing to document a
  live one are the same defect; only the second is hard to notice.
- **⌘\ shipped undocumented**, surfaced only in the sidebar toggle's
  tooltip.

**⌘-numbers are positions in the full registry, not in the visible
list.** A section's number is a property of that section, so toggling
an optional section never renumbers its neighbours. The cost is that
⌘9 is inert while Boards is off, and Settings — tenth — has no number
and keeps ⌘,. Before this, enabling Boards silently moved Settings off
⌘9, which is exactly the muscle-memory break the registry comment
claimed Boards' ninth position avoided.

## Open questions — not yet decided, not yet shipped

Lifted from `dev-docs/archive/macos-native-design-system.md` when that
document was retired (see its header). They are recorded here as
**questions**, not specifications: neither describes shipped
behaviour, and the archived file's specifications were wrong about
this product in every other respect.

**Context menus.** Apple's HIG expects a right-click menu on every
interactive object; the app currently has `.context-menu-item` styling
and the ⌘K palette, but no general context-menu contract. If one is
added, the archived draft's proposed targets are a reasonable starting
list (account card, project row, token badge), and the hard
requirement is keyboard navigability — arrows, Enter, Escape — because
a mouse-only menu is worse than none for the surfaces that already
have keyboard paths.

**Liquid Glass / macOS Tahoe.** Tahoe applies glass to toolbars,
sidebars and popovers automatically, and an explicit
`NSVisualEffectView` *blocks* it. Relevant to this app only if the
window chrome is ever revisited; paper-mono is deliberately flat, so
adopting glass would be a change of register rather than a fix. Noted
because the accessibility behaviour is automatic and useful either
way: Reduce Transparency frosts it, Increase Contrast makes it opaque
with a border, Reduce Motion disables the elastic effects.

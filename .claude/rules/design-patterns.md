# Design Patterns — composition recipes (paper-mono)

These are the composed building blocks for Claudepot UI. Start here
when building a new view: check whether an existing pattern already
solves it. Reach for the recipe, then adapt — do not handwrite styles
from tokens first.

**Load order:**

1. `design-principles.md` — the *why*
2. This file — the *how to compose*
3. `ui-design-system.md` — the *which tokens, current defaults*

If a recipe here fights a principle, fix the recipe. Recipes are
current house style, not scripture.

## Window shell

```
┌───────────────────────────────────────────────────────────────┐
│ ◯ ◯ ◯   ⌂ ~/.claude › accounts          [⌘K Jump to…]  ☾    │  WindowChrome (38px)
├───────────┬───────────────────────────────────────────────────┤
│ SWAP      │                                                   │
│  ┌─cli─┐  │                                                   │
│  ┌deskt┐  │                                                   │
│           │                                                   │
│  Accounts │                 Content pane                      │
│  Projects │                                                   │
│  Sessions │                                                   │
│  Settings │                                                   │
│           │                                                   │
│ ~/.claude │                                                   │
│   …tree…  │                                                   │
│           │                                                   │
│  ● synced │                                                   │
├───────────┴───────────────────────────────────────────────────┤
│ MAIN · 24 PROJECTS · 187 SESSIONS · …      ⚡ claude-sonnet-4-5│  StatusBar (24px)
└───────────────────────────────────────────────────────────────┘
```

| Region | Width | Background |
|---|---|---|
| WindowChrome | full, 38px tall | `var(--bg)` |
| Sidebar | 240px | `var(--bg)` |
| Content | flex:1 | `var(--bg)` |
| StatusBar | full, 24px tall | `var(--bg-sunken)` |

Hairlines (`var(--line)`) separate each region. No shadows on any of
these surfaces.

## Selectable list row

Lists are ARIA listboxes: `<ul role="listbox">` with `<li
role="option" aria-selected tabIndex={0}>`. Rows are `28px` tall by
default, `8px 10px` inner padding, `6px` radius.

- Rest: transparent background, `--fg-muted` text.
- Hover: `--bg-hover`, `--fg`.
- Active (selected): `--bg-active`, `--fg`, weight 600, 2px accent
  left border.
- Focus-visible: 3px `--accent-border` outline, offset -1px.

### Metadata composition

Zero-value fields disappear. Build the meta string by filter-join:

```ts
const meta = [
  sessions > 0 && `${sessions} session${sessions === 1 ? '' : 's'}`,
  size > 0 && formatBytes(size),
  daysAgo != null && `${daysAgo}d ago`,
].filter(Boolean).join(' · ');
```

Never render `0 sessions · …`. Filter zeros first. Order: most
actionable first.

## Detail grid

Right-aligned label column, generous row spacing. No internal
identifiers in the primary grid — put them behind a `DevBadge` or
a CollapsibleSection.

```css
.detail-grid {
  display: grid;
  grid-template-columns: minmax(100px, max-content) 1fr;
  column-gap: var(--s-4);
  row-gap: var(--s-2);
}
.detail-row dt { text-align: right; color: var(--fg-muted); }
.detail-row dd { color: var(--fg); }
```

## Segmented / filter bar

Options carry text labels (or icon + visible label). A tooltip is
not a substitute for a label — readers do not hover-to-discover.

```tsx
<SegmentedControl
  value={filter}
  onChange={setFilter}
  options={[
    { id: 'all',      label: `All · ${counts.all}` },
    { id: 'pinned',   label: 'Pinned' },
    { id: 'claudemd', label: 'CLAUDE.md' },
  ]}
/>
```

Counts appear on the *active* segment only (or always if the count is
load-bearing, e.g. the Accounts screen's "needs attention" badge).

## Banner (persistent state)

Used when a state requires attention and persists longer than a toast.
Tone via modifier: `banner-warn`, `banner-bad`, `banner-accent`.

```tsx
<div className="banner banner-warn" role="alert">
  <Glyph g={NF.warn} />
  <div className="banner-body">
    <strong>{primary}</strong>
    <span className="banner-hint">{consequence}</span>
  </div>
  <Button variant="subtle" onClick={onAction}>{actionLabel}</Button>
</div>
```

- Background: `--accent-soft` / `color-mix(--warn 12%, transparent)`.
- Border: 1px `--accent-border` / `--warn`.
- Radius: `--r-2`.
- Text: `--fg` strong, `--fg-muted` hint.

## Running-op strip

Bottom-of-window strip for background ops with progress. Appears when
ops are running; disappears when empty. Never duplicated by a toast
while the op is active — the strip owns the state.

## Status tag / badge

Small pill via the `Tag` primitive. Tones:
`neutral | accent | ok | warn | danger | ghost`. Paired with text,
never color alone.

```tsx
<Tag tone="warn" glyph={NF.warn}>stale</Tag>
```

Height 18 px, padding `0 6px`, uppercase `--ls-wide`, `--fs-xs`.

## Modal

Centered via flex, `--scrim` background, 2px blur backdrop. Width
caller-controlled; default `480px`. Closes on scrim click or Esc.
`ModalHeader` uses `.mono-cap` title + close `IconButton`.
`ModalFooter` is right-aligned with 8px gap between buttons.

- Radius `--r-3`, border `1px --line-strong`.
- Shadow: `0 16px 48px rgba(0,0,0,0.18), 0 2px 8px rgba(0,0,0,0.08)`.

Only for blocking flows (destructive confirmations, add account,
rename with side-effects). Never for completion messages — use a
toast.

## Destructive button with inline consequence

The button label carries the verb and count. Inline hint below
states the consequence. No naked "Delete…" with no context.

```tsx
<div className="destructive-action">
  <Button variant="solid" className="danger" disabled={count === 0}>
    Clean {count} project{count === 1 ? '' : 's'}
  </Button>
  {count > 0 && (
    <p className="destructive-hint">
      Removes {formatBytes(bytes)} of session data. Cannot be undone.
    </p>
  )}
</div>
```

## Empty state

Quiet, concrete, one action at most.

```tsx
<div className="empty-state" role="status">
  <Glyph g={NF.folder} size={28} color="var(--fg-ghost)" />
  <p>No projects yet.</p>
  <p className="mono-faint">Projects appear after you run <code>claude</code> in a directory.</p>
</div>
```

## Context menu

Every interactive object in the primary data views (account cards,
project rows, token badges) must have an `onContextMenu` handler
providing relevant actions. Use the shared `ContextMenu` component
(existing in `src/components/`). Menu items must match the actions
available via buttons in the same view — context menus are a
shortcut, not a hidden feature surface.

## Command palette

Opens on `⌘K` / `Ctrl+K`. 580px modal, 14vh from top, scrim + 3px
blur backdrop. Search input at top, results list, bottom hint bar
with `↑↓ ↵ ⎋` Kbd hints. Indexes four kinds of items:

- **Go-to screens** — Accounts, Projects, Sessions, Settings
- **Switch CLI / Desktop to…** — one per registered account × 2 targets
- **Projects** — every registered project
- **Sessions** — recent sessions (once backend lands)

Fuzzy filter over label + sub + kind. Results capped at 30; 20 shown
when query empty. Keyboard: `↑/↓` move, `↵` activate, `Esc` close.

## Feedback ladder

Every state change has exactly one canonical surface. Pick from this
table before adding a new toast/banner/strip.

| Scenario | Surface |
|---|---|
| Row selection, filter change, sort | **No feedback** — state is self-evident |
| "Copied to clipboard" | **Toast (2s)** |
| Inline rename saved | **Inline note** next to the field, fades after 3s |
| Operation completed (sync done, clean done) | **Toast (4s)** |
| Operation failed, recoverable | **Inline error on the affected surface** |
| Operation failed, unrecoverable | **Persistent toast** with dismiss + copy-details |
| Long background op with progress | **`RunningOpStrip`** |
| Persistent state requiring attention (pending journals, keychain locked) | **Banner** at top of content pane |
| Destructive action about to happen | **Modal** with consequence copy |
| New content appeared | **Row appears; no extra feedback** |
| Unrecoverable app-level failure | **Banner (bad tone)** + disable affected controls |

**Rules:**

1. One surface per state. If you write both a toast and a banner for
   the same event, delete the weaker one (usually the toast).
2. Running ops own their status. A long op shows in the
   `RunningOpStrip` only. No "renaming…" toast while the strip is
   active.
3. Errors escalate, they don't duplicate.
4. Banners are for state, not events.
5. Modals are for blocking the user — not for reporting completion.
6. No toast for things the user just did deliberately.

## Named Claudepot anti-patterns

These are failures shipped in prior iterations and must not recur.
All are BLOCK-level in review.

1. **SVG icon library mixed with NF glyphs.** Principle 10. Single
   imported lucide icon breaks the aesthetic.
2. **Raw hex / rgb / oklch** in component CSS. Use variables.
3. **Zero-state metadata** — rendering `0 sessions · 33.8 MB · 21d`.
4. **Internal identifiers** in primary detail grids.
5. **Disabled buttons without inline reason.**
6. **Scroll container clipping the first row** — sticky filter bar +
   scrollable list requires the list's top padding to equal the
   sticky bar's height.
7. **Status spray** — same event firing toast + banner + strip.
8. **Toasting a persistent state** — locked keychain as a 4-second
   toast.
9. **Silent long op** — background op with no `RunningOpStrip` entry.
10. **Large radii or deep shadows on list rows** — elevation is
    reserved for popovers, dropdowns, and modals.

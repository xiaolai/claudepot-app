---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# Design Patterns — Composition Recipes

These are the composed building blocks for Claudepot UI. Start here
when building a new view: check whether an existing pattern already
solves it. Reach for the recipe, then adapt — do not handwrite CSS
first.

**Load order:**

1. `design-principles.md` — the *why*
2. `design-references.md` — the *what to imitate*
3. `feedback-ladder.md` — the *which surface for this state*
4. This file — the *how to compose*
5. `ui-design-system.md` — the *which tokens, current defaults*

If a recipe here fights a principle, fix the recipe. Recipes are
current house style, not scripture.

## Hierarchy budget

Every screen has a **hierarchy budget of one primary surface**.

- **One** primary navigation surface (nav rail). The list pane inside
  the content area is not a second navigation surface — it is a list
  *within* the selected section.
- **One** primary action per view (rename, add, clean).
- **One** accent-colored element visible at rest.

"Two navigation surfaces" means a nav rail *plus a second rail*, or a
nav rail *plus a fixed sidebar that routes to a third view*. A list
pane with selectable rows is normal; it does not count as a second
nav.

## Window shell

```
┌───────────────────────────────────────────────────┐
│ ● ● ●    (traffic lights float on nav rail)       │
│┌──┬──────────────┬──────────────────────────────┐ │
││  │              │                              │ │
││R │  LIST PANE   │        DETAIL PANE           │ │
││48│  240–320     │        flex: 1               │ │
││  │              │                              │ │
│└──┴──────────────┴──────────────────────────────┘ │
└───────────────────────────────────────────────────┘
```

- **Nav rail** (48 px): vibrancy, runs y=0 to bottom. First icon at
  ~52 px to clear traffic lights.
- **List pane** (240–320 px): opaque `var(--bg)`.
- **Detail pane**: flex: 1, opaque `var(--bg)`.
- **Separators**: 0.5 px `var(--border)`, vertical, uninterrupted
  top-to-bottom. No horizontal separator in window chrome.

Tauri config: `titleBarStyle: "Overlay"`, `windowEffects: ["sidebar"]`
on the rail. Do not emulate vibrancy with CSS `backdrop-filter` — use
the OS.

## Selectable list row (listbox option)

Claudepot's project and account lists are ARIA listboxes: `<ul
role="listbox">` with `<li role="option" aria-selected tabIndex={0}>`.
This recipe matches `src/sections/projects/ProjectsList.tsx` and
`src/components/SidebarAccountItem.tsx`.

```tsx
<ul className="list-pane" role="listbox" aria-label="Projects">
  {items.map((item) => (
    <li
      key={item.id}
      role="option"
      aria-selected={item.id === selectedId}
      tabIndex={0}
      className={`list-row${item.id === selectedId ? " active" : ""}`}
      onClick={() => onSelect(item.id)}
      onContextMenu={(e) => onContextMenu(e, item)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect(item.id);
        }
      }}
    >
      <div className="list-row-text">
        <strong className="list-row-name">{item.name}</strong>
        {meta(item) && (
          <span className="list-row-meta">{meta(item)}</span>
        )}
      </div>
    </li>
  ))}
</ul>
```

```css
.list-row {
  display: block;
  padding: 8px 12px;
  border-radius: 6px;
  cursor: default;
}
.list-row:hover { background: var(--hover-bg); }
.list-row.active,
.list-row[aria-selected="true"] {
  background: var(--accent-weak);
}
.list-row:focus-visible {
  outline: 3px solid var(--focus-ring);
  outline-offset: -1px;
}
.list-row-text { display: flex; flex-direction: column; gap: 2px; }
.list-row-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text);
  line-height: 1.3;
}
.list-row-meta {
  font-size: 11px;
  font-weight: 400;
  color: var(--muted);
  line-height: 1.3;
}
```

### Metadata composition

Zero-value fields disappear. Build the meta string by filter-join:

```ts
const meta = [
  sessions > 0 && `${sessions} session${sessions === 1 ? "" : "s"}`,
  size > 0 && formatBytes(size),
  daysAgo != null && `${daysAgo}d ago`,
].filter(Boolean).join(" · ");
```

Never render `0 sessions · 33.8 MB · 21d ago`. Filter zeros first.

Order: **most actionable first**. Size-then-age for projects, because
stale large projects are cleanup candidates. Session counts only when
non-zero.

## Action-button row (non-list)

For lists of *actions* (e.g., a toolbar's stacked buttons), a row is
a `<button>`, not a listbox option:

```tsx
<button
  type="button"
  className="action-row"
  onClick={onClick}
  title={tooltip}
>
  <Icon size={14} />
  <span>{label}</span>
</button>
```

Use this when clicking performs an action (triggers rename, opens
settings). Use the selectable list row recipe when clicking *selects*
an object that drives the detail pane.

## Detail grid

Right-aligned label column, generous row spacing. Matches System
Settings. No internal identifiers in the primary grid (principle §1).

```tsx
<dl className="detail-grid">
  {rows.map(([label, value]) => (
    <div key={label} className="detail-row">
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  ))}
</dl>
```

```css
.detail-grid {
  display: grid;
  grid-template-columns: minmax(100px, max-content) 1fr;
  column-gap: 16px;
  row-gap: 10px;
  margin: 0;
}
.detail-row { display: contents; }
.detail-row dt {
  text-align: right;
  font-size: 13px;
  color: var(--muted);
  font-weight: 400;
}
.detail-row dd {
  margin: 0;
  font-size: 13px;
  color: var(--text);
  display: flex; align-items: center; gap: 8px;
}
.detail-row dd code {
  font-family: var(--mono);
  font-size: 11px;
}
```

**Rules:**
- Only user-meaningful fields. Internal keys (DB slugs, UUIDs, row
  IDs) live behind a disclosure or not at all — see principle §1.
- Zero-state rows drop. Don't render `Memory files: 0`.
- Path-like values use `<code>` + `<CopyButton>`. Nothing else needs
  mono.

## Segmented / filter bar

Segmented controls must be readable cold. Options carry text labels
(or icon + visible label below). Counts appear on the *active*
segment only.

```tsx
<SegmentedControl
  options={[
    { id: "all", label: `All · ${counts.all}` },
    { id: "orphaned", label: "Orphaned" },
    { id: "offline", label: "Offline" },
    { id: "empty", label: "Empty" },
  ]}
  value={filter}
  onChange={setFilter}
/>
```

If space truly forbids text labels, pair each icon with a visible text
label *below* — icon-over-label, both visible at rest:

```tsx
<button className="filter-pill" aria-pressed={active}>
  <LinkIcon size={14} />
  <span className="filter-pill-label">Orphaned</span>
</button>
```

A tooltip is not a substitute for a label. macOS mouse users do not
hover-to-discover.

## Banner (persistent state)

Use when a state requires attention and persists longer than a toast
(see `feedback-ladder.md`). Live examples: `PendingJournalsBanner`
(warn), a keychain-locked banner (warn), backend-unreachable banner
(error).

```tsx
<div className="banner banner-warn" role="alert">
  <AlertTriangle strokeWidth={2} />
  <div className="banner-body">
    <strong>{primary}</strong>
    <span className="banner-hint">{consequence}</span>
  </div>
  <button className="btn" onClick={onAction}>{actionLabel}</button>
</div>
```

```css
.banner {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 16px;
  border-radius: 8px;
  background: var(--surface);
  border: 0.5px solid var(--border);
}
.banner-warn { background: var(--warn-weak); border-color: var(--warn); }
.banner-bad  { background: var(--bad-weak);  border-color: var(--bad);  }
.banner-body { flex: 1; display: flex; flex-direction: column; gap: 2px; }
.banner-body strong { font-size: 13px; font-weight: 600; }
.banner-hint { font-size: 11px; color: var(--muted); }
```

## Running-op strip

Bottom-of-window strip for background ops with progress. Lives in
`src/components/RunningOpStrip.tsx`. Appears only when ops are
running; disappears when empty.

Do not duplicate its signal with toasts while the op is running —
see `feedback-ladder.md` §2.

## Status badge

Small pill for a row's health state (ok / warn / bad) or a count.
Paired with text, never color alone.

```tsx
<span className="status-badge status-badge-warn">
  <AlertTriangle size={10} strokeWidth={2.5} /> stale
</span>
```

```css
.status-badge {
  display: inline-flex; align-items: center; gap: 4px;
  padding: 1px 8px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
  line-height: 1.5;
}
.status-badge-ok   { background: var(--ok-weak);   color: var(--ok);   }
.status-badge-warn { background: var(--warn-weak); color: var(--warn); }
.status-badge-bad  { background: var(--bad-weak);  color: var(--bad);  }
```

## Context menu

Right-click menu on any interactive object. Items mirror visible
actions — the menu is a shortcut, not a hidden feature surface.

Use the shared `ContextMenu` component in `src/components/`. Every
interactive object in `design-principles.md` (accounts, projects, token
badges, list rows) must have one.

## Collapsible section

Used in the detail pane for secondary groups (e.g., advanced fields,
internal identifiers under a "Show details" disclosure). Lives in
`src/components/CollapsibleSection.tsx`.

Use sparingly — each collapse is a place the user has to discover.
If a field is useful, show it; if it isn't, drop it entirely.

## Search field

Inline input with a leading search icon. 28 px height (matches button
height). Escape clears when focused. See also `Cmd+F` shortcut in
`ui-design-system.md`.

```tsx
<div className="search-field">
  <Search size={14} strokeWidth={1.5} />
  <input
    type="search"
    placeholder="Search…"
    value={query}
    onChange={(e) => setQuery(e.target.value)}
  />
</div>
```

## Destructive button with inline consequence

The button label carries the verb and count. Inline hint below states
the consequence. No naked "Delete…" with no context.

```tsx
<div className="destructive-action">
  <button
    className="btn danger"
    onClick={onClick}
    disabled={count === 0}
  >
    Clean {count} project{count === 1 ? "" : "s"}
  </button>
  {count > 0 && (
    <p className="destructive-hint">
      Removes {formatBytes(bytes)} of session data. Cannot be undone.
    </p>
  )}
  {count === 0 && (
    <p className="destructive-hint muted">
      Nothing to clean.
    </p>
  )}
</div>
```

See `design-principles.md` §3 for the underlying rule.

## Modal

440 px wide. Backdrop `rgba(0,0,0,0.30)`. 12 px radius. Cancel left,
Confirm right (macOS convention). ARIA in `accessibility.md`.

```css
.modal-backdrop {
  position: fixed; inset: 0;
  background: rgba(0, 0, 0, 0.30);
  display: flex; align-items: center; justify-content: center;
}
.modal {
  width: 440px;
  background: var(--surface);
  border-radius: 12px;
  padding: 20px 24px;
  display: flex; flex-direction: column; gap: 16px;
}
.modal h2 { margin: 0; font-size: 13px; font-weight: 700; }
.modal-actions { display: flex; justify-content: flex-end; gap: 8px; }
```

Only for blocking flows (destructive confirmations, rename with
side-effects). Never for completion messages — use a toast.

## Toast (HUD)

Top-center, dark translucent. Ephemeral. See `feedback-ladder.md`
for when a toast is correct.

```css
.toast-container {
  position: fixed; top: 16px; left: 50%;
  transform: translateX(-50%);
  display: flex; flex-direction: column; gap: 8px;
  z-index: 1000;
}
.toast {
  background: rgba(0, 0, 0, 0.75);
  color: white;
  border-radius: 10px;
  padding: 10px 16px;
  font-size: 13px;
  backdrop-filter: blur(20px);
}
.toast.error { background: rgba(220, 60, 55, 0.85); }
```

## Empty state

Quiet, concrete, one action at most. Keychain-Access calm, not Things-3-
warm.

```tsx
<div className="empty-state" role="status">
  <Folder size={28} strokeWidth={1} />
  <p>No projects yet.</p>
  <p className="empty-state-hint">
    Projects appear after you run <code>claude</code> in a directory.
  </p>
</div>
```

```css
.empty-state {
  display: flex; flex-direction: column; align-items: center;
  gap: 8px;
  padding: 48px 24px;
  color: var(--muted);
  text-align: center;
}
.empty-state svg { color: var(--tertiary); }
.empty-state p { margin: 0; font-size: 13px; }
.empty-state-hint { font-size: 11px; }
```

## Buttons

28 px height. 0.5 px border. 6 px radius. Background fill on hover,
never border change.

```css
.btn {
  height: 28px; padding: 0 12px;
  border: 0.5px solid var(--border);
  border-radius: 6px;
  background: var(--bg);
  color: var(--text);
  font-size: 13px;
  cursor: default;
  display: inline-flex; align-items: center; gap: 6px;
}
.btn:hover { background: var(--hover-bg); }
.btn.primary {
  background: var(--accent);
  color: var(--accent-text);
  border-color: transparent;
  font-weight: 500;
}
.btn.danger {
  color: var(--bad);
  border-color: var(--bad);
}
.btn.danger.primary {
  background: var(--bad);
  color: white;
  border-color: transparent;
}
.btn:focus-visible {
  outline: 3px solid var(--focus-ring);
  outline-offset: 2px;
}
.btn.icon-only {
  width: 28px; padding: 0; justify-content: center;
}
```

Only one `.btn.primary` visible at a time.

## Density defaults

- **Padding up before padding down.** When cramped, add space. The
  references are airier than you think.
- **Sizes 11 / 13 / 15 / 22.** Don't invent sizes between these.
- **0.5 px borders.** On retina, 1 px reads as truck-sized.
- **Hover reveals detail.** At rest, rows are quiet. Hover adds tint
  and surfaces menu triggers. Resist showing everything at rest.

## Named Claudepot anti-patterns

These are the failures we've actually shipped. `design-review.md`
promotes all of them to BLOCK.

1. **Two navigation surfaces** — rail plus a fixed sidebar that routes
   to a third view. A list pane with selectable rows is not a second
   nav.
2. **Icon-only segmented controls** with ambiguous meaning.
3. **Zero-state metadata** — rendering `0 sessions · …` at all.
4. **Internal identifiers** in the primary detail grid (principle §1).
5. **Disabled buttons without inline reason** (principle §3).
6. **Horizontal separator across the traffic-light zone** — breaks
   unified title bar.
7. **Scroll container clipping the first row** — sticky filter bar
   plus scrollable list requires the list's top padding to equal the
   sticky bar's height.
8. **Status spray** — same event firing toast + banner + strip.
9. **Toasting a persistent state** — locked keychain as a 4-second
   toast.
10. **Silent long op** — background op with no `RunningOpStrip` entry.

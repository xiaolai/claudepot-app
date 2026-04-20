---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# Accessibility Standards

The accessibility floor for every component. Not the ceiling — pair
with the principles in `design-principles.md`. Tokens cited here come
from `ui-design-system.md`; never invent new color/size values when
satisfying these rules.

## Keyboard navigation

Every interactive element must be keyboard-reachable via Tab.

- Use real `<button>` and `<input>` elements wherever possible — they
  carry keyboard semantics for free.
- Custom interactive elements (`<li role="option">`, `<div>` shells)
  require `tabIndex={0}` and an `onKeyDown` handler that activates on
  `Enter` and `Space`.
- Focus order follows visual order. No positive `tabIndex`.
- Modifier shortcuts must not fire when an `<input>` or `<textarea>`
  has focus, or when a modal is open. Check `document.activeElement`.

## Focus visibility

Every interactive element shows a visible focus indicator. Use
`:focus-visible` (not `:focus`) so mouse clicks don't show rings.

The paper-mono primitives (`Button`, `IconButton`, `SidebarItem`,
selectable list rows) already render an accent-bordered focus ring.
When you author CSS for new interactive surfaces, mirror them:

```css
.your-element:focus-visible {
  outline: 3px solid var(--accent-border);
  outline-offset: -1px;
}
```

`outline-offset: -1px` keeps the ring inside hairline-bordered
controls so it never visually clashes with the existing border.

## Disabled states

Disabled controls must:

1. Set the `disabled` HTML attribute (or `aria-disabled` for custom
   roles like menu items).
2. Show an inline reason near the control — small `var(--fg-faint)`
   text. The `Button` primitive renders `opacity: 0.45` automatically;
   the inline reason is your job.
3. Optionally carry a `title` tooltip as a *secondary* explanation.
   Tooltips are not a substitute for visible reason text.

Color and opacity alone never indicate disabled state — pair with
muted text or a dashed-border treatment (see the disabled `ActionCard`
CTA in `src/sections/accounts/ActionCard.tsx`).

## Color and contrast

Color alone never conveys meaning. Always pair with:

- A text label (`valid`, `expired`, not just a green/red dot).
- An icon or shape difference.
- Position / context.

The `Tag` primitive (`src/components/primitives/Tag.tsx`) bakes this
in — every tone (`ok | warn | danger | accent`) renders both color
and a glyph + text. Reach for it before inventing a color-only badge.

Contrast targets:

- Body text against `--bg`: WCAG AAA (≥ 7:1).
- Muted/faint text: WCAG AA Large at minimum.
- Tag/badge text against the colored background: WCAG AA Normal at
  minimum.

The OKLCH palette in `tokens.css` is authored to clear AA across both
themes. If you introduce a new color, verify contrast in both light
and dark before checking in.

## Modals

Every modal must satisfy all of these. The `Modal` primitive
(`src/components/primitives/Modal.tsx`) handles items 1, 2, 3, and 5;
items 4 and 6 are the consumer's job.

1. `role="dialog"` + `aria-modal="true"` on the dialog div.
2. `aria-labelledby` pointing to the header element's `id` (use
   `useId()` and pass to `<ModalHeader id={…}>`).
3. Escape key handler that calls `onClose`.
4. Focus trap: Tab / Shift+Tab wraps within the modal. Use the
   `useFocusTrap` hook (`src/hooks/useFocusTrap.ts`) — `AddAccountModal`
   is the reference implementation.
5. Backdrop click closes (the primitive does this).
6. `autoFocus` on the primary action or the first focusable input.

Never use `window.confirm()`, `window.alert()`, or `window.prompt()` —
they don't render reliably in Tauri webviews. Always use `Modal` +
`ConfirmDialog`.

## ARIA

- Status banners: `role="alert"` (the `AnomalyBanner` and pending
  journals banner already do this).
- Toasts: container with `aria-live="polite"` (info) or
  `aria-live="assertive"` (error). `ToastContainer` already wires
  this — pick the right severity when calling `pushToast`.
- Icon-only buttons: `aria-label` describing the action.
  `IconButton` accepts `aria-label`; pass it.
- Navigation state: `aria-current="page"` on the active sidebar item
  (`SidebarItem` does this automatically when `active` is true).
- Listboxes: `<ul role="listbox">` + `<li role="option" aria-selected
  tabIndex={0}>`. The `ProjectsTable` and account-card grid follow
  this; new lists must too.
- Tab navigation: `role="tab"` + `aria-selected` for tab buttons,
  matching the Settings sub-nav and the projects filter strip.

## Semantic HTML

Use the right element for the role:

- `<main>` wraps the content column inside the shell.
- `<header>` (the `ScreenHeader` primitive) introduces a screen.
- `<section>` for grouped content within a screen.
- `<article>` for standalone cards (`AccountCard` uses this).
- `<aside>` for the app sidebar and the project detail pane.
- `<nav>` for breadcrumb groups.
- `<h1>` is the screen title (one per screen, set by `ScreenHeader`).
  `<h2>` for in-content section headings (Settings tab titles).
  Don't skip levels.

Never use a heading purely for styling — reach for `.mono-cap` or a
`SectionLabel` instead.

## Motion

Respect `prefers-reduced-motion`. Wrap any non-trivial animation in:

```css
@media (prefers-reduced-motion: no-preference) {
  .your-thing { transition: opacity var(--dur-base) var(--ease-out); }
}
```

The paper-mono primitives use only `background` / `color` / `opacity`
transitions at 80–120ms — short enough to feel snappy without
reduced-motion override. If you author a longer or layout-affecting
animation, you must wrap it.

Animate only `opacity` and `transform`. Never animate layout
properties (`width`, `height`, `top`, `left`, `padding`).

## High contrast

Respect `prefers-contrast: more`. Borders and separators must become
more prominent; text contrast must remain at WCAG AAA.

```css
@media (prefers-contrast: more) {
  :root {
    --line:        oklch(85% 0.005 60);
    --line-strong: oklch(75% 0.006 60);
  }
}
@media (prefers-contrast: more) and (prefers-color-scheme: dark) {
  :root {
    --line:        oklch(40% 0.008 60);
    --line-strong: oklch(50% 0.010 60);
  }
}
```

Add a `prefers-contrast: more` block to `tokens.css` whenever you
introduce a low-contrast border/separator token.

## Reduced transparency

Respect `prefers-reduced-transparency`. The `Modal` primitive uses a
2px backdrop blur; if `prefers-reduced-transparency` is set, fall
back to opaque:

```css
@media (prefers-reduced-transparency) {
  .modal-backdrop,
  .palette,
  .toast {
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    background: var(--bg);
  }
}
```

The `WindowChrome`, `AppSidebar`, and `AppStatusBar` are already
opaque (`var(--bg)` / `var(--bg-sunken)`) — no override needed.

## Context menus

Every interactive object in a primary data view must have an
`onContextMenu` handler with the most common actions for that object.
macOS users expect this universally. Items must be keyboard-navigable
(see the `ContextMenu` component).

Menu items must mirror visible actions in the same view — context
menus are a shortcut, not a separate feature surface.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| ⌘K / Ctrl+K | Open command palette |
| ⌘R | Refresh |
| ⌘N | Add account |
| ⌘, | Settings |
| ⌘1/2/3/4 | Switch to nth primary-nav section |
| ⌘F | Focus search input |
| Esc | Close modal / palette |

Wire app-level shortcuts via `useGlobalShortcuts`. They must not fire
while a modal is open or an input is focused. Standard Cmd+Q/W/H/M
come free from Tauri.

All shortcuts must be discoverable: shown in the UI (Kbd hint inside
`WindowChrome`'s palette button, tooltips on toolbar buttons), or
documented in this rules file.

---
globs: ["src/**/*.tsx", "src/**/*.css"]
---

# Accessibility Standards

## Keyboard navigation

Every interactive element must be keyboard-reachable via Tab.
Custom interactive elements (non-button/input) need `tabIndex={0}`
and `onKeyDown` handling for Enter/Space.

Focus order must follow visual order. Do not use positive `tabIndex`
values.

## Focus visibility

All interactive elements must show a visible focus indicator.
Use `:focus-visible` (not `:focus`) to avoid showing focus rings
on mouse clicks.

```css
button:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 2px;
}
```

## Disabled states

Disabled buttons must:
1. Set the `disabled` HTML attribute.
2. Show a visible inline reason (small muted text near the button).
3. Have a `title` tooltip as a secondary explanation.

Color alone must never be the sole indicator of disabled state —
use `opacity: 0.45` AND muted text.

## Color and contrast

Color alone never conveys meaning. Always pair with:
- Text label (e.g., "valid", "expired", not just green/red dot)
- Icon or shape difference
- Position/context

Token badges use both color AND text to convey status:
- `ok` = green + "valid · Nm"
- `bad` = red + "expired"
- `warn` = amber + status text

## Modals

Every modal must satisfy all of these:
- `role="dialog"` and `aria-modal="true"` on the `.modal` div
- `aria-labelledby` pointing to the heading's `id`
- Escape key handler calling `onClose`/`onCancel` (via `useEffect` + `keydown`)
- Focus trap: Tab/Shift+Tab wraps within the modal
- Backdrop click to close
- `autoFocus` on the primary action or first focusable element

## ARIA

- Status messages: `role="alert"` on banners (e.g., keychain locked)
- Toasts: container has `aria-live="polite"` (info) or `aria-live="assertive"` (error)
- Icon-only buttons: `aria-label="descriptive text"`
- Active state indicators: `aria-current="true"` on the active account card

## Semantic HTML

- `<main>` for app container
- `<header>` for brand + status area
- `<footer>` for actions bar
- `<section>` for account list
- `<article>` for individual account cards
- `<h1>` app title, `<h2>` modal titles, `<h3>` card titles

Do not skip heading levels. Do not use headings for styling only.

## Motion

Respect `prefers-reduced-motion`. Wrap animations in:

```css
@media (prefers-reduced-motion: no-preference) {
  .toast { animation: toast-in 0.15s ease; }
}
```

## High contrast

Respect `prefers-contrast: more`. When enabled, borders and separators
must become more prominent. Text contrast must remain at WCAG AAA.

```css
@media (prefers-contrast: more) {
  :root { --border: rgba(0, 0, 0, 0.30); }
}
@media (prefers-contrast: more) and (prefers-color-scheme: dark) {
  :root { --border: rgba(255, 255, 255, 0.30); }
}
```

Every new color token must have a `prefers-contrast: more` variant
if the default value has low contrast against adjacent surfaces.

## Reduced transparency

Respect `prefers-reduced-transparency`. When enabled, translucent
surfaces (vibrancy sidebar) must fall back to opaque:

```css
@media (prefers-reduced-transparency) {
  .sidebar { background: var(--bg); }
}
```

Tauri's `windowEffects` vibrancy is handled at the OS level, but the
CSS transparent background would show nothing without vibrancy —
the opaque fallback prevents a blank sidebar.

## Context menus

Every interactive object must have a right-click context menu with
the most common actions for that object. macOS users expect this
universally. Context menu items must be keyboard-navigable.

## Keyboard shortcuts

Standard macOS shortcuts (Cmd+Q, W, H, M) are provided by Tauri.
App-specific shortcuts (Cmd+R refresh, Cmd+N add, Cmd+, settings)
must be wired in the frontend. See `rules/ui-design-system.md` for
the full shortcut table.

All keyboard shortcuts must be discoverable: either shown in a menu
bar or documented in tooltips on the corresponding buttons.

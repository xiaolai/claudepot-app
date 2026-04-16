---
globs: ["src/**/*.tsx", "src/**/*.ts"]
---

# React Component Conventions

## File structure

One component per file. Max 120 lines per component file.
Props interface defined inline above the component (no separate types file
for component-local types).

```
src/
  App.tsx              — shell: IconContext provider, layout, modals, toasts
  components/
    Sidebar.tsx        — account list sidebar (transparent for vibrancy)
    ContentPane.tsx    — selected account detail + actions
    AccountDetail.tsx  — detail grid (email, UUID, timestamps, etc.)
    AddAccountModal.tsx
    ConfirmDialog.tsx
    ToastContainer.tsx — HUD-style toasts (top-center)
    CopyButton.tsx
    EmptyState.tsx
    TokenBadge.tsx
  hooks/               — custom hooks (reused across 2+ components)
  api.ts               — Tauri invoke wrappers
  types.ts             — shared DTO types (synced with Rust dto.rs)
  test/fixtures.ts     — test data factories
```

## Component patterns

- Functional components only. No class components (except Error Boundary).
- Props destructured in the function signature.
- No default exports except `App.tsx`. Use named exports everywhere else.
- Co-locate component CSS classes in `App.css` under a section comment
  (`/* ---------- component-name ---------- */`).
- All icons from `@phosphor-icons/react`. Never use emoji or text for icons.

## Layout

App uses a sidebar + content pane split:
- `Sidebar` — 240px, transparent background for macOS vibrancy
- `ContentPane` — flex:1, opaque `var(--bg)` background
- Selection: click sidebar item → ContentPane shows that account's detail

## Hooks

Extract a custom hook when:
- Logic is reused across 2+ components, OR
- A single component's hook logic exceeds 20 lines

Hook files: `src/hooks/useName.ts`. Export a single named function.

## State management

- No state library. React `useState` + `useCallback` + prop drilling.
- State that crosses an `await` point: use functional updater
  `setState(prev => ...)` to avoid stale closures.
- Busy states: per-entity Set. Each component checks membership for its key.

## Modals and dialogs

See `rules/accessibility.md` for full ARIA and keyboard requirements
on modals (role, aria-modal, Escape handler, focus trap).

Never use `window.confirm()`, `window.alert()`, or `window.prompt()`.
These are invisible in Tauri webviews. Always use in-app dialog components.

## Context menus

Every interactive object (account card, project row, token badge)
must have an `onContextMenu` handler providing relevant actions.

Pattern:
```tsx
const handleContextMenu = useCallback((e: React.MouseEvent) => {
  e.preventDefault();
  // Show custom context menu with actions relevant to this item
}, []);
```

Use a shared `<ContextMenu>` component. Menu items must match the
actions available via buttons/icons in the same view — context menus
are a shortcut, not a separate feature surface.

See `rules/ui-design-system.md` for the required menu items per object.

## Keyboard shortcuts

Wire app-level shortcuts via a `useKeyboardShortcuts` hook in App.tsx.
Shortcuts must not fire when a modal is open or an input is focused
(check `document.activeElement`).

See `rules/ui-design-system.md` for the full shortcut table.

## Buttons

- Always provide a `title` tooltip explaining the action.
- Use `cursor: default` (set globally). Never add `cursor: pointer`.
- Disabled buttons: set `disabled` AND provide a visible inline hint
  explaining why (not just a tooltip).
- Use className variants: `primary`, `danger`, `warn`,
  `danger primary` (filled danger).
- Icon-only buttons: use `.icon-btn` class (28x28, no border).

## Toasts

- HUD-style, top-center positioned. Dark translucent background.
- Info toasts: auto-dismiss after 4 seconds.
- Error toasts: persist until manually dismissed (close button).
- Toast IDs: incrementing counter, not `Date.now() + Math.random()`.

## Data fetching

- All Tauri invocations go through `src/api.ts`. Components never call
  `invoke()` directly.
- Refresh on window focus (debounced 2s) + manual refresh button.
- Startup sync: call `syncFromCurrentCc()` before loading account list.

## Testing

- Test file next to component: `ComponentName.test.tsx`
- Use `vi.doMock` + dynamic import pattern for Tauri command mocking
  (see existing `App.test.tsx`).
- Test fixtures in `src/test/fixtures.ts` — factory functions with
  `Partial<T>` overrides.
- Test behavior, not implementation. Assert on visible text and ARIA
  roles, not CSS classes or internal state.
- Email text may appear in multiple places (sidebar + content + detail).
  Use `getAllByText` / `findAllByText` when asserting on email presence.

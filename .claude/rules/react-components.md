---
globs: ["src/**/*.tsx", "src/**/*.ts"]
---

# React Component Conventions

## File structure

<!-- TARGET: after WI-10 (component decomposition) -->
<!-- Until WI-10, App.tsx contains all components in one file. -->

One component per file. Max 120 lines per component file.
Props interface defined inline above the component (no separate types file
for component-local types).

```
src/
  App.tsx              — shell: providers, layout, toast container
  components/          — all UI components
  hooks/               — custom hooks (reused across 2+ components)
  api.ts               — Tauri invoke wrappers
  types.ts             — shared DTO types (synced with Rust dto.rs)
  test/fixtures.ts     — test data factories
```

## Component patterns

- Functional components only. No class components (except Error Boundary).
- Props destructured in the function signature.
- No default exports except `App.tsx`. Use named exports everywhere else.
  <!-- TARGET: applies after WI-10 component decomposition -->
- Co-locate component CSS classes in `App.css` under a section comment
  (`/* ---------- component-name ---------- */`).

## Hooks

Extract a custom hook when:
- Logic is reused across 2+ components, OR
- A single component's hook logic exceeds 20 lines

Hook files: `src/hooks/useName.ts`. Export a single named function.

## State management

- No state library. React `useState` + `useCallback` + prop drilling.
- State that crosses an `await` point: use functional updater
  `setState(prev => ...)` to avoid stale closures.
- Busy states: per-entity Set (not a global string). Each component
  checks membership for its own key.
  <!-- TARGET: after WI-2. Current code uses a single busy string. -->

## Modals and dialogs

See `rules/accessibility.md` for full ARIA and keyboard requirements
on modals (role, aria-modal, Escape handler, focus trap).

Never use `window.confirm()`, `window.alert()`, or `window.prompt()`.
These are invisible in Tauri webviews. Always use in-app dialog components.

## Buttons

- Always provide a `title` tooltip explaining the action.
- Disabled buttons: set `disabled` AND provide a visible inline hint
  explaining why (not just a tooltip — tooltips are invisible on touch
  and require hover on disabled elements).
- Use className variants: `primary`, `danger`, `warn`,
  `danger primary` (filled danger).

## Toasts

- Info toasts: auto-dismiss after 4 seconds.
- Error toasts: persist until manually dismissed (close button).
  <!-- TARGET: after WI-8. Current code auto-dismisses all toasts. -->
- Toast IDs: incrementing counter, not `Date.now() + Math.random()`.

## Data fetching

- All Tauri invocations go through `src/api.ts`. Components never call
  `invoke()` directly.
- Refresh on window focus (debounced 2s) + manual refresh button.
  <!-- TARGET: after WI-1. Current code loads once on mount. -->
- Startup sync: call `syncFromCurrentCc()` before loading account list.

## Testing

- Test file next to component: `ComponentName.test.tsx`
- Use `vi.doMock` + dynamic import pattern for Tauri command mocking
  (see existing `App.test.tsx`).
- Test fixtures in `src/test/fixtures.ts` — factory functions with
  `Partial<T>` overrides.
- Test behavior, not implementation. Assert on visible text and ARIA
  roles, not CSS classes or internal state.

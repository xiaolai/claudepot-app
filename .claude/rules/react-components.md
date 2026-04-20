---
globs: ["src/**/*.tsx", "src/**/*.ts"]
---

# React Component Conventions

## File structure

One component per file. Max 120 lines per component file (kept loose
where the file owns a tightly-coupled set of helpers — e.g.,
`AddAccountModal` ships its preflight summary mapper alongside).
Props interfaces inline above the component; no separate types file
for component-local types.

```
src/
  App.tsx                — shell mount: providers + WindowChrome +
                            Sidebar + content + StatusBar
  main.tsx               — entry; loads tokens.css then App.css
  api.ts                 — Tauri invoke wrappers (one per command)
  types.ts               — shared DTOs synced with Rust dto.rs
  icons.ts               — NF codepoint map (kebab-case ICONS + camelCase NF)
  styles/
    tokens.css           — single source of truth for tokens
  components/
    primitives/          — paper-mono primitives (Glyph, Button,
                            IconButton, Input, Tag, Kbd, Divider,
                            SectionLabel, Avatar, SidebarItem, Modal*,
                            DevBadge)
    Icon.tsx             — legacy kebab-case glyph wrapper, still used
                            by unported components; new code uses Glyph
    PendingJournalsBanner.tsx
    RunningOpStrip.tsx
    OperationProgressModal (under sections/projects/)
    ContextMenu.tsx
    CommandPalette.tsx
    ConfirmDangerousAction.tsx
    ConfirmDialog.tsx
    ToastContainer.tsx
    CopyButton.tsx · CollapsibleSection.tsx · EmptyState.tsx ·
    SegmentedControl.tsx
  shell/                 — paper-mono shell (WindowChrome, AppSidebar,
                            AppStatusBar, ScreenHeader,
                            SidebarTargetSwitcher)
  sections/
    registry.tsx         — primary-nav SectionDef[]
    AccountsSection.tsx · ProjectsSection.tsx · SettingsSection.tsx ·
    SessionsSection.tsx
    accounts/            — AccountCard, UsageBlock, AnomalyBanner,
                            HealthFooter, ActionCard, AddAccountModal,
                            format
    projects/            — ProjectsTable, ProjectDetail, plus the
                            existing rename/clean/adopt/repair flows
    settings/            — ProtectedPathsPane (others inline)
  hooks/                 — custom hooks (theme, dev mode, accounts,
                            sections, refresh, usage, ops, palette,
                            tauri-events, focus trap, …)
  test/fixtures.ts       — test data factories
```

## Component patterns

- Functional components only. No class components except `ErrorBoundary`.
- Props destructured in the function signature.
- Named exports everywhere; the only default export is `App.tsx`.
- Inline styles are the paper-mono norm — primitives inline their
  token-driven styles. Class-based CSS in `App.css` is reserved for
  unported legacy components and a few global concerns; do not author
  new `.foo` classes for new primitives.
- For decorative chrome (rules, separators, container backgrounds),
  use the token vars directly (`var(--line)`, `var(--bg-sunken)`).
- Co-locate small helper components in the same file when they exist
  only to compose the public component (e.g., `TargetSwitchOption`
  inside `SidebarTargetSwitcher.tsx`). Bigger helpers move to a
  sibling file.

## Icons — Nerd Font, never SVG libraries

All iconography is Nerd Font codepoints. Two surfaces:

- **New code (paper-mono):** `Glyph` primitive
  (`src/components/primitives/Glyph.tsx`) + the camelCase `NF` map in
  `src/icons.ts`. Reach for `<Glyph g={NF.warn} />`.
- **Legacy code:** `Icon` component (`src/components/Icon.tsx`) +
  the kebab-case `ICONS` map. Still used by unported components
  (PendingJournalsBanner, ConfirmDangerousAction, ContextMenu, etc.).
  When migrating a component to paper-mono, switch from `Icon` to
  `Glyph` at the same time.

Never install or import `lucide-react`, `heroicons`, `@phosphor-icons`,
Font Awesome SVGs, or any other icon library. Never use emoji as
icons. If a glyph isn't in the NF map, look up its codepoint on
[the Nerd Fonts cheat sheet](https://www.nerdfonts.com/cheat-sheet)
and add it.

## Layout

The app shell is fixed:

- `WindowChrome` 38px top — breadcrumb, ⌘K hint, theme toggle.
- `AppSidebar` 240px left — swap targets + primary nav + tree + sync.
- Content column flex:1 — section bodies render here.
- `AppStatusBar` 24px bottom — stats + active model.
- `PendingJournalsBanner` and `RunningOpStrip` mount inside the
  content column when active; `OperationProgressModal` overlays.

Section bodies own their own internal layout (Accounts: card grid;
Projects: table + right detail pane; Settings: 200px tab nav + pane;
Sessions: empty state).

## Hooks

Extract a custom hook when:

- Logic is reused across 2+ components, OR
- A single component's hook logic exceeds 20 lines, OR
- The hook crosses an `await` boundary and benefits from being unit-
  testable in isolation.

Hook files: `src/hooks/useName.ts`. Export a single named function;
add a second exported helper only when it is paired with the hook
(e.g., `useAccounts` + `bindingFrom`).

Existing hooks worth knowing about:

- `useAccounts` — global account list + window-focus refresh.
- `useTheme` — light/dark/system, persisted to `cp-theme`.
- `useDevMode` — DevBadge toggle, persisted to `cp-dev-mode`.
- `useSection` — primary-nav routing + per-section sub-routes.
- `useRefresh` / `useUsage` — accounts state inside AccountsSection.
- `useOperations` / `useRunningOps` — long-op pipeline.
- `useFocusTrap` — modal focus trap.
- `useTauriEvent` — typed listen() with cleanup.
- `useGlobalShortcuts` — ⌘R / ⌘N / ⌘K wiring.

## State management

- No state library. React `useState` + `useCallback` + prop drilling
  + a couple of `Context` providers for cross-section state
  (`OperationsProvider`).
- State that crosses an `await` boundary: use the functional updater
  `setState(prev => …)` to avoid stale closures.
- Token-sequenced refresh: when a load can be triggered from multiple
  callers (mount, ⌘R, completion), use the `refreshTokenRef` pattern
  — see `ProjectsSection` and `SettingsSection.DiagnosticsPane`.
- Per-entity busy: `Set<string>` keyed by an action+id (`re-${uuid}`).
  Components pass `loginBusy={busy.has(`re-${a.uuid}`)}`, never share
  a single global busy flag.

## Modals and dialogs

Use the `Modal` primitive (`src/components/primitives/Modal.tsx`) for
every dialog. Compose with `ModalHeader` / `ModalBody` / `ModalFooter`.
Pair with `useFocusTrap` and `useId` for `aria-labelledby`. See
`AddAccountModal` for the canonical example.

Never use `window.confirm()`, `window.alert()`, or `window.prompt()`
— invisible in Tauri webviews. Use `ConfirmDialog` (legacy) or
`ConfirmDangerousAction` (consequence-loud) for destructive flows.

## Context menus

Every interactive object in a primary data view (account card,
project row, token badge) must have an `onContextMenu` handler with
relevant actions. Use the shared `ContextMenu` component.

Pattern:

```tsx
const handleContextMenu = useCallback(
  (e: React.MouseEvent, target: T) => {
    e.preventDefault();
    setCtxMenu({ x: e.clientX, y: e.clientY, target });
  },
  [],
);
```

Menu items must match the actions available via buttons in the same
view — context menus are a shortcut, not a separate feature surface.

## Keyboard shortcuts

Wire app-level shortcuts via `useGlobalShortcuts`. The `useSection`
hook handles ⌘1..⌘N. `WindowChrome` exposes ⌘K via the palette hint
button; `AccountsSection` listens for the `cp-open-palette` event so
the click in chrome can open the palette without prop-drilling.

Shortcuts must not fire when a modal is open or an input is focused
— check `document.activeElement`. Full table in
`ui-design-system.md` and `accessibility.md`.

## Buttons

- Use the `Button` primitive — five variants
  (`solid · ghost · subtle · outline · accent`) plus a `danger`
  modifier. `IconButton` for square icon-only buttons.
- Always provide a `title` tooltip explaining the action; pass
  `aria-label` to `IconButton`.
- Disabled buttons: pass `disabled` AND show an inline reason text
  next to the button (`accessibility.md`).
- Only one `solid` button visible per view — that is the primary
  action.

## Toasts

- HUD-style, top-center positioned, dark translucent.
- Info toasts: auto-dismiss after 4 seconds.
- Error toasts: persist until manually dismissed.
- Toast IDs: incrementing counter, not `Date.now() + Math.random()`.
- Container is mounted by `AccountsSection` and `SettingsSection` —
  one per section that pushes toasts.

For long ops, do NOT toast "renaming…" — the `RunningOpStrip` owns
that signal. Toast on terminal events only.

## Data fetching

- All Tauri invocations go through `src/api.ts`. Components never
  call `invoke()` directly.
- Refresh on window focus (debounced 2s in `useAccounts` and inside
  the section hooks) + manual refresh button.
- Startup sync: `useAccounts` calls `syncFromCurrentCc()` before
  loading the account list.
- Long-running ops use the `*_start` → `op_id` → event pipeline; see
  `useOperations` and `OperationProgressModal`.

## Tests

- Test file next to component: `Component.test.tsx`.
- Use `vi.doMock` + dynamic import for Tauri command mocking — see
  `App.test.tsx`. Reset modules + clear `localStorage` in
  `beforeEach`; `useTheme`, `useDevMode`, `useSection` all persist.
- Test fixtures in `src/test/fixtures.ts` — factory functions with
  `Partial<T>` overrides.
- Test behavior, not implementation. Assert on visible text and ARIA
  roles, not CSS classes or internal state.
- Email and other identifiers may appear in multiple places (sidebar
  swap-target preview + account card + status bar). Prefer
  `findAllByText` and assert on the count, or scope queries with
  `within(region)`.

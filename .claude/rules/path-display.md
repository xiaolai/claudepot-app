---
description: How to display file system paths so the user can always read and copy the full string
when_applicable:
  - Adding a UI element that shows a file path, directory path, or absolute filesystem location
  - Refactoring a list / table row that includes a path column
  - Reviewing a PR that introduces path UI
---

# Path display

Filesystem paths are first-class data. The user must always be able
to (a) **read** the full path, and (b) **copy** the full path. The
default visible string may be truncated for layout — the rule fills
in the gaps that truncation creates.

## Three states

A path display falls into exactly one of three states:

### State A — Fully visible (no truncation)

Example: a 25-character path inside a 60-character-wide cell. The
visible string IS the full string.

- No tooltip required. (One is harmless but redundant.)
- Copy button required only when the path is the *primary* data of
  the surface (project header, file viewer header). For incidental
  mentions inside a paragraph, native text-selection is sufficient.
- The text must be `user-select: text` (or carry `.selectable`) so
  the user can grab it with the cursor.

### State B — Truncated, copy reachable elsewhere

Example: a project path truncated in a list row whose detail view
already shows the full path with a copy button.

- **Tooltip mandatory** — `title` attribute (or richer tooltip
  primitive) carrying the full path so hover discloses it.
- Copy button **not required** on the truncated instance. The detail
  surface (header / breadcrumb / drawer) is the canonical copy site.
- Document the canonical copy site in the row component's comment so
  reviewers don't add a redundant copy button later.

### State C — Truncated, no canonical copy site

Example: a one-row toast that names a path; a context-menu entry; a
breadcrumb segment that's the only place the path appears.

- **Tooltip mandatory.**
- **Copy affordance required** — either a sibling `<CopyButton>`, a
  context-menu "Copy path" entry, or a row kebab. Inline button is
  the default; promote to a kebab when there are other secondary
  actions to group.

## Anti-patterns

- **Truncated path with no `title`**: the user sees `…/foo/bar` and
  can't see what was clipped. Ship a `title` even if a copy button
  exists — they answer different needs (read vs. paste).
- **Tooltip showing the SAME visible string**: useless. The tooltip
  must carry the *full* path. If the visible string is already
  full, drop the tooltip.
- **Copy button on a row in a 200-row list**: visual noise. Move to a
  row kebab / context menu and document that pattern in the row
  component.
- **Selecting text as the only way to copy a long path**: works for
  power users, fails everyone else. If the path is critical, give a
  copy button.
- **Native `title` attribute for paths longer than ~120 chars on
  Windows**: GDI clips the tooltip text. Long paths warrant a real
  tooltip primitive, not the native one. (For now, native is fine
  for Mac/Linux; revisit when Windows users hit it.)
- **Right-truncation that hides the basename**: paths read more
  meaningfully from the *end* (basename). Prefer left-truncation
  (`text-overflow: ellipsis` with `direction: rtl` + `text-align:
  left` flip, or a manual middle-truncate helper) for long paths in
  narrow columns. The basename is the identity; the prefix is
  shared context.

## Implementation primitives

- `<CopyButton text={fullPath} />` — paper-mono icon button, copies
  via `navigator.clipboard` (fine for non-secret values from our own
  renderer; secrets must use the Rust-side `key_*_copy` clipboard
  commands — see rules/architecture.md), shows a brief success
  state. Use this for state C and for the canonical copy site in
  state B.
- `title={fullPath}` — native tooltip, sufficient for state B.
- `.mono.selectable` — span class that combines monospace font and
  `user-select: text`. Pair with `direction` flip if you need
  middle-truncation.
- String-level truncation (e.g. inside a toast template): no shared
  helper exists yet. If you need one, add it to `src/lib/` as
  `formatPathDisplay(path, maxChars)` — middle-truncate preserving
  the basename (per the left-truncation guidance above) — rather
  than writing an ad-hoc truncator at the call site. Whatever
  produces the truncated string, make it accessible: the visible
  text is the truncated form, with `aria-label` (or `title`)
  carrying the full path.

## Surfaces already correct

- `ProjectDetail` header — full path, `selectable`, `<CopyButton>`. ✓
- `SessionDetailHeaderFull` — full session id and project path with
  `<CopyButton>` each. ✓

## Migration checklist for an existing path display

1. Identify state (A / B / C) by reading the column width and the
   detail surface that the row links to.
2. State A: ensure `.selectable`. Done.
3. State B: ensure `title={fullPath}` is set on the truncating
   element. Confirm the canonical copy site exists; comment the row
   to point to it.
4. State C: ensure `title={fullPath}` AND a `<CopyButton>` (or a
   row-kebab "Copy path" entry).
5. Never leave a truncated path in state "no tooltip, no copy."

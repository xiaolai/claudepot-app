# CC state that a project move must migrate — the blast radius

Claude Code stores per-project state in **two shapes**:

1. **Inside the project directory** (`.claude/settings.json`,
   `.mcp.json`, …) — travels automatically when the directory moves.
2. **In global files keyed by the project's absolute path** — does
   **not** travel. Every one of these is a move landmine: rename the
   project and the binding is left pointing at a path that no longer
   exists.

Shape 2 is the whole problem. The authoritative list below was
established empirically (grep the old path across all of `~/.claude`
after a real move; see the v0.1.53 plugin-binding incident). It is the
target set for `claudepot_core::project::move_project`'s phases **and**
for the staleness detectors.

## The path-keyed global state (and its owning phase)

| Global file | Keyed by | Migrated by |
|---|---|---|
| `projects/<slug>/*.jsonl` (transcripts) | sanitized abs path (dir name) | P4 (dir rename) + P6 (`cwd` fields) |
| `history.jsonl` | abs path | P5 |
| `~/.claude.json` `projects[<abs path>]` | abs path (map key) | P7 |
| auto-memory dir | sanitized git-root path | P8 |
| `<proj>/.claude/settings.json` `autoMemoryDirectory` | abs path (value) | P9 |
| `plugins/installed_plugins.json` `projectPath` | abs path (field) | **P10** |

Verified **not** path-keyed (safe across a move): `state.db` (keyed by
session UUID). Re-verify if CC's schema changes.

## The invariant

**Every global, path-keyed piece of CC state MUST have (a) a
`move_project` phase that migrates it, and (b) a staleness detector
that finds it after an *external* move.** One without the other is a
half-fix:

- Phase without detector → only helps when the move goes through
  `claudepot project move`. Users move with Finder / `mv` / `git`,
  which bypasses every phase. (This is how the plugin bug reached a
  user: the move was manual, so P4–P9 never ran, and there was no P10
  at all.)
- Detector without phase → surfaces the breakage but can't repair it.

Current detectors:

- Transcripts → `session::move_::detect_orphaned_projects` (slug whose
  `cwd` is gone) → repair via `project move <old> <new>`.
- Plugins → `project::detect_stale_plugin_bindings` (`projectPath`
  gone) → CLI `project plugin-bindings` → repair via `project move`.

`history.jsonl` and the `~/.claude.json` key are repaired by the same
`project move <old> <new>` reconcile (AlreadyMoved + `--merge`), so
they need no separate detector — but any NEW path-keyed file does.

## When you add a new path-keyed global CC file

1. Add a `move_project` phase (`Pn`) that rewrites it. Reuse
   `project_rewrite::rewrite_strings_in_value_pub` (boundary rule) for
   JSON, and snapshot before mutating (see P7/P10).
2. Add it to the table above.
3. Add a staleness detector (or fold it into the `project move`
   AlreadyMoved reconcile and note that here).
4. Cover all four path shapes in tests
   (`/Users/…`, `C:\…`, `\\server\share\…`, `\\?\C:\…`) per
   `rules/paths.md`.
5. Wire the phase into the dry-run plan, the CLI result, and the GUI
   progress map (`projectMoveProgress.tsx`) — a phase that isn't
   surfaced everywhere the others are is a review finding.

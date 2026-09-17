# case_02_null_clobber

`null` at a higher layer clobbers a populated container below it.

Verified: claude-code@2.1.274 by parity-harness/dump.ts (2026-09-17).

The key is a neutral probe (`xParityContainer`) on purpose. Claude Code
validates schema-typed keys before merging, and on 2.1.274 a
`"hooks": null` in project settings does **not** clobber the user's
hooks — the invalid `null` is dropped during validation, so the case
this fixture used to state with `hooks` now tests the schema rather than
the merge. An untyped key reaches the merge as written. Claudepot's
merge does not model that validation step; see the README's known
uncovered surface.

| key | winner | why |
|---|---|---|
| `xParityContainer` | project (`null`) | a later `null` replaces the whole container |
| `theme` | user (`"dark"`) | absent above — retained |

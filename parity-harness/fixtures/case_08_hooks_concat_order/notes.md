# case_08_hooks_concat_order

Arrays of objects (hook entries) across two layers: concatenated in
lower-then-upper order, and an EQUAL-CONTENT duplicate is kept — CC
never dedupes objects.

The user's `Bash` entry and the project's first entry are
byte-identical JSON. The merged output still contains both, because
CC's dedupe is identity-based, and two objects from separate JSON
parses are never identical. The fixture locks both properties: object
no-dedupe AND concat order (all user entries before all project
entries).

Derived from claude-code@2.1.88 source:

- Array-on-array → `mergeArrays`
  (`src/utils/settings/settings.ts:529-531`):
  `uniq([...targetArray, ...sourceArray])` — lower layer's elements
  first.
- `uniq` is `[...new Set(xs)]` (`src/utils/array.ts:11-13`). `Set`
  uses SameValueZero, which is reference identity for objects, so
  equal-content hook entries from different files are both kept.
- Empirically verified against CC's own dependency: `lodash-es`
  `mergeWith` from the 2.1.88 tree with the verbatim customizer
  produces exactly the 3-element array in expected.json (duplicate
  Bash entry preserved, order user-then-project).

Expected values:

| path | value | why |
|---|---|---|
| `hooks.PreToolUse[0]` | user's `Bash` entry | lower layer first |
| `hooks.PreToolUse[1]` | project's identical `Bash` entry | objects never deduped |
| `hooks.PreToolUse[2]` | project's `Edit` entry | upper layer order preserved |

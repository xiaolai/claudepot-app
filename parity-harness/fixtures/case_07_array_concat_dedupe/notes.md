# case_07_array_concat_dedupe

The same array-of-primitives key in two layers: concatenate
lower-then-upper, dedupe by value, first occurrence keeps its
position. First fixture to merge an array key ACROSS layers — case_04
only tested single-layer retention.

Derived from claude-code@2.1.88 source:

- Array-on-array goes through `settingsMergeCustomizer`
  (`src/utils/settings/settings.ts:538-547`) → `mergeArrays`
  (`settings.ts:529-531`): `uniq([...targetArray, ...sourceArray])` —
  target (lower) elements first, then source (upper).
- `uniq` is CC's own `[...new Set(xs)]` (`src/utils/array.ts:11-13`):
  SameValueZero semantics — value-equality dedupe for primitives,
  first occurrence wins the position.
- Empirically verified against CC's own dependency: `lodash-es`
  `mergeWith` from the 2.1.88 tree with the verbatim customizer gives
  `["Bash(git *)","Read"] + ["Read","Bash(npm *)"]` →
  `["Bash(git *)","Read","Bash(npm *)"]`.

Expected values:

| key | value | why |
|---|---|---|
| `permissions.allow` | `["Bash(git *)", "Read", "Bash(npm *)"]` | concat user-then-project, `"Read"` deduped at its first position |
| `permissions.deny` | `["WebFetch"]` | only user defines it — retained |

# case_02_null_clobber

`null` at higher precedence clobbers a populated container below.

Derived from claude-code@2.1.88 source:

- `settingsMergeCustomizer` (`src/utils/settings/settings.ts:538-547`)
  only special-cases array-on-array; for `hooks: null` over
  `hooks: {…}` it returns `undefined`, so lodash `mergeWith` default
  applies. lodash skips only `undefined` source values — `null` is
  assigned, replacing the destination object wholesale.
- Empirically verified against CC's own dependency: `lodash-es`
  `mergeWith` from the 2.1.88 tree with the verbatim customizer gives
  `mergeWith({a:{x:1}}, {a:null}, customizer)` → `{a:null}`.

Expected values:

| key | winner | why |
|---|---|---|
| `hooks` | project (`null`) | null clobbers the user's hooks object |
| `theme` | user (`"dark"`) | absent in project — retained |

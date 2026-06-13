# case_12_scalar_clobber_empty_object_noop

Two non-array shape collisions in one fixture:

1. A higher-precedence SCALAR (`sandbox: "disabled"`) clobbers a
   lower populated object wholesale — no key of the lower object
   survives.
2. A higher-precedence EMPTY OBJECT (`statusLine: {}`) is a no-op —
   the lower object survives untouched.

Derived from claude-code@2.1.88 source:

- Both cases pass through `settingsMergeCustomizer`
  (`src/utils/settings/settings.ts:538-547`), which returns
  `undefined` for non-array pairs, deferring to lodash `mergeWith`
  default behavior: a primitive source value is assigned over an
  object destination; an object source deep-merges, and an empty one
  contributes zero keys.
- Empirically verified against CC's own dependency: `lodash-es`
  `mergeWith` from the 2.1.88 tree with the verbatim customizer gives
  `{a:{x:1}} + {a:"str"}` → `{a:"str"}` and `{a:{x:1}} + {a:{}}` →
  `{a:{x:1}}`.

Expected values:

| key | winner | why |
|---|---|---|
| `sandbox` | project (`"disabled"`) | scalar clobbers the user's object |
| `statusLine` | user (full object) | empty object above is a no-op |

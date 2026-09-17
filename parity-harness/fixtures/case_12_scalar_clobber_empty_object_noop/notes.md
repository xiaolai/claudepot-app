# case_12_scalar_clobber_empty_object_noop

A higher scalar clobbers a lower object wholesale; a higher empty object
merges as a no-op.

Verified: claude-code@2.1.274 by parity-harness/dump.ts (2026-09-17).

Neutral probe keys again. With the real keys this fixture used,
`sandbox: "disabled"` and `statusLine: {}` both fail 2.1.274's schema,
and a project file with *any* validation error is skipped whole — so
the project layer never reached the merge at all.

| key | winner | why |
|---|---|---|
| `xParityObject` | project (`"disabled"`) | a scalar replaces the object below |
| `xParityEmpty` | user | an empty object adds nothing and removes nothing |

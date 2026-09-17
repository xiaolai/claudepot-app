# case_05_local_over_project

The local layer overrides project and user at shared keys.

Verified: claude-code@2.1.274 by parity-harness/dump.ts (2026-09-17).

The themes are real theme names. The previous inputs (`"local-theme"`,
…) are not valid values, and on 2.1.274 Claude Code drops an invalid
`theme` silently — no validation error — leaving the lower valid value
in place, so the fixture was testing the schema.

| key | winner | why |
|---|---|---|
| `theme` | local (`"dark-daltonized"`) | local is above project and user |
| `model` | local (`"sonnet"`) | local is above project |
| `editor` | user (`"vim"`) | only user defines it |

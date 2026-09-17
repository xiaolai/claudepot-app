# case_06_flag_over_local

The `--settings` flag layer overrides local; local's other keys stay.

Verified: claude-code@2.1.274 by parity-harness/dump.ts (2026-09-17).

Real theme names, for the reason given in case_05.

| key | winner | why |
|---|---|---|
| `theme` | flag (`"light-daltonized"`) | flag is above local |
| `verbose` | local (`true`) | absent in flag — retained |

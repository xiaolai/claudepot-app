# case_06_flag_over_local

The flag layer (`--settings` file / SDK inline settings) overrides
local at shared keys. First end-to-end coverage of the `flag` slot —
the four original fixtures all set `"flag": null`.

Derived from claude-code@2.1.88 source:

- `flagSettings` comes after `localSettings` (and before
  `policySettings`) in `SETTING_SOURCES`
  (`src/utils/settings/constants.ts:7-21`); default enablement order
  `src/bootstrap/state.ts:313-319`.
- The flag source merges in the same loop as the file sources: a
  `--settings <file>` path goes through `settings.ts:741-765`, and
  SDK inline settings merge at `settings.ts:771-781` — both at the
  flagSettings position, after local.
- Scalar conflict resolved by lodash default assignment
  (`settingsMergeCustomizer`, `settings.ts:538-547`).

Expected values:

| key | winner | why |
|---|---|---|
| `theme` | flag (`"flag-theme"`) | flag merges after project and local |
| `verbose` | local (`true`) | absent in flag — retained |

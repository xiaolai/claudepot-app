# case_05_local_over_project

The local layer (`settings.local.json`) overrides both project and
user at shared keys. First end-to-end coverage of the `local` slot —
the four original fixtures all set `"local": null`.

Derived from claude-code@2.1.88 source:

- `localSettings` comes after `projectSettings` and `userSettings` in
  `SETTING_SOURCES` (`src/utils/settings/constants.ts:7-21`, "Order
  matters - later sources override earlier ones"); the default
  enablement order is `src/bootstrap/state.ts:313-319`.
- Each file source is parsed and merged in loop order in
  `loadSettingsFromDisk` (`src/utils/settings/settings.ts:674`,
  `:741-765`), so local's `mergeWith` call runs last of the three and
  its scalars overwrite (`settingsMergeCustomizer`,
  `settings.ts:538-547`, returns `undefined` for non-arrays → lodash
  default assignment).

Expected values:

| key | winner | why |
|---|---|---|
| `theme` | local (`"local-theme"`) | defined in all three; local merges last |
| `editor` | user (`"vim"`) | only user defines it — retained |
| `model` | local (`"sonnet"`) | local overrides project |

# case_01_user_project_merge

Plain precedence: project overrides user at a shared scalar key;
keys missing from the higher layer are retained from the lower.

Derived from claude-code@2.1.88 source:

- Source order (user before project, later overrides earlier):
  `src/utils/settings/constants.ts:7-21` (`SETTING_SOURCES`, "Order
  matters - later sources override earlier ones") and the default
  enablement order `src/bootstrap/state.ts:313-319`.
- Merge loop: `src/utils/settings/settings.ts:674` (iterate enabled
  sources), `settings.ts:741-765` (parse file source, `mergeWith(...,
  settingsMergeCustomizer)`).
- Scalar-on-scalar conflict: `settingsMergeCustomizer`
  (`settings.ts:538-547`) returns `undefined` for non-arrays, so
  lodash `mergeWith` default applies — the later (project) value
  overwrites; destination-only keys (`verbose`, `cleanupPeriodDays`)
  are retained.

Expected values:

| key | winner | why |
|---|---|---|
| `theme` | project (`"light"`) | later source overwrites |
| `verbose` | user (`true`) | absent in project — retained |
| `cleanupPeriodDays` | user (`7`) | absent in project — retained |
| `diffTool` | project (`"delta"`) | only project defines it |

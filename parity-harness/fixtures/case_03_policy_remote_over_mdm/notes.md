# case_03_policy_remote_over_mdm

Policy first-source-wins: a populated remote source wins and the MDM
source is never consulted; the winning policy object merges at the
highest precedence over the file-based layers.

Derived from claude-code@2.1.88 source:

- Policy block in `loadSettingsFromDisk`:
  `src/utils/settings/settings.ts:677-737`. Remote is checked first
  (`settings.ts:682-693`); once `policySettings` is set, every later
  branch is gated off by `if (!policySettings)` (`settings.ts:696`,
  `:705`, `:714`), so `mdm_admin` never wins here.
- The winning policy object is merged via `mergeWith(...,
  settingsMergeCustomizer)` at `settings.ts:723-729`. Because
  `policySettings` is the LAST entry in `SETTING_SOURCES`
  (`src/utils/settings/constants.ts:7-21`), it overrides user and
  project at shared keys.

Expected values:

| key | winner | why |
|---|---|---|
| `theme` | policy remote (`"policy-remote"`) | remote wins first-source-wins, then overrides user/project |
| `permissions.defaultMode` | user (`"ask"`) | not set by policy — retained |
| `verbose` | project (`true`) | not set by policy — retained |

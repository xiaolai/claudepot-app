# case_10_policy_hkcu_winner

Policy fallthrough exhausts remote, MDM, and the managed-file
composite; the user-writable HKCU source — the lowest-priority policy
origin — wins, and still overrides the non-policy layers.

Derived from claude-code@2.1.88 source:

- HKCU branch: `src/utils/settings/settings.ts:714-719` — runs only
  when every higher policy branch produced nothing ("lowest —
  user-writable, only if nothing above exists"), and wins only when
  non-empty (`Object.keys(hkcu.settings).length > 0` at `:716`).
- The winner merges last (`settings.ts:723-729`; `policySettings` is
  last in `SETTING_SOURCES`, `src/utils/settings/constants.ts:7-21`),
  so HKCU's `theme` overrides the user settings file.

Expected values:

| key | winner | why |
|---|---|---|
| `theme` | HKCU (`"hkcu-theme"`) | last policy fallthrough, still beats user layer |
| `verbose` | user (`true`) | not set by policy — retained |

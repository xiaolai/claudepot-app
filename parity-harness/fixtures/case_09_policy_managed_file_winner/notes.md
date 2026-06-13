# case_09_policy_managed_file_winner

Policy fallthrough reaches the managed-file composite: remote and MDM
are absent, so `managed-settings.json` (+ drop-ins) wins, and the
lower-priority HKCU source is never consulted even though populated.

Derived from claude-code@2.1.88 source:

- Policy block: `src/utils/settings/settings.ts:677-737`. The
  managed-file branch (`settings.ts:705-712`) runs only when remote
  (`:682-693`) and MDM (`:696-701`) produced nothing.
- `loadManagedFileSettings` (`settings.ts:74-121`) returns a non-null
  object only when at least one non-empty source file was found (the
  `found` flag, gated by `Object.keys(settings).length > 0` at
  `:86-89` and `:108-111`) — so a populated composite wins here.
- HKCU is gated off by `if (!policySettings)` at `settings.ts:714` —
  it never runs once the managed file won.
- The winner merges last (`settings.ts:723-729`; `policySettings` is
  last in `SETTING_SOURCES`, `src/utils/settings/constants.ts:7-21`),
  so its `theme` overrides the user's.

Expected values:

| key | winner | why |
|---|---|---|
| `theme` | managed file (`"managed-theme"`) | composite wins fallthrough, overrides user |
| `verbose` | user (`true`) | not set by policy — retained |
| `permissions.defaultMode` | managed file (`"plan"`) | only policy defines it |

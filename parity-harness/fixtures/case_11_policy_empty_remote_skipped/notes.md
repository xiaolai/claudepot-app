# case_11_policy_empty_remote_skipped

A remote policy source that is PRESENT but EMPTY (`{}`) does not win
first-source-wins — fallthrough continues to the populated MDM
source. Distinguishes "present but empty" from "missing": both fall
through, neither errors.

Derived from claude-code@2.1.88 source:

- Remote gate: `src/utils/settings/settings.ts:683` —
  `if (remoteSettings && Object.keys(remoteSettings).length > 0)`.
  An empty object fails the length check, so `policySettings` stays
  null and no error is recorded.
- MDM branch then runs (`settings.ts:696-701`) and wins because its
  settings object is non-empty
  (`Object.keys(mdmResult.settings).length > 0` at `:698`).
- The MDM winner merges last (`settings.ts:723-729`), overriding the
  user layer's `theme`.

Expected values:

| key | winner | why |
|---|---|---|
| `theme` | MDM (`"mdm-theme"`) | empty remote skipped; MDM wins and overrides user |
| `permissions.defaultMode` | MDM (`"acceptEdits"`) | only MDM defines it |

# case_04_plugin_base_agent

Plugin settings are the lowest-precedence base; a project layer
deep-merges over them, overriding one nested key while retaining the
plugin's siblings.

Derived from claude-code@2.1.88 source:

- Plugin settings merged first, before the source loop:
  `src/utils/settings/settings.ts:659-667` (`getPluginSettingsBase()`
  merged into the empty accumulator with `settingsMergeCustomizer` —
  "Start with plugin settings as the lowest priority base").
- Project deep merge: `settingsMergeCustomizer`
  (`settings.ts:538-547`) returns `undefined` for object-on-object,
  so lodash `mergeWith` recurses: `agent.model` is overwritten by
  project, `agent.tools` (absent in project) is retained from the
  plugin base.

Expected values:

| key | winner | why |
|---|---|---|
| `agent.model` | project (`"sonnet"`) | deep merge, later overwrites |
| `agent.tools` | plugin base (`["Bash"]`) | absent above — retained |
| `theme` | user (`"dark"`) | only user defines it |

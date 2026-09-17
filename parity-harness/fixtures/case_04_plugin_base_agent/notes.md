# case_04_plugin_base_agent

Plugin settings are the lowest-precedence base, below every file layer.

Verified: claude-code@2.1.274 by parity-harness/dump.ts (2026-09-17).

A plugin's `settings.json` is cut down to an allowlist before it joins
the merge. On 2.1.274 that allowlist is `["agent", "subagentStatusLine"]`
(`SE().pick(…).strip()` over `rWe` in the plugin loader) — anything else
a plugin ships, `theme` included, is dropped. `agent` is a string in the
current schema, so the old version of this fixture, which gave it an
object and deep-merged into it, was rejected outright. The fixture's
`plugin_base` is the layer *after* that cut, which is what
`effective_settings::compute_raw` takes; the cut itself is tested in
`config_view::plugin_base`.

| key | winner | why |
|---|---|---|
| `agent` | project (`"project-agent"`) | a file layer overrides the plugin base |
| `subagentStatusLine` | plugin base | nothing above sets it |
| `theme` | user (`"dark"`) | only user defines it |

//! Help template catalog — see `dev-docs/activity-cards-design.md` §6.
//!
//! Each template is a `(template_id, render_fn)` entry. The render
//! function receives the card's `HelpRef.args` map and returns the
//! final user-facing English. Keeping the text out of SQLite makes
//! catalog edits a code change, not a data migration — and avoids
//! the encoded-backslash pain of JSON template strings.
//!
//! New templates are added here. The classifier (in `classifier.rs`)
//! decides *which* template a given JSONL line maps to; this file
//! decides *what each template renders to*.
//!
//! v1 ships exactly one template (`hook.plugin_missing`). The other
//! eight from the design doc land in Phase 2 — by then we have the
//! full classifier rule set and can add their match conditions and
//! render lines together.

use super::card::HelpRef;

/// Render a help reference to its final English string.
///
/// Returns `None` for unknown template ids — never panics, never
/// fabricates a fallback. A card whose template_id is missing from
/// the catalog renders without a help line, which is the correct
/// behavior on a version mismatch (an older binary reading rows
/// written by a newer one).
pub fn render(help: &HelpRef) -> Option<String> {
    match help.template_id.as_str() {
        "hook.plugin_missing" => Some(render_plugin_missing(help)),
        _ => None,
    }
}

/// `hook.plugin_missing` — the single most common hook failure on the
/// reference machine (216 instances out of ~2000 historical failures).
///
/// Args:
/// - `plugin` — the plugin name extracted from the stderr message.
///   Falls back to `"the plugin"` when extraction failed.
fn render_plugin_missing(help: &HelpRef) -> String {
    let plugin = help
        .args
        .get("plugin")
        .map(String::as_str)
        .unwrap_or("the plugin");
    format!("Plugin {plugin} is missing. Run /plugin and reinstall.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn href(id: &str, args: &[(&str, &str)]) -> HelpRef {
        HelpRef {
            template_id: id.to_string(),
            args: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn unknown_template_returns_none_not_placeholder() {
        // A future writer may persist a card with a template_id this
        // older binary doesn't know about. The right answer is None
        // (= no help line shown), not a "[unknown template]" string.
        let h = href("hook.future_thing_we_dont_have_yet", &[]);
        assert_eq!(render(&h), None);
    }

    #[test]
    fn plugin_missing_renders_with_plugin_name() {
        let h = href(
            "hook.plugin_missing",
            &[("plugin", "mermaid-preview@xiaolai")],
        );
        assert_eq!(
            render(&h).as_deref(),
            Some("Plugin mermaid-preview@xiaolai is missing. Run /plugin and reinstall.")
        );
    }

    #[test]
    fn plugin_missing_renders_without_plugin_name() {
        // Falls back to a neutral phrasing — never panics, never
        // shows raw "{plugin}" placeholder text to the user.
        let h = href("hook.plugin_missing", &[]);
        assert_eq!(
            render(&h).as_deref(),
            Some("Plugin the plugin is missing. Run /plugin and reinstall.")
        );
    }
}

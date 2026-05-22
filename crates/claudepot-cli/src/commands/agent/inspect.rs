//! Read-only `agent list` / `agent show` verbs.
//!
//! Both are pure introspection over `~/.claudepot/agents.json` —
//! they never mutate the store, never touch a scheduler artifact.
//! They are grouped in one file because they share the same
//! read-the-store-and-format shape; splitting them would fragment
//! closely-related code (see `rules/commands.md`).

use anyhow::{Context, Result};
use claudepot_core::agent::AgentStore;
use uuid::Uuid;

use super::{agent_to_json, trigger_summary};

/// Run `agent list` — print every agent with id, name, lifecycle,
/// and a one-line trigger summary.
pub fn list_cmd(json: bool) -> Result<()> {
    let store = AgentStore::open().context("opening the agent store")?;
    let agents = store.list();

    if json {
        let arr: Vec<serde_json::Value> = agents.iter().map(agent_to_json).collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if agents.is_empty() {
        println!("No agents.\n\nAn AI client can propose one with `claudepot agent draft`;\nyou install it from Claudepot > Agents.");
        return Ok(());
    }

    let mut out = String::new();
    out.push_str(&format!(
        "  {:<38}  {:<24}  {:<10}  {}\n",
        "ID", "NAME", "LIFECYCLE", "TRIGGER"
    ));
    out.push_str(&format!(
        "  {:<38}  {:<24}  {:<10}  {}\n",
        "─".repeat(36),
        "─".repeat(22),
        "─".repeat(9),
        "───────"
    ));
    for a in agents {
        let lifecycle = match a.lifecycle {
            claudepot_core::agent::Lifecycle::Draft => "draft",
            claudepot_core::agent::Lifecycle::Installed => "installed",
        };
        out.push_str(&format!(
            "  {:<38}  {:<24}  {:<10}  {}\n",
            a.id,
            truncate(&a.name, 24),
            lifecycle,
            trigger_summary(a),
        ));
    }
    out.push_str(&format!("\n{} agent(s).", agents.len()));
    println!("{out}");
    Ok(())
}

/// Run `agent show <id>` — print one agent's full spec. `id` may be
/// an agent UUID or a name (resolved via the store's name index).
pub fn show_cmd(json: bool, id_or_name: &str) -> Result<()> {
    let store = AgentStore::open().context("opening the agent store")?;
    let target = id_or_name.trim();

    // Accept either a UUID or a name — `show` is a human-driven
    // verb and names are easier to type than uuids.
    let agent = match Uuid::parse_str(target) {
        Ok(uuid) => store.get(&uuid),
        Err(_) => store.get_by_name(target),
    }
    .with_context(|| format!("no agent matching '{id_or_name}'"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&agent_to_json(agent))?);
        return Ok(());
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Agent: {}\n",
        agent.display_name.as_deref().unwrap_or(&agent.name)
    ));
    out.push_str(&format!("  id:           {}\n", agent.id));
    out.push_str(&format!("  name:         {}\n", agent.name));
    let lifecycle = match agent.lifecycle {
        claudepot_core::agent::Lifecycle::Draft => "draft (inert — install in the GUI to arm)",
        claudepot_core::agent::Lifecycle::Installed => "installed",
    };
    out.push_str(&format!("  lifecycle:    {lifecycle}\n"));
    if let Some(by) = &agent.drafted_by {
        out.push_str(&format!("  drafted-by:   {by}\n"));
    }
    out.push_str(&format!("  enabled:      {}\n", agent.enabled));
    if let Some(d) = &agent.description {
        out.push_str(&format!("  description:  {d}\n"));
    }
    out.push_str(&format!(
        "  model:        {}\n",
        agent.model.as_deref().unwrap_or("(CLI default)")
    ));
    out.push_str(&format!("  cwd:          {}\n", agent.cwd));
    out.push_str(&format!("  trigger:      {}\n", trigger_summary(agent)));
    out.push_str(&format!(
        "  permissions:  {}\n",
        agent.permission_mode.as_cli_flag()
    ));
    if !agent.allowed_tools.is_empty() {
        out.push_str(&format!(
            "  allow-tools:  {}\n",
            agent.allowed_tools.join(", ")
        ));
    }
    if !agent.disallowed_tools.is_empty() {
        out.push_str(&format!(
            "  deny-tools:   {}\n",
            agent.disallowed_tools.join(", ")
        ));
    }
    if !agent.mcp_servers.is_empty() {
        let names: Vec<String> = agent
            .mcp_servers
            .iter()
            .map(|m| match m {
                claudepot_core::agent::McpServerRef::ClaudepotMemory => {
                    "claudepot-memory".to_string()
                }
                claudepot_core::agent::McpServerRef::Custom { name, .. } => name.clone(),
            })
            .collect();
        out.push_str(&format!("  mcp-servers:  {}\n", names.join(", ")));
    }
    if let Some(ra) = &agent.run_as {
        out.push_str(&format!("  run-as:       {ra}\n"));
    }
    if let Some(tb) = agent.task_budget {
        out.push_str(&format!("  task-budget:  {tb} tokens/run\n"));
    }
    if let Some(rl) = &agent.rate_limit {
        let mut parts = Vec::new();
        if let Some(i) = rl.min_interval_secs {
            parts.push(format!("min {i}s between runs"));
        }
        if let Some(d) = rl.max_per_day {
            parts.push(format!("max {d}/day"));
        }
        if !parts.is_empty() {
            out.push_str(&format!("  rate-limit:   {}\n", parts.join(", ")));
        }
    }
    out.push_str(&format!("  output:       {}\n", agent.output_format.as_cli_flag()));
    out.push_str("  prompt:\n");
    for line in agent.prompt.lines() {
        out.push_str(&format!("    {line}\n"));
    }
    print!("{out}");
    Ok(())
}

/// Truncate a string to `max` chars, appending an ellipsis when cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_leaves_short_strings() {
        assert_eq!(truncate("short", 24), "short");
    }

    #[test]
    fn truncate_cuts_long_strings_with_ellipsis() {
        let got = truncate("aaaaaaaaaaaaaaaaaaaaaaaaaaaa", 10);
        assert_eq!(got.chars().count(), 10);
        assert!(got.ends_with('…'));
    }
}

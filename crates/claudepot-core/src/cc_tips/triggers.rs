//! Plain-English trigger summaries + default shortcut bindings.
//!
//! Hand-authored from inspection of each tip's `isRelevant` body in
//! CC source. New tip ids that aren't in this table fall back to a
//! generic "shows occasionally" string, with the raw `isRelevant`
//! source surfaced in the UI under a "Show details" disclosure.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerInfo {
    pub summary: &'static str,
    /// Optional GrowthBook flag name when the tip is gated by an
    /// active experiment. Mirrors `RawTip.experiment_flag` but as a
    /// hand-curated annotation in case extraction misses it.
    pub experiment: Option<&'static str>,
}

const TRIGGERS: &[(&str, TriggerInfo)] = &[
    (
        "new-user-warmup",
        TriggerInfo {
            summary: "First 10 CC sessions",
            experiment: None,
        },
    ),
    (
        "plan-mode-for-complex-tasks",
        TriggerInfo {
            summary: "Plan mode unused for 7+ days",
            experiment: None,
        },
    ),
    (
        "default-permission-mode-config",
        TriggerInfo {
            summary: "Used plan mode but no default mode set",
            experiment: None,
        },
    ),
    (
        "git-worktrees",
        TriggerInfo {
            summary: "After 50 sessions, no worktree configured",
            experiment: None,
        },
    ),
    (
        "color-when-multi-clauding",
        TriggerInfo {
            summary: "2+ concurrent CC sessions, no color set",
            experiment: None,
        },
    ),
    (
        "terminal-setup",
        TriggerInfo {
            summary: "OS keybinding helper not installed",
            experiment: None,
        },
    ),
    (
        "shift-enter",
        TriggerInfo {
            summary: "Multi-line keybinding installed, after 3 sessions",
            experiment: None,
        },
    ),
    (
        "shift-enter-setup",
        TriggerInfo {
            summary: "Multi-line keybinding helper not installed",
            experiment: None,
        },
    ),
    (
        "memory-command",
        TriggerInfo {
            summary: "Never used /memory",
            experiment: None,
        },
    ),
    (
        "theme-command",
        TriggerInfo {
            summary: "Always (low priority, 20-session cooldown)",
            experiment: None,
        },
    ),
    (
        "colorterm-truecolor",
        TriggerInfo {
            summary: "Terminal lacks truecolor support",
            experiment: None,
        },
    ),
    (
        "powershell-tool-env",
        TriggerInfo {
            summary: "Windows + PowerShell tool env unset",
            experiment: None,
        },
    ),
    (
        "status-line",
        TriggerInfo {
            summary: "No custom status line configured",
            experiment: None,
        },
    ),
    (
        "prompt-queue",
        TriggerInfo {
            summary: "Used queue feature fewer than 3 times",
            experiment: None,
        },
    ),
    (
        "enter-to-steer-in-relatime",
        TriggerInfo {
            summary: "Always",
            experiment: None,
        },
    ),
    (
        "todo-list",
        TriggerInfo {
            summary: "Always",
            experiment: None,
        },
    ),
    (
        "vscode-command-install",
        TriggerInfo {
            summary: "macOS + VS Code-style terminal, command not in PATH",
            experiment: None,
        },
    ),
    (
        "ide-upsell-external-terminal",
        TriggerInfo {
            summary: "Running outside a supported IDE terminal but an IDE is open",
            experiment: None,
        },
    ),
    (
        "install-github-app",
        TriggerInfo {
            summary: "Never set up GitHub integration",
            experiment: None,
        },
    ),
    (
        "install-slack-app",
        TriggerInfo {
            summary: "Never set up Slack integration",
            experiment: None,
        },
    ),
    (
        "permissions",
        TriggerInfo {
            summary: "After 10 sessions",
            experiment: None,
        },
    ),
    (
        "drag-and-drop-images",
        TriggerInfo {
            summary: "Not in an SSH session",
            experiment: None,
        },
    ),
    (
        "paste-images-mac",
        TriggerInfo {
            summary: "macOS only",
            experiment: None,
        },
    ),
    (
        "double-esc",
        TriggerInfo {
            summary: "File history disabled",
            experiment: None,
        },
    ),
    (
        "double-esc-code-restore",
        TriggerInfo {
            summary: "File history enabled",
            experiment: None,
        },
    ),
    (
        "continue",
        TriggerInfo {
            summary: "Always",
            experiment: None,
        },
    ),
    (
        "rename-conversation",
        TriggerInfo {
            summary: "Custom titles enabled, after 10 sessions",
            experiment: None,
        },
    ),
    (
        "custom-commands",
        TriggerInfo {
            summary: "After 10 sessions",
            experiment: None,
        },
    ),
    (
        "shift-tab",
        TriggerInfo {
            summary: "Always",
            experiment: None,
        },
    ),
    (
        "image-paste",
        TriggerInfo {
            summary: "Always",
            experiment: None,
        },
    ),
    (
        "custom-agents",
        TriggerInfo {
            summary: "After 5 sessions",
            experiment: None,
        },
    ),
    (
        "agent-flag",
        TriggerInfo {
            summary: "After 5 sessions",
            experiment: None,
        },
    ),
    (
        "desktop-app",
        TriggerInfo {
            summary: "Not Linux",
            experiment: None,
        },
    ),
    (
        "desktop-shortcut",
        TriggerInfo {
            summary: "macOS or Windows x64, upsell flag enabled",
            experiment: None,
        },
    ),
    (
        "web-app",
        TriggerInfo {
            summary: "Always",
            experiment: None,
        },
    ),
    (
        "mobile-app",
        TriggerInfo {
            summary: "Always",
            experiment: None,
        },
    ),
    (
        "remote-control",
        TriggerInfo {
            summary: "Always",
            experiment: None,
        },
    ),
    (
        "push-notif",
        TriggerInfo {
            summary: "Long tasks finished, no push setup",
            experiment: None,
        },
    ),
    (
        "voice-mode",
        TriggerInfo {
            summary: "Voice not configured",
            experiment: None,
        },
    ),
    (
        "no-flicker",
        TriggerInfo {
            summary: "Renderer setting opt-in available",
            experiment: None,
        },
    ),
    (
        "opusplan-mode-reminder",
        TriggerInfo {
            summary: "Default model is opusplan, plan mode unused 3+ days",
            experiment: None,
        },
    ),
    (
        "frontend-design-plugin",
        TriggerInfo {
            summary: "Recently read .html / .css / .htm files",
            experiment: None,
        },
    ),
    (
        "vercel-plugin",
        TriggerInfo {
            summary: "Recently used `vercel` CLI or read vercel.json",
            experiment: None,
        },
    ),
    (
        "effort-high-nudge",
        TriggerInfo {
            summary: "1P API customer, model supports effort",
            experiment: Some("tengu_tide_elm"),
        },
    ),
    (
        "subagent-fanout-nudge",
        TriggerInfo {
            summary: "1P API customer",
            experiment: Some("tengu_tern_alloy"),
        },
    ),
    (
        "loop-command-nudge",
        TriggerInfo {
            summary: "1P API customer, kairos cron enabled",
            experiment: Some("tengu_timber_lark"),
        },
    ),
    (
        "guest-passes",
        TriggerInfo {
            summary: "Eligible for referral rewards, hasn't visited /passes",
            experiment: None,
        },
    ),
    (
        "overage-credit",
        TriggerInfo {
            summary: "Eligible for overage credit upsell",
            experiment: None,
        },
    ),
    (
        "feedback-command",
        TriggerInfo {
            summary: "After 5 sessions, not Anthropic-internal",
            experiment: None,
        },
    ),
    (
        "important-claudemd",
        TriggerInfo {
            summary: "Anthropic-internal users only",
            experiment: None,
        },
    ),
    (
        "skillify",
        TriggerInfo {
            summary: "Anthropic-internal users only",
            experiment: None,
        },
    ),
    (
        "btw-side-question",
        TriggerInfo {
            summary: "Spinner active 30+ seconds, never used /btw",
            experiment: None,
        },
    ),
    (
        "clear-stale-context",
        TriggerInfo {
            summary: "Spinner active 30+ minutes",
            experiment: None,
        },
    ),
];

pub fn trigger_for(id: &str) -> TriggerInfo {
    TRIGGERS
        .iter()
        .find_map(|(k, v)| if *k == id { Some(v.clone()) } else { None })
        .unwrap_or(TriggerInfo {
            summary: "Shows occasionally",
            experiment: None,
        })
}

/// Return the count of known IDs (for partial-catalog detection).
pub fn known_id_count() -> usize {
    TRIGGERS.len()
}

/// Default shortcut bindings used when the user hasn't customized
/// their keybindings. Used to render `${Mf("chat:cycleMode","Chat",
/// "shift+tab")}` as `Shift+Tab`.
pub fn default_shortcut(key: &str) -> &'static str {
    match key {
        "chat:cycleMode" => "Shift+Tab",
        "chat:imagePaste" => "Ctrl+V",
        "chat:newLine" => "Shift+Enter",
        _ => "",
    }
}

//! Tip → category map. Hand-classified from CC source inspection.
//!
//! New tip ids that appear in the registry and aren't in this table
//! render as `Category::Misc` and surface a developer log line.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Onboarding,
    Workflow,
    Shortcut,
    Setup,
    MemoryConfig,
    MultiSession,
    Ide,
    AppsExtensions,
    Plugins,
    Experiments,
    Billing,
    Misc,
    Internal,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Onboarding => "Onboarding",
            Category::Workflow => "Workflow",
            Category::Shortcut => "Shortcut",
            Category::Setup => "Setup",
            Category::MemoryConfig => "Memory & Config",
            Category::MultiSession => "Multi-session",
            Category::Ide => "IDE",
            Category::AppsExtensions => "Apps & Extensions",
            Category::Plugins => "Plugins",
            Category::Experiments => "Experiments",
            Category::Billing => "Billing",
            Category::Misc => "Misc",
            Category::Internal => "Internal",
        }
    }
}

/// Static map. Adding a new tip = add a row here + a row in
/// `triggers::TRIGGERS`.
pub const CATEGORIES: &[(&str, Category)] = &[
    ("new-user-warmup", Category::Onboarding),
    ("plan-mode-for-complex-tasks", Category::Workflow),
    ("default-permission-mode-config", Category::Workflow),
    ("git-worktrees", Category::MultiSession),
    ("color-when-multi-clauding", Category::MultiSession),
    ("terminal-setup", Category::Setup),
    ("shift-enter", Category::Shortcut),
    ("shift-enter-setup", Category::Setup),
    ("memory-command", Category::MemoryConfig),
    ("theme-command", Category::MemoryConfig),
    ("colorterm-truecolor", Category::Setup),
    ("powershell-tool-env", Category::Setup),
    ("status-line", Category::MemoryConfig),
    ("prompt-queue", Category::Workflow),
    ("enter-to-steer-in-relatime", Category::Workflow),
    ("todo-list", Category::Workflow),
    ("vscode-command-install", Category::Ide),
    ("ide-upsell-external-terminal", Category::Ide),
    ("install-github-app", Category::AppsExtensions),
    ("install-slack-app", Category::AppsExtensions),
    ("permissions", Category::MemoryConfig),
    ("drag-and-drop-images", Category::Shortcut),
    ("paste-images-mac", Category::Shortcut),
    ("double-esc", Category::Workflow),
    ("double-esc-code-restore", Category::Workflow),
    ("continue", Category::Workflow),
    ("rename-conversation", Category::Workflow),
    ("custom-commands", Category::Workflow),
    ("shift-tab", Category::Shortcut),
    ("image-paste", Category::Shortcut),
    ("custom-agents", Category::Workflow),
    ("agent-flag", Category::Workflow),
    ("desktop-app", Category::AppsExtensions),
    ("desktop-shortcut", Category::AppsExtensions),
    ("web-app", Category::AppsExtensions),
    ("mobile-app", Category::AppsExtensions),
    ("remote-control", Category::AppsExtensions),
    ("push-notif", Category::AppsExtensions),
    ("voice-mode", Category::Workflow),
    ("no-flicker", Category::Setup),
    ("opusplan-mode-reminder", Category::Workflow),
    ("frontend-design-plugin", Category::Plugins),
    ("vercel-plugin", Category::Plugins),
    ("effort-high-nudge", Category::Experiments),
    ("subagent-fanout-nudge", Category::Experiments),
    ("loop-command-nudge", Category::Experiments),
    ("guest-passes", Category::Billing),
    ("overage-credit", Category::Billing),
    ("feedback-command", Category::Misc),
    ("important-claudemd", Category::Internal),
    ("skillify", Category::Internal),
    // Spinner-override tips (not in registry; ids only seen in
    // tipsHistory). Prose is bundled below in `spinner_override_prose`.
    ("btw-side-question", Category::Workflow),
    ("clear-stale-context", Category::Workflow),
];

pub fn category_for(id: &str) -> Category {
    CATEGORIES
        .iter()
        .find_map(|(k, v)| if *k == id { Some(*v) } else { None })
        .unwrap_or(Category::Misc)
}

/// Spinner-override tip ids fire from `Spinner.tsx` directly, not
/// from `tipRegistry.ts`. Their prose is hand-mirrored here so the
/// ledger can render them when they show up in the user's
/// `tipsHistory` map. Fewer than 5 entries; updating them when CC
/// changes is part of the integration-test drift detection.
pub fn spinner_override_prose(id: &str) -> Option<&'static str> {
    match id {
        "btw-side-question" => {
            Some("Use /btw to ask a quick side question without interrupting Claude's current work")
        }
        "clear-stale-context" => {
            Some("Use /clear to start fresh when switching topics and free up context")
        }
        _ => None,
    }
}

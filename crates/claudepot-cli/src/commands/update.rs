//! `claudepot update …` verb-group module.
//!
//! Public verbs (re-exported below for `main.rs`'s match block):
//! - **check** — detect installs + probe upstream for latest versions
//! - **cli** — force-run `claude update` (CC's own updater)
//! - **desktop** — drive Desktop install (brew or direct .zip)
//! - **config** — show or modify update settings
//!
//! Per `.claude/rules/commands.md`, nouns with ≥3 verbs live in a
//! directory module. This entry file holds shared formatters + the
//! submodule declarations; verbs themselves are in sibling files.
//! All handlers are thin wrappers around `claudepot_core::updates`
//! — no business logic here, per `.claude/rules/architecture.md`.

pub mod check;
pub mod cli;
pub mod config;
pub mod desktop;

use claudepot_core::updates::{
    detect_cli_installs, detect_desktop_install, is_desktop_running, CliInstall, DesktopInstall,
};

pub(crate) fn cli_install_summary(c: &CliInstall) -> String {
    let active = if c.is_active { "active" } else { "inactive" };
    let auto = if c.auto_updates {
        "auto-updates"
    } else {
        "manual"
    };
    let v = c.version.as_deref().unwrap_or("?");
    format!(
        "{} {} ({}, {}, {})",
        c.binary_path.display(),
        v,
        c.kind.label(),
        active,
        auto
    )
}

pub(crate) fn desktop_install_summary(d: &DesktopInstall) -> String {
    let v = d.version.as_deref().unwrap_or("?");
    let manageable = if d.manageable {
        "manageable"
    } else {
        "managed elsewhere"
    };
    format!(
        "{} {} ({}, {})",
        d.app_path.display(),
        v,
        d.source.label(),
        manageable
    )
}

pub(crate) struct StatusSnapshot {
    pub cli_installs: Vec<CliInstall>,
    pub desktop: Option<DesktopInstall>,
    pub desktop_running: bool,
}

pub(crate) fn collect_status() -> StatusSnapshot {
    StatusSnapshot {
        cli_installs: detect_cli_installs(),
        desktop: detect_desktop_install(),
        desktop_running: is_desktop_running(),
    }
}

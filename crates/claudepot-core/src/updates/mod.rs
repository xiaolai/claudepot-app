//! Auto-updates for CC CLI and Claude Desktop.
//!
//! See `dev-docs/auto-updates.md` for the full design rationale.
//!
//! Public surface:
//! - [`detect`] — enumerate installs, classify source, count locks
//! - [`version`] — fetch latest from upstream, compare versions
//! - [`settings_bridge`] — read/write CC's `~/.claude/settings.json`
//! - [`state`] — persisted user preferences + cached probe results
//! - [`cli_driver`] — drive `claude update`
//! - [`desktop_driver`] — drive `brew upgrade` / direct DMG install

pub mod cli_driver;
pub mod desktop_driver;
pub mod detect;
pub mod errors;
pub mod poller;
pub mod settings_bridge;
pub mod state;
pub mod version;

pub use detect::{
    count_running_cli_locks, detect_cli_installs, detect_desktop_install, is_desktop_running,
    CliInstall, CliInstallKind, DesktopInstall, DesktopSource,
};
pub use errors::{Result, UpdateError};
pub use poller::{
    run_one_check_cycle, save_state, AutoInstallOutcome, CheckCycleOutcome, PollerGate,
};
pub use state::{
    CliCache, CliSettings, DesktopCache, DesktopSettings, UpdateCache, UpdateSettings, UpdateState,
    UpdateStateMutex,
};
pub use version::{
    compare_versions, fetch_cli_latest, fetch_desktop_latest, Channel, DesktopRelease,
};

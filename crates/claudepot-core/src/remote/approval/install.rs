//! Wiring the approval hook into Claude Code's settings, and taking it
//! back out.
//!
//! **Installation is coupled to the remote surface's own switch.** The
//! ability to approve a tool call from a phone exists only while the
//! thing that reaches the phone is switched on, so `remote serve`
//! installs on start and removes on stop. That coupling is the reason
//! this feature is acceptable at all: it does not quietly widen what
//! the machine will do on an install that never turns remote on.
//!
//! It is the *outer* half of the gate, and the weaker one — settings
//! files are hand-edited, apps are force-quit, and an entry can outlive
//! the process that wrote it. [`super::store::gate`] is the half that
//! holds when this one has failed.
//!
//! The entry itself — exec form, verb-matched, mutex-written — is
//! [`crate::cc_hook_entry`], shared with the permission-grant hook.
//! Two Claudepot hooks in one file must be able to come and go
//! independently: this one leaves with `remote serve`, that one with
//! the last live grant, and neither may take the other with it.

use std::path::Path;

use crate::cc_hook_entry::{self, HookSpec};
use crate::settings_mutex::Mutation;

pub use crate::cc_hook_entry::InstallError;

/// The event we hook. CC fires it exactly when it is about to draw a
/// permission prompt — unlike `PreToolUse`, which fires on every call
/// including ones already allowed, and would pause work nobody was
/// being asked about.
pub const SPEC: HookSpec = HookSpec {
    event: "PermissionRequest",
    verb: &["hook", "permission-request"],
    timeout_secs: super::HOOK_TIMEOUT_SECS,
};

/// Add the hook, replacing any earlier copy of ours. Idempotent.
pub fn install(binary: &Path) -> Result<Mutation<()>, InstallError> {
    cc_hook_entry::install(&SPEC, binary)
}

/// Take the hook back out. Safe to call when it was never installed.
pub fn uninstall() -> Result<Mutation<()>, InstallError> {
    cc_hook_entry::uninstall(&SPEC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_names_the_prompt_event_and_the_hidden_verb() {
        assert_eq!(SPEC.event, "PermissionRequest");
        assert_eq!(SPEC.verb, &["hook", "permission-request"]);
    }

    #[test]
    fn the_installed_timeout_outlasts_the_wait() {
        assert_eq!(SPEC.timeout_secs, super::super::HOOK_TIMEOUT_SECS);
        assert!(super::super::WAIT.as_secs() < SPEC.timeout_secs);
    }
}

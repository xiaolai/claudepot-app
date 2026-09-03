//! CC's `permissions.defaultMode` value as a typed enum.
//!
//! Verified against CC's published settings reference
//! (`code.claude.com/docs/en/settings-reference#permissions-defaultmode`)
//! and the 2.1.259 binary (2026-09-03). The documented values are
//! `default` / `manual` / `acceptEdits` / `plan` / `auto` / `dontAsk`
//! / `bypassPermissions`. `auto` was once an internal, feature-flagged
//! mode CC never wrote to disk; it is now the built-in starting mode
//! on Pro, Max and Team plans and a value users set themselves, so it
//! has its own variant. Anything else (`bubble`, a future mode) is
//! [`PermissionMode::Unknown`] rather than rejected — forward-compat
//! with a newer CC.

use serde::{Deserialize, Serialize};

/// A value of CC's `permissions.defaultMode` setting.
///
/// Serializes to/from the exact wire strings CC uses. Unknown values
/// round-trip verbatim through [`PermissionMode::Unknown`] so a file
/// written by a newer CC is never clobbered by Claudepot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionMode {
    /// `default` — prompt for every non-allowlisted operation.
    ///
    /// CC v2.1.200+ labels this mode "Manual" in its own UI and
    /// accepts `manual` on disk as an alias; that spelling parses to
    /// [`PermissionMode::Manual`], which behaves identically.
    Default,
    /// `manual` — CC v2.1.200+ alias for [`PermissionMode::Default`],
    /// matching the label CC now shows in its own UI.
    ///
    /// Kept as its own variant rather than folded into `Default` so a
    /// settings file written with `manual` is never silently rewritten
    /// to `default` when Claudepot reverts a grant.
    ///
    /// Semantically the two are one mode: any check for "the mode that
    /// prompts" must accept both, so `== PermissionMode::Default` is a
    /// bug waiting to happen — match `Default | Manual`. Nothing in
    /// Claudepot makes that comparison today; every code path compares
    /// against the *granted* mode instead.
    ///
    /// Writing `manual` to a machine running a pre-2.1.200 CC would be
    /// an unrecognized value, so Claudepot only preserves the spelling
    /// it reads; it never introduces it.
    Manual,
    /// `acceptEdits` — auto-accept file edits, still prompt for the rest.
    AcceptEdits,
    /// `plan` — read-only planning mode.
    Plan,
    /// `auto` — a classifier answers prompts instead of the user.
    /// Like [`PermissionMode::BypassPermissions`], CC ignores it when
    /// it comes from a project-scope settings file (see
    /// `settings::PROJECT_SCOPE_IGNORES_SINCE`).
    Auto,
    /// `dontAsk` — suppress prompts for allowlisted ops without
    /// granting bypass.
    DontAsk,
    /// `bypassPermissions` — the elevated mode: no prompts at all.
    /// This is the only [`is_elevated`](Self::is_elevated) variant.
    BypassPermissions,
    /// Any other string (an internal mode such as `bubble`, or a
    /// future CC mode). Preserved verbatim so writes never corrupt it.
    Unknown(String),
}

impl PermissionMode {
    /// The wire string CC reads/writes for this mode.
    pub fn as_wire_str(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::Manual => "manual",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::DontAsk => "dontAsk",
            Self::BypassPermissions => "bypassPermissions",
            Self::Unknown(s) => s,
        }
    }

    /// Parse a wire string into a mode. Never fails — unrecognized
    /// strings become [`PermissionMode::Unknown`].
    pub fn from_wire_str(s: &str) -> Self {
        match s {
            "default" => Self::Default,
            "manual" => Self::Manual,
            "acceptEdits" => Self::AcceptEdits,
            "plan" => Self::Plan,
            "auto" => Self::Auto,
            "dontAsk" => Self::DontAsk,
            "bypassPermissions" => Self::BypassPermissions,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// True only for `bypassPermissions` — the mode that disables all
    /// permission prompts. The permission dashboard flags these loud.
    pub fn is_elevated(&self) -> bool {
        matches!(self, Self::BypassPermissions)
    }

    /// Whether this is a mode Claudepot recognizes. `false` for
    /// [`PermissionMode::Unknown`] — the UI shows it read-only.
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

impl std::fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

impl Serialize for PermissionMode {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_wire_str())
    }
}

impl<'de> Deserialize<'de> for PermissionMode {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        Ok(Self::from_wire_str(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_modes_round_trip_through_wire_str() {
        for m in [
            PermissionMode::Default,
            PermissionMode::Manual,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::Auto,
            PermissionMode::DontAsk,
            PermissionMode::BypassPermissions,
        ] {
            let wire = m.as_wire_str().to_string();
            assert_eq!(PermissionMode::from_wire_str(&wire), m);
        }
    }

    #[test]
    fn manual_is_a_known_mode_not_an_unknown_string() {
        // CC v2.1.200+ writes `manual` for what older versions wrote as
        // `default`. Treating it as Unknown made the Permissions pane
        // show such a project as unmanageable.
        let m = PermissionMode::from_wire_str("manual");
        assert_eq!(m, PermissionMode::Manual);
        assert!(m.is_known());
        assert!(!m.is_elevated());
    }

    #[test]
    fn manual_spelling_survives_a_round_trip() {
        // Revert writes `previous_mode` back verbatim. Collapsing
        // `manual` into `Default` would silently rewrite the user's
        // settings file to the other spelling.
        assert_eq!(PermissionMode::Manual.as_wire_str(), "manual");
        let json = serde_json::to_string(&PermissionMode::Manual).unwrap();
        assert_eq!(json, r#""manual""#);
        assert_eq!(
            serde_json::from_str::<PermissionMode>(&json).unwrap(),
            PermissionMode::Manual
        );
    }

    #[test]
    fn manual_and_default_stay_distinct_values() {
        // They mean the same thing to CC, but they are different
        // strings on disk and must not compare equal — equality is what
        // the revert path uses to decide whether the user took over.
        assert_ne!(PermissionMode::Manual, PermissionMode::Default);
    }

    #[test]
    fn unknown_mode_preserves_string_verbatim() {
        let m = PermissionMode::from_wire_str("bubble");
        assert_eq!(m, PermissionMode::Unknown("bubble".into()));
        assert_eq!(m.as_wire_str(), "bubble");
        assert!(!m.is_known());
    }

    #[test]
    fn auto_is_a_known_mode_since_it_is_a_documented_settings_value() {
        // `auto` is the built-in starting mode on paid plans and a value
        // this user's own `~/.claude/settings.json` carries. Reading it
        // as Unknown made the Permissions pane call a normal setup
        // unmanageable.
        let m = PermissionMode::from_wire_str("auto");
        assert_eq!(m, PermissionMode::Auto);
        assert!(m.is_known());
        assert!(!m.is_elevated());
        assert_eq!(serde_json::to_string(&m).unwrap(), r#""auto""#);
    }

    #[test]
    fn only_bypass_is_elevated() {
        assert!(PermissionMode::BypassPermissions.is_elevated());
        for m in [
            PermissionMode::Default,
            PermissionMode::Manual,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::Auto,
            PermissionMode::DontAsk,
            PermissionMode::Unknown("bubble".into()),
        ] {
            assert!(!m.is_elevated(), "{m} must not be elevated");
        }
    }

    #[test]
    fn serde_uses_wire_strings() {
        let json = serde_json::to_string(&PermissionMode::BypassPermissions).unwrap();
        assert_eq!(json, r#""bypassPermissions""#);
        let back: PermissionMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, PermissionMode::BypassPermissions);
    }

    #[test]
    fn serde_unknown_round_trips() {
        let m = PermissionMode::Unknown("bubble".into());
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, r#""bubble""#);
        assert_eq!(serde_json::from_str::<PermissionMode>(&json).unwrap(), m);
    }
}

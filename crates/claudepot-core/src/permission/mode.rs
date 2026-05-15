//! CC's `permissions.defaultMode` value as a typed enum.
//!
//! Verified against `~/github/claude_code_src/src/types/permissions.ts`
//! (`EXTERNAL_PERMISSION_MODES`). Only the *external* modes are
//! user-addressable in `settings.json`; `auto` / `bubble` are
//! internal and never written to disk by CC, so an on-disk file
//! carrying one is treated as [`PermissionMode::Unknown`] rather
//! than rejected — forward-compat with a newer CC.

use serde::{Deserialize, Serialize};

/// A value of CC's `permissions.defaultMode` setting.
///
/// Serializes to/from the exact wire strings CC uses. Unknown values
/// round-trip verbatim through [`PermissionMode::Unknown`] so a file
/// written by a newer CC is never clobbered by Claudepot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionMode {
    /// `default` — prompt for every non-allowlisted operation.
    Default,
    /// `acceptEdits` — auto-accept file edits, still prompt for the rest.
    AcceptEdits,
    /// `plan` — read-only planning mode.
    Plan,
    /// `dontAsk` — suppress prompts for allowlisted ops without
    /// granting bypass.
    DontAsk,
    /// `bypassPermissions` — the elevated mode: no prompts at all.
    /// This is the only [`is_elevated`](Self::is_elevated) variant.
    BypassPermissions,
    /// Any other string (e.g. a feature-flagged `auto`, or a future
    /// CC mode). Preserved verbatim so writes never corrupt it.
    Unknown(String),
}

impl PermissionMode {
    /// The wire string CC reads/writes for this mode.
    pub fn as_wire_str(&self) -> &str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
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
            "acceptEdits" => Self::AcceptEdits,
            "plan" => Self::Plan,
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
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::DontAsk,
            PermissionMode::BypassPermissions,
        ] {
            let wire = m.as_wire_str().to_string();
            assert_eq!(PermissionMode::from_wire_str(&wire), m);
        }
    }

    #[test]
    fn unknown_mode_preserves_string_verbatim() {
        let m = PermissionMode::from_wire_str("auto");
        assert_eq!(m, PermissionMode::Unknown("auto".into()));
        assert_eq!(m.as_wire_str(), "auto");
        assert!(!m.is_known());
    }

    #[test]
    fn only_bypass_is_elevated() {
        assert!(PermissionMode::BypassPermissions.is_elevated());
        for m in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::DontAsk,
            PermissionMode::Unknown("auto".into()),
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
        let m = PermissionMode::Unknown("auto".into());
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, r#""auto""#);
        assert_eq!(serde_json::from_str::<PermissionMode>(&json).unwrap(), m);
    }
}

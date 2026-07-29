//! Failure modes for [`crate::cc_env`].
//!
//! # Secrets never reach a message
//!
//! `rules/architecture.md` audits error `Display` impls for interpolated
//! secrets, and every variant here that could name a value routes it through
//! [`redacted`] first. That is not belt-and-braces: an error string is the one
//! thing that reliably ends up in a toast, a log line, and a bug report at the
//! same time.

use crate::cc_env::spec::{Blocked, EnvVarSpec};
use crate::settings_mutex::SettingsMutexError;
use std::path::PathBuf;

/// What a message is allowed to say about a value.
///
/// A rejected value belongs in the error — "12x is not a number" is only
/// actionable if it names `12x` — unless it might be a credential, in which
/// case it never appears anywhere, including in the complaint that it was
/// malformed.
///
/// Two independent gates, because either alone leaks. The variable's own
/// `secret` flag catches a credential in the field built for one. The content
/// scan catches the case the flag cannot see: a token pasted into a field
/// that is *not* credential-bearing — `MAX_THINKING_TOKENS = "sk-ant-…"` is
/// rejected as a non-integer, and `rules/rust-conventions.md` says never to
/// put a string starting with `sk-ant-` in an error message regardless of
/// which field it arrived in.
pub fn redacted(spec: &EnvVarSpec, value: &str) -> String {
    if spec.safety.secret || looks_like_a_credential(value) {
        "<redacted>".to_string()
    } else {
        value.to_string()
    }
}

/// Whether a value should never be echoed whatever field it came from.
///
/// Deliberately narrow and conservative: it defends the one family the
/// conventions name explicitly rather than guessing at entropy, which would
/// redact ordinary values and teach the reader to ignore `<redacted>`.
pub fn looks_like_a_credential(value: &str) -> bool {
    !crate::secret_patterns::sk_ant_scan_ranges(value).is_empty()
}

#[derive(Debug, thiserror::Error)]
pub enum CcEnvError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json parse: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("settings file is not a JSON object at {0}")]
    NotAJsonObject(PathBuf),

    /// `env` exists but is not an object — `null`, an array, a string. Never
    /// coerced: rewriting it would destroy whatever the user meant by it.
    #[error("`env` in {path} is a {found}, not an object — fix it by hand")]
    EnvNotAnObject { path: PathBuf, found: &'static str },

    /// A name that is in no documented list. `set` refuses these outright;
    /// `clear` accepts any name actually present in the map, which is what
    /// makes a hand-set key removable.
    #[error("{0} is not a documented Claude Code environment variable")]
    UnknownVariable(String),

    #[error("{name} cannot be edited here: {}", describe_blocked(*reason))]
    NotEditable { name: String, reason: Blocked },

    #[error("{name} accepts {}, not `{value}`", allowed.join(" or "))]
    InvalidEnumValue {
        name: String,
        value: String,
        allowed: Vec<String>,
    },

    /// Syntax only. The spec carries no bounds, and inventing them would
    /// reject values Claude Code accepts.
    #[error("{name} takes a whole number, not `{value}`")]
    InvalidNumber { name: String, value: String },

    #[error("{0} is not currently set, so there is nothing to clear")]
    NotSet(String),

    #[error("{path} is being written by something else; try again")]
    Contended { path: PathBuf },

    /// The compiled-in spec artifact is unusable. A build defect the
    /// `verify-docs` gate and the spec tests both catch, surfaced as an error
    /// rather than a panic per `rules/rust-conventions.md`.
    #[error("the embedded Claude Code env-var spec is unusable: {0}")]
    MalformedSpec(String),
}

fn describe_blocked(reason: Blocked) -> &'static str {
    match reason {
        Blocked::BootstrapSplitBrain => {
            "Claude Code reads it before settings load, so writing it here would \
             leave one run resolving paths against two different directories — \
             set it in your shell instead"
        }
        Blocked::HostInjected => {
            "it is injected per run by whatever launched Claude Code, so a value \
             written here would be overwritten immediately"
        }
    }
}

impl From<SettingsMutexError> for CcEnvError {
    fn from(e: SettingsMutexError) -> Self {
        match e {
            SettingsMutexError::Io(e) => Self::Io(e),
            SettingsMutexError::JsonParse(e) => Self::JsonParse(e),
            SettingsMutexError::NotAJsonObject(p) => Self::NotAJsonObject(p),
            SettingsMutexError::Contended { path, .. } => Self::Contended { path },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_env::spec::lookup;

    #[test]
    fn a_rejected_value_is_named_unless_the_variable_is_a_secret() {
        let plain = lookup("MAX_THINKING_TOKENS").unwrap();
        assert_eq!(redacted(plain, "12x"), "12x");

        let secret = lookup("ANTHROPIC_API_KEY").unwrap();
        assert_eq!(redacted(secret, "sk-ant-oat01-real"), "<redacted>");
        assert!(!redacted(secret, "sk-ant-oat01-real").contains("sk-ant"));
    }

    /// The gate the `secret` flag alone cannot close: a token pasted into a
    /// field that is not credential-bearing. `MAX_THINKING_TOKENS` rejects it
    /// as a non-integer, and that rejection must not quote it back.
    #[test]
    fn a_token_in_a_non_secret_field_is_still_never_echoed() {
        let plain = lookup("MAX_THINKING_TOKENS").unwrap();
        assert!(!plain.safety.secret);
        assert_eq!(redacted(plain, "sk-ant-oat01-pasted-here"), "<redacted>");

        let msg = CcEnvError::InvalidNumber {
            name: plain.name.clone(),
            value: redacted(plain, "sk-ant-oat01-pasted-here"),
        }
        .to_string();
        assert!(!msg.contains("sk-ant"), "{msg}");
    }

    #[test]
    fn credential_detection_does_not_fire_on_ordinary_values() {
        for ordinary in ["12x", "https://example.com", "opus", "", "0", "sk-"] {
            assert!(
                !looks_like_a_credential(ordinary),
                "{ordinary} should not read as a credential"
            );
        }
    }

    #[test]
    fn blocked_messages_say_what_to_do_instead() {
        let e = CcEnvError::NotEditable {
            name: "CLAUDE_CONFIG_DIR".into(),
            reason: Blocked::BootstrapSplitBrain,
        };
        assert!(e.to_string().contains("set it in your shell"));
    }
}

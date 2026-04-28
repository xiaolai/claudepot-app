//! Automation name validation.
//!
//! A name maps directly to a launchd label component, a systemd
//! unit-name fragment, and a Task Scheduler path component, so we
//! keep the rule narrow enough that no platform needs to escape:
//!
//! ```text
//! ^[a-z0-9][a-z0-9-]{0,62}[a-z0-9]$    OR a single [a-z0-9]
//! ```
//!
//! Lowercase ASCII alnum + dash, 1–64 chars, no leading or
//! trailing dash, no consecutive dashes. The single-character form
//! is allowed (`a` is valid; `-` is not).

use super::error::AutomationError;

const MAX_LEN: usize = 64;

/// Validate an automation name. Returns the trimmed name on
/// success. Does **not** check uniqueness — that's the store's job.
pub fn validate_name(input: &str) -> Result<String, AutomationError> {
    let name = input.trim();
    if name.is_empty() {
        return Err(AutomationError::InvalidName(
            input.to_string(),
            "name cannot be empty",
        ));
    }
    if name.len() > MAX_LEN {
        return Err(AutomationError::InvalidName(
            input.to_string(),
            "name exceeds 64 characters",
        ));
    }
    let bytes = name.as_bytes();
    if !is_alnum_lower(bytes[0]) {
        return Err(AutomationError::InvalidName(
            input.to_string(),
            "name must start with a-z or 0-9",
        ));
    }
    if !is_alnum_lower(bytes[bytes.len() - 1]) {
        return Err(AutomationError::InvalidName(
            input.to_string(),
            "name must end with a-z or 0-9",
        ));
    }
    let mut prev_dash = false;
    for &b in bytes {
        match b {
            b'-' => {
                if prev_dash {
                    return Err(AutomationError::InvalidName(
                        input.to_string(),
                        "consecutive dashes are not allowed",
                    ));
                }
                prev_dash = true;
            }
            b if is_alnum_lower(b) => prev_dash = false,
            _ => {
                return Err(AutomationError::InvalidName(
                    input.to_string(),
                    "only lowercase a-z, 0-9, and `-` are allowed",
                ))
            }
        }
    }
    Ok(name.to_string())
}

#[inline]
fn is_alnum_lower(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'0'..=b'9')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_shapes() {
        for ok in [
            "a",
            "ab",
            "morning-pr",
            "a1",
            "1a",
            "a-b-c",
            "release-notes-summary",
            "abcdef0123456789",
        ] {
            assert!(validate_name(ok).is_ok(), "expected ok: {ok:?}");
        }
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("\t").is_err());
    }

    #[test]
    fn rejects_uppercase_and_punct() {
        for bad in ["Morning", "morning_pr", "morning.pr", "morning/pr", "MORNING"] {
            assert!(validate_name(bad).is_err(), "expected err: {bad:?}");
        }
    }

    #[test]
    fn rejects_leading_or_trailing_dash() {
        assert!(validate_name("-foo").is_err());
        assert!(validate_name("foo-").is_err());
        assert!(validate_name("-").is_err());
    }

    #[test]
    fn rejects_consecutive_dashes() {
        assert!(validate_name("foo--bar").is_err());
        assert!(validate_name("a---b").is_err());
    }

    #[test]
    fn rejects_overlong() {
        let long = "a".repeat(65);
        assert!(validate_name(&long).is_err());
        let max = "a".repeat(64);
        assert!(validate_name(&max).is_ok());
    }

    #[test]
    fn trims_input_and_returns_trimmed() {
        assert_eq!(validate_name("  morning-pr  ").unwrap(), "morning-pr");
    }
}

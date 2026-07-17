/// The anthropic-beta header value required for OAuth endpoints.
///
/// Pinned value, verified against the installed `claude` binary on
/// 2026-04-12 (see reference.md §III.3 for where the binary carries
/// it). Deliberately NOT extracted at runtime — the value has been
/// stable across CC releases, and a parse of the minified binary is
/// far more fragile than a pin. Maintenance rule: if OAuth calls
/// start failing with header/beta errors, re-verify this value
/// against the current `claude` binary and bump the date here.
const DEFAULT_BETA_HEADER: &str = "oauth-2025-04-20";

/// Get the beta header value (the pinned default above).
pub fn get_or_default() -> &'static str {
    DEFAULT_BETA_HEADER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beta_header_not_empty() {
        let header = get_or_default();
        assert!(!header.is_empty());
    }

    #[test]
    fn test_beta_header_format() {
        let header = get_or_default();
        // Should be "oauth-YYYY-MM-DD" format
        assert!(header.starts_with("oauth-"));
        assert_eq!(header.len(), "oauth-2025-04-20".len());
    }
}

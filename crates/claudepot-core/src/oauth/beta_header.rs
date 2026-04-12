/// The anthropic-beta header value required for OAuth endpoints.
///
/// Hardcoded fallback. In production, this should be extracted from
/// the installed `claude` binary (see reference.md §III.3).
/// For now, use the known-good value verified on 2026-04-12.
const DEFAULT_BETA_HEADER: &str = "oauth-2025-04-20";

/// Get the beta header value. Currently returns the hardcoded default.
/// TODO: implement extraction from the claude binary (Step 4).
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

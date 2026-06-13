//! `add` verb — register a new account from current CC credentials,
//! a refresh token, or a browser OAuth login.
//!
//! Sub-module of `commands/account.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

pub async fn add(ctx: &AppContext, from_current: bool, from_token: Option<String>) -> Result<()> {
    use claudepot_core::services::account_service;

    let result = if from_current {
        ctx.info("Reading current CC credentials...");
        ctx.info("Fetching account profile...");
        account_service::register_from_current(&ctx.store).await?
    } else if let Some(token_arg) = from_token {
        let stdin_line = if token_arg == "-" {
            ctx.info("Reading refresh token from stdin...");
            let mut buf = String::new();
            std::io::stdin().read_line(&mut buf)?;
            buf
        } else {
            String::new()
        };
        let token =
            resolve_refresh_token(&token_arg, &stdin_line).map_err(|e| anyhow::anyhow!("{e}"))?;
        ctx.info("Exchanging refresh token...");
        ctx.info("Fetching account profile...");
        account_service::register_from_token(&ctx.store, &token).await?
    } else {
        return add_via_browser(ctx).await;
    };

    print_register_result(&result, ctx.json);
    Ok(())
}

/// Error produced by [`resolve_refresh_token`]. Kept as a small,
/// std-only type so unit tests can assert on the variant without
/// pulling in thiserror (per `.claude/rules/rust-conventions.md`,
/// thiserror is reserved for `claudepot-core`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TokenSourceError {
    /// Both the arg and the stdin fallback resolved to an empty token.
    Empty,
}

impl std::fmt::Display for TokenSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "refresh token is empty (nothing on stdin?)"),
        }
    }
}
impl std::error::Error for TokenSourceError {}

/// Pick the token source: when `arg == "-"`, trim and validate the
/// line read from stdin; otherwise trim and validate the arg. Empty
/// results surface a clear error instead of falling through to
/// `register_from_token` with a blank string.
pub(crate) fn resolve_refresh_token(
    arg: &str,
    stdin_line: &str,
) -> Result<String, TokenSourceError> {
    let source = if arg == "-" { stdin_line } else { arg };
    let trimmed = source.trim();
    if trimmed.is_empty() {
        Err(TokenSourceError::Empty)
    } else {
        Ok(trimmed.to_string())
    }
}

/// Browser-based add delegates to core's register_from_browser.
async fn add_via_browser(ctx: &AppContext) -> Result<()> {
    use claudepot_core::services::account_service;

    ctx.info("Opening browser for OAuth login...");
    ctx.info("(Complete the login in your browser)");

    let result = account_service::register_from_browser(&ctx.store).await?;
    print_register_result(&result, ctx.json);
    Ok(())
}

fn print_register_result(
    result: &claudepot_core::services::account_service::RegisterResult,
    json: bool,
) {
    if json {
        println!(
            "{}",
            serde_json::json!({
                "registered": true,
                "email": result.email,
                "org": result.org_name,
                "plan": result.subscription_type,
                "uuid": result.uuid.to_string(),
            })
        );
    } else {
        println!(
            "{}",
            format_register_human(
                &result.email,
                &result.subscription_type,
                result.rate_limit_tier.as_deref(),
            )
        );
    }
}

/// Produce the human-readable "Registered: …" line. Extracted as a
/// pure function so formatting edge cases (missing tier, missing
/// subscription, tier with no `_` separator) are unit-tested.
///
/// The `rate_limit_tier` received from `/profile` looks like
/// `rate_tier_20` — only the trailing segment after the last `_` is
/// user-facing ("20"). When the tier (or its trailing segment) is
/// empty, we omit it instead of emitting `(Max )` with a dangling
/// space, which the old implementation produced.
pub(crate) fn format_register_human(
    email: &str,
    subscription_type: &str,
    rate_limit_tier: Option<&str>,
) -> String {
    let sub = capitalize(subscription_type);
    let tier = rate_limit_tier
        .and_then(|t| t.split('_').next_back())
        .unwrap_or("")
        .trim();
    let plan = match (sub.is_empty(), tier.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(" ({sub})"),
        (true, false) => format!(" ({tier})"),
        (false, false) => format!(" ({sub} {tier})"),
    };
    format!("Registered: {email}{plan}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- resolve_refresh_token ------------------------------------

    #[test]
    fn test_resolve_token_arg_inline_returns_trimmed() {
        // Inline arg path: we accept the user's value verbatim but strip
        // trailing newline / stray whitespace that wrappers commonly add.
        assert_eq!(
            resolve_refresh_token("sk-ant-oat01-abc", "").unwrap(),
            "sk-ant-oat01-abc"
        );
        assert_eq!(
            resolve_refresh_token("  sk-ant-oat01-abc  ", "").unwrap(),
            "sk-ant-oat01-abc"
        );
    }

    #[test]
    fn test_resolve_token_stdin_trims_and_returns() {
        // The `-` sentinel routes to stdin; trailing newline MUST be
        // stripped because `stdin().read_line` keeps it.
        assert_eq!(
            resolve_refresh_token("-", "sk-ant-oat01-stdin\n").unwrap(),
            "sk-ant-oat01-stdin"
        );
        assert_eq!(
            resolve_refresh_token("-", "  sk-ant-oat01-stdin \n").unwrap(),
            "sk-ant-oat01-stdin"
        );
    }

    #[test]
    fn test_resolve_token_stdin_empty_errors() {
        // Regression guard: previously an empty stdin produced an empty
        // token that propagated into `register_from_token` and surfaced
        // as an opaque "token exchange failed" error. We now fail fast
        // with a clear local error.
        assert_eq!(resolve_refresh_token("-", ""), Err(TokenSourceError::Empty));
        assert_eq!(
            resolve_refresh_token("-", "\n"),
            Err(TokenSourceError::Empty)
        );
        assert_eq!(
            resolve_refresh_token("-", "   \t  \n"),
            Err(TokenSourceError::Empty)
        );
    }

    #[test]
    fn test_resolve_token_inline_empty_errors() {
        // Catches `claudepot account add --from-token ""`. Same failure
        // mode as empty stdin, same clear error.
        assert_eq!(resolve_refresh_token("", ""), Err(TokenSourceError::Empty));
        assert_eq!(
            resolve_refresh_token("   ", ""),
            Err(TokenSourceError::Empty)
        );
    }

    #[test]
    fn test_resolve_token_stdin_line_ignored_when_arg_not_dash() {
        // Defensive: if the caller populates stdin_line for some reason
        // but the arg isn't "-", we must use the arg, not the stdin line.
        assert_eq!(
            resolve_refresh_token("inline", "stdin-value").unwrap(),
            "inline"
        );
    }

    #[test]
    fn test_token_source_error_display() {
        // Stable user-facing message — checked so future edits don't
        // silently change the CLI's error text.
        assert_eq!(
            TokenSourceError::Empty.to_string(),
            "refresh token is empty (nothing on stdin?)"
        );
    }

    // ---- format_register_human -----------------------------------

    #[test]
    fn test_register_line_both_present() {
        // Happy path — rate_tier_20 collapses to "20", Max capitalises.
        assert_eq!(
            format_register_human("a@b.com", "max", Some("rate_tier_20")),
            "Registered: a@b.com (Max 20)"
        );
    }

    #[test]
    fn test_register_line_no_tier_drops_trailing_space() {
        // Regression: previously rendered "Registered: a@b.com (Max )"
        // with a dangling space before the paren.
        assert_eq!(
            format_register_human("a@b.com", "pro", None),
            "Registered: a@b.com (Pro)"
        );
    }

    #[test]
    fn test_register_line_tier_without_underscore() {
        // If the server ever returns an un-prefixed tier name,
        // `split('_').next_back()` leaves it intact — still rendered,
        // just without a separator.
        assert_eq!(
            format_register_human("a@b.com", "max", Some("bespoke")),
            "Registered: a@b.com (Max bespoke)"
        );
    }

    #[test]
    fn test_register_line_empty_subscription_keeps_tier() {
        // Defensive: if subscription_type is blank, we shouldn't emit
        // "Registered: a@b.com ( 20)" — the helper reorganises.
        assert_eq!(
            format_register_human("a@b.com", "", Some("rate_tier_20")),
            "Registered: a@b.com (20)"
        );
    }

    #[test]
    fn test_register_line_all_empty_drops_parens() {
        // Extreme defensive case: no plan metadata at all.
        assert_eq!(
            format_register_human("a@b.com", "", None),
            "Registered: a@b.com"
        );
    }

    #[test]
    fn test_register_line_empty_tier_string_treated_as_missing() {
        // Some("") in rate_limit_tier — split gives Some(""), the
        // helper then trims to "" and the empty-tier branch is taken.
        assert_eq!(
            format_register_human("a@b.com", "max", Some("")),
            "Registered: a@b.com (Max)"
        );
    }
}

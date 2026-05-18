//! Parse an `origin` remote URL into (owner, repo).
//!
//! GitHub's remote URLs come in three shapes:
//!   * SSH: `git@github.com:owner/repo.git` (or without `.git`)
//!   * HTTPS: `https://github.com/owner/repo.git`
//!   * Git protocol: `git://github.com/owner/repo.git` (rare)
//!
//! We accept all three. Non-GitHub remotes return `None` — Claudepot
//! only renders the badge for GitHub, and a future GitLab badge
//! would live in a sibling module with its own parser.
//!
//! `.git` suffix is stripped if present. Owner and repo are
//! returned exactly as the URL spells them (case-sensitive, since
//! GitHub URLs are case-sensitive in the API even when its web UI
//! redirects).

#[derive(Debug, PartialEq, Eq)]
pub struct OwnerRepo {
    pub owner: String,
    pub repo: String,
}

/// Parse a remote URL into (owner, repo) iff it points at GitHub.
pub fn parse_github_origin(url: &str) -> Option<OwnerRepo> {
    let trimmed = url.trim();
    let body = strip_known_prefix(trimmed)?;
    // body is now "owner/repo" or "owner/repo.git".
    let (owner, rest) = body.split_once('/')?;
    if owner.is_empty() {
        return None;
    }
    // Take everything up to the first further slash or end; defends
    // against pasted URLs that include `/pull/123` or similar tails.
    let repo_with_suffix = rest.split(['/', '?', '#']).next()?;
    let repo = repo_with_suffix
        .strip_suffix(".git")
        .unwrap_or(repo_with_suffix);
    if repo.is_empty() {
        return None;
    }
    Some(OwnerRepo {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

fn strip_known_prefix(url: &str) -> Option<&str> {
    // Order matters: SSH form (`git@github.com:`) shares the
    // `github.com` substring with the HTTPS form, so check SSH first.
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        return Some(rest);
    }
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        return Some(rest);
    }
    if let Some(rest) = url.strip_prefix("http://github.com/") {
        return Some(rest);
    }
    if let Some(rest) = url.strip_prefix("git://github.com/") {
        return Some(rest);
    }
    // `ssh://git@github.com/owner/repo` is rarer but legal.
    if let Some(rest) = url.strip_prefix("ssh://git@github.com/") {
        return Some(rest);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(owner: &str, repo: &str) -> OwnerRepo {
        OwnerRepo {
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    #[test]
    fn parses_ssh_with_git_suffix() {
        assert_eq!(
            parse_github_origin("git@github.com:xiaolai/claudepot.git"),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn parses_ssh_without_git_suffix() {
        assert_eq!(
            parse_github_origin("git@github.com:xiaolai/claudepot"),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn parses_https_with_git_suffix() {
        assert_eq!(
            parse_github_origin("https://github.com/xiaolai/claudepot.git"),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn parses_https_without_git_suffix() {
        assert_eq!(
            parse_github_origin("https://github.com/xiaolai/claudepot"),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn parses_git_protocol() {
        assert_eq!(
            parse_github_origin("git://github.com/xiaolai/claudepot.git"),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn parses_ssh_url_scheme() {
        assert_eq!(
            parse_github_origin("ssh://git@github.com/xiaolai/claudepot.git"),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn strips_url_tail_like_pull_request_path() {
        // User-pasted URLs occasionally include the PR path. We
        // recover the owner/repo and ignore the tail.
        assert_eq!(
            parse_github_origin("https://github.com/xiaolai/claudepot/pull/42"),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn ignores_query_string_and_fragment() {
        assert_eq!(
            parse_github_origin("https://github.com/xiaolai/claudepot?utm=x"),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn rejects_non_github_host() {
        assert_eq!(parse_github_origin("git@gitlab.com:x/y.git"), None);
        assert_eq!(parse_github_origin("https://bitbucket.org/x/y.git"), None);
    }

    #[test]
    fn rejects_empty_or_partial() {
        assert_eq!(parse_github_origin(""), None);
        assert_eq!(parse_github_origin("git@github.com:"), None);
        assert_eq!(parse_github_origin("git@github.com:xiaolai"), None);
        assert_eq!(parse_github_origin("https://github.com/"), None);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            parse_github_origin("  https://github.com/xiaolai/claudepot  "),
            Some(mk("xiaolai", "claudepot"))
        );
    }

    #[test]
    fn preserves_case() {
        assert_eq!(
            parse_github_origin("git@github.com:XiaoLai/Claudepot.git"),
            Some(mk("XiaoLai", "Claudepot"))
        );
    }
}

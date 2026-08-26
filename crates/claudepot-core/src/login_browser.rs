//! Opening the OAuth authorize URL for a login without disturbing the
//! user's own `claude.ai` session.
//!
//! # The problem this exists for
//!
//! `claude auth login` hands its authorize URL to the OS default
//! handler. Measured against CC **2.1.246** (2026-08-26) by capturing
//! the URL with a `PATH` shim, that URL is:
//!
//! ```text
//! https://claude.com/cai/oauth/authorize
//!   ?code=true&client_id=…&response_type=code&redirect_uri=…
//!   &scope=…&code_challenge=…&code_challenge_method=S256&state=…
//! ```
//!
//! There is **no `prompt` parameter** — no `select_account`, no
//! `login`. So the endpoint honours whatever `claude.ai` session the
//! browser already holds: adding a second account silently
//! re-authorizes the first, and it presents as success because a
//! credential lands and `/profile` agrees with it.
//!
//! # Two ways to fix that, and why this prefers the second
//!
//! Signing the browser out of `claude.ai` first works, and is what
//! [`crate::onboard::clear_browser_session`] does. It is also blunt:
//! the operation "add account B" has no business ending the user's
//! reading session as account A.
//!
//! Isolation has no such cost. Opened in a **private window**, the flow
//! starts with an empty cookie jar — there is no session to fight and
//! nothing of the user's to clear. The localhost callback still works,
//! because a private window reaches localhost like any other.
//!
//! # Where it degrades, and to what
//!
//! Private-window invocation is per-browser and Safari has no
//! command-line equivalent at all. So this is best-effort by
//! construction: [`plan_open`] returns [`OpenPlan::LogoutThenDefault`]
//! for any browser it cannot isolate, which is exactly the 0.5.5
//! behaviour. Nobody loses correctness; some users keep the sign-out.

/// How to put the authorize URL in front of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenPlan {
    /// Launch this browser with a private/incognito window. The user's
    /// session is never touched, so no logout is performed.
    Private { program: String, args: Vec<String> },
    /// No private mode reachable from the command line for this
    /// browser (Safari, or an unrecognised default). Clear the
    /// `claude.ai` session, then open normally — correctness is kept,
    /// the sign-out is the price.
    LogoutThenDefault,
}

/// Browsers whose private mode is reachable from the command line, by
/// macOS bundle id. Kept as a table rather than a match chain so the
/// supported set is one readable list.
///
/// Chromium forks are listed individually on purpose: matching a
/// substring like `chrome` would also catch unrelated bundle ids, and a
/// wrong `--incognito` on a browser that does not take it opens nothing
/// at all — a silent failure, which is the outcome this whole module is
/// trying to remove.
const MACOS_PRIVATE_FLAGS: &[(&str, &str, &str)] = &[
    // (bundle id, app name for `open -na`, private-window flag)
    ("com.google.chrome", "Google Chrome", "--incognito"),
    (
        "com.google.chrome.canary",
        "Google Chrome Canary",
        "--incognito",
    ),
    ("com.brave.browser", "Brave Browser", "--incognito"),
    ("com.microsoft.edgemac", "Microsoft Edge", "--inprivate"),
    ("com.vivaldi.vivaldi", "Vivaldi", "--incognito"),
    ("com.operasoftware.opera", "Opera", "--private"),
    ("org.mozilla.firefox", "Firefox", "--private-window"),
    (
        "org.mozilla.firefoxdeveloperedition",
        "Firefox Developer Edition",
        "--private-window",
    ),
    ("company.thebrowser.browser", "Arc", "--incognito"),
];

/// Decide how to open `url` given the resolved default browser.
///
/// Pure: the judgement is separated from both the platform probe and
/// the process spawn, so every branch is testable without a browser.
/// `browser_id` is a macOS bundle id, lowercased by the caller; `None`
/// means the probe could not answer, which is treated the same as an
/// unrecognised browser.
pub fn plan_open(browser_id: Option<&str>, url: &str) -> OpenPlan {
    let Some(id) = browser_id else {
        return OpenPlan::LogoutThenDefault;
    };
    let id = id.to_ascii_lowercase();
    for (bundle, app, flag) in MACOS_PRIVATE_FLAGS {
        if id == *bundle {
            return OpenPlan::Private {
                program: "/usr/bin/open".to_string(),
                args: vec![
                    // -n: a new instance, so the URL cannot be handed to
                    // an already-running window that has the user's
                    // cookies. Without it the private flag is silently
                    // ignored when the browser is already open — the
                    // failure that makes this whole feature a no-op.
                    "-n".to_string(),
                    "-a".to_string(),
                    (*app).to_string(),
                    "--args".to_string(),
                    (*flag).to_string(),
                    url.to_string(),
                ],
            };
        }
    }
    OpenPlan::LogoutThenDefault
}

/// The macOS bundle id registered to handle `https`, lowercased.
///
/// Reads LaunchServices' handler map. Any failure — the key absent, the
/// output in a shape this does not parse, `defaults` missing — returns
/// `None`, which [`plan_open`] treats as "cannot isolate" and therefore
/// falls back to the behaviour that already shipped. There is no
/// failure here that costs correctness.
#[cfg(target_os = "macos")]
pub async fn default_browser_id() -> Option<String> {
    let out = tokio::process::Command::new("defaults")
        .args([
            "read",
            "com.apple.LaunchServices/com.apple.launchservices.secure",
            "LSHandlers",
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_https_handler(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(target_os = "macos"))]
pub async fn default_browser_id() -> Option<String> {
    // Only macOS is wired up. Elsewhere the fallback is the previous
    // behaviour, which is correct — just not as quiet. Documented in
    // AGENTS.md rather than left for someone to discover.
    None
}

/// Pull the `https` handler out of `defaults read … LSHandlers` output.
///
/// The format is a plist rendered as nested braces, one block per
/// handler, e.g.
///
/// ```text
/// {
///     LSHandlerPreferredVersions =         {
///         LSHandlerRoleAll = "-";
///     };
///     LSHandlerRoleAll = "com.google.chrome";
///     LSHandlerURLScheme = https;
/// }
/// ```
///
/// Note `LSHandlerRoleAll` appears **twice** in that block — once
/// nested inside `LSHandlerPreferredVersions` with a junk value. Taking
/// the first match in the block yields `"-"`, so this takes the last
/// one before the scheme line, which is the real handler.
pub(crate) fn parse_https_handler(text: &str) -> Option<String> {
    let mut candidate: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("LSHandlerRoleAll") {
            if let Some(v) = rest.split('=').nth(1) {
                let v = v.trim().trim_end_matches(';').trim().trim_matches('"');
                // "-" is the placeholder inside LSHandlerPreferredVersions.
                if !v.is_empty() && v != "-" {
                    candidate = Some(v.to_ascii_lowercase());
                }
            }
        } else if line.starts_with("LSHandlerURLScheme") {
            let scheme = line
                .split('=')
                .nth(1)
                .unwrap_or("")
                .trim()
                .trim_end_matches(';')
                .trim()
                .trim_matches('"')
                .to_ascii_lowercase();
            if scheme == "https" {
                if let Some(c) = candidate.take() {
                    return Some(c);
                }
            }
            // A block for a different scheme: whatever handler we were
            // holding belonged to it, so drop it rather than letting it
            // leak into the next block.
            candidate = None;
        } else if line == "}" || line == "}," {
            // Block boundary. Keep `candidate` — the scheme line can
            // follow the handler line within the same block, and the
            // nested `LSHandlerPreferredVersions` dict closes here too.
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://claude.com/cai/oauth/authorize?state=abc";

    #[test]
    fn unknown_browser_falls_back_to_logout() {
        assert_eq!(plan_open(None, URL), OpenPlan::LogoutThenDefault);
        assert_eq!(
            plan_open(Some("com.example.weirdbrowser"), URL),
            OpenPlan::LogoutThenDefault
        );
    }

    /// Safari is the case that keeps the fallback alive: it is a common
    /// macOS default and has no command-line private mode.
    #[test]
    fn safari_falls_back_to_logout() {
        assert_eq!(
            plan_open(Some("com.apple.safari"), URL),
            OpenPlan::LogoutThenDefault
        );
    }

    #[test]
    fn chrome_gets_an_incognito_window() {
        let plan = plan_open(Some("com.google.chrome"), URL);
        let OpenPlan::Private { program, args } = plan else {
            panic!("expected a private window, got {plan:?}");
        };
        assert_eq!(program, "/usr/bin/open");
        assert!(args.contains(&"--incognito".to_string()));
        assert!(args.contains(&URL.to_string()));
    }

    /// `-n` forces a new instance. Without it macOS hands the URL to an
    /// already-running browser, the private flag is ignored, and the
    /// feature silently becomes a no-op against the user's own cookies.
    #[test]
    fn private_launch_always_forces_a_new_instance() {
        for (bundle, _, _) in MACOS_PRIVATE_FLAGS {
            let plan = plan_open(Some(bundle), URL);
            let OpenPlan::Private { args, .. } = plan else {
                panic!("{bundle} should be isolatable");
            };
            assert_eq!(args.first().map(String::as_str), Some("-n"), "{bundle}");
        }
    }

    #[test]
    fn bundle_id_match_is_case_insensitive() {
        assert!(matches!(
            plan_open(Some("com.Google.Chrome"), URL),
            OpenPlan::Private { .. }
        ));
    }

    /// A near-miss must not be isolated: a wrong private flag opens
    /// nothing, which is a silent failure rather than a loud one.
    #[test]
    fn substring_lookalikes_are_not_treated_as_chrome() {
        assert_eq!(
            plan_open(Some("com.google.chrome.helper"), URL),
            OpenPlan::LogoutThenDefault
        );
    }

    #[test]
    fn parses_the_https_handler_from_launchservices_output() {
        let sample = r#"(
    {
        LSHandlerPreferredVersions =         {
            LSHandlerRoleAll = "-";
        };
        LSHandlerRoleAll = "com.apple.safari";
        LSHandlerURLScheme = http;
    },
    {
        LSHandlerPreferredVersions =         {
            LSHandlerRoleAll = "-";
        };
        LSHandlerRoleAll = "com.google.chrome";
        LSHandlerURLScheme = https;
    }
)"#;
        assert_eq!(
            parse_https_handler(sample),
            Some("com.google.chrome".to_string())
        );
    }

    /// The nested `LSHandlerPreferredVersions` dict carries its own
    /// `LSHandlerRoleAll = "-"`. Taking the first match in a block
    /// yields that placeholder instead of the browser.
    #[test]
    fn placeholder_inside_preferred_versions_is_not_mistaken_for_a_handler() {
        let sample = r#"{
        LSHandlerPreferredVersions =         {
            LSHandlerRoleAll = "-";
        };
        LSHandlerRoleAll = "org.mozilla.firefox";
        LSHandlerURLScheme = https;
    }"#;
        assert_eq!(
            parse_https_handler(sample),
            Some("org.mozilla.firefox".to_string())
        );
    }

    /// No https block at all — a real state on a fresh machine, and it
    /// must degrade rather than guess.
    #[test]
    fn absent_https_handler_is_none() {
        let sample = r#"{
        LSHandlerRoleAll = "com.apple.safari";
        LSHandlerURLScheme = ftp;
    }"#;
        assert_eq!(parse_https_handler(sample), None);
        assert_eq!(parse_https_handler(""), None);
    }

    /// A handler belonging to a different scheme must not leak into a
    /// later block that has no handler line of its own.
    #[test]
    fn a_handler_does_not_leak_across_scheme_blocks() {
        let sample = r#"{
        LSHandlerRoleAll = "com.google.chrome";
        LSHandlerURLScheme = mailto;
    },
    {
        LSHandlerURLScheme = https;
    }"#;
        assert_eq!(parse_https_handler(sample), None);
    }
}

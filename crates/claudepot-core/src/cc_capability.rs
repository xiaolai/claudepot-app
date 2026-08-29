//! Does the installed Claude Code binary actually have the subcommand
//! we are about to spawn?
//!
//! # The asymmetry this module exists for
//!
//! `claude`'s grammar is `claude [options] [command] [prompt]`. An
//! unrecognized **positional** is not an error — it becomes the
//! *prompt*. So `claude daemon status` against a binary with no
//! `daemon` subcommand does not fail: it starts a headless session
//! whose prompt is the word `daemon`, calls the model, and bills for
//! it. Anthropic acknowledged this class upstream at CC **2.1.199**
//! ("Fixed `claude --dangerously-skip-permissions daemon
//! <subcommand>` being treated as a chat prompt instead of running
//! the subcommand"), so it is observed behaviour, not a theory.
//!
//! Reported as issue #94: Claudepot polled `claude daemon status`
//! once a minute against CC 2.1.39 — which predates `daemon` — and
//! burned ~20K uncached input tokens per poll, 288 headless sessions
//! in one day, while rendering nothing. The scrape was "infallible by
//! design", so every one of those failures degraded to a hidden badge.
//!
//! An **option** behaves the opposite way: commander rejects an
//! unknown one with a non-zero exit and a usage message, and never
//! folds it into the prompt. That asymmetry is what makes it safe to
//! implement a capability probe in terms of the very CLI being
//! probed — `--version` cannot become a billed prompt.
//!
//! # What a floor can and cannot do
//!
//! A floor guards the **lower** bound only. It answers "is this binary
//! too old to have the subcommand" and nothing else. It cannot guard
//! the upper bound: a future CC that renames or removes a subcommand
//! sails straight through a `>= 2.1.139` check and falls through to a
//! prompt again.
//!
//! So the floor is the right guard for a spawn a **human triggered
//! once**, where the cost of being wrong is one call, and the wrong
//! guard for a **polled** one, where it is one call per minute
//! forever. Polled data must not reach the CLI at all — see
//! [`crate::cc_daemon`], which reads `roster.json` and spawns nothing;
//! where there is no file to read instead, the guard is a circuit
//! breaker (`src-tauri/src/cc_doctor_watcher.rs`). Do not "fix" a
//! future polling surface by adding a variant here.
//!
//! # Unknown fails closed
//!
//! [`Capability::Unknown`] is returned when the version cannot be
//! read, and callers must treat it as *do not spawn*. The asymmetry is
//! the same one that motivates the module: refusing to spawn costs a
//! feature the user can still reach another way, and spawning costs
//! money silently. There is no reading of "we could not tell" that
//! justifies the billed branch.
//!
//! # Why the subcommand is a type and not a string
//!
//! [`GatedSubcommand`] is an enum, so every value has a floor by
//! construction and a typo cannot exist. An earlier revision took
//! `&str` and answered `Supported` for any name it did not recognise —
//! defensible for genuinely ungated verbs, and a silent fail-open for
//! `check("ath", …)`. On a gate whose failure mode is billing, that
//! trade is the wrong way round.

use crate::proc_utils::NoWindowExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

/// Wall-clock cap on the `claude --version` probe. Real runs land in
/// 30–80 ms; the cap exists so a half-installed `claude` that hangs on
/// a stdin read cannot stall a sign-in.
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// A parsed `major.minor.patch`. Ordering is tuple ordering, which is
/// what makes `>=` against a floor mean what it looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CcVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl CcVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse the leading `N.N.N` of `claude --version` output.
    ///
    /// Accepts what CC actually prints — `"2.1.251 (Claude Code)"` — by
    /// taking the first whitespace-separated token. Rejects anything
    /// that is not exactly three dot-separated runs of ASCII digits, so
    /// `"unknown"`, `"2.1"` and an error message all yield `None`
    /// rather than a version that compares as very old (silently
    /// disabling a working feature) or very new (silently authorising
    /// the billed spawn).
    ///
    /// This is the **only** parse step between the subprocess and a
    /// decision. An earlier revision funnelled the output through
    /// `cc_doctor::probes::parse_version_line` first, which accepts
    /// `"2.1"` and `"2.1.3.4"` — shapes this then rejected, so the two
    /// validators disagreed about what a version is.
    pub fn parse(s: &str) -> Option<Self> {
        let token = s.split_whitespace().next()?;
        let mut parts = token.split('.');
        let mut next = || -> Option<u32> {
            let p = parts.next()?;
            if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()
        };
        let major = next()?;
        let minor = next()?;
        let patch = next()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self::new(major, minor, patch))
    }
}

impl std::fmt::Display for CcVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A CC subcommand whose version floor Claudepot has verified.
///
/// This is a **registry of verified floors**, not an inventory of live
/// spawn sites: [`GatedSubcommand::Daemon`] is retained after
/// `cc_daemon` stopped spawning it, so that re-adding a spawn cannot
/// silently skip a gate whose number is already established. A spawn
/// site with no variant here is a review finding — it is a billed
/// prompt waiting for a user on an older binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatedSubcommand {
    /// `claude auth login` — the sign-in path in [`crate::onboard`].
    Auth,
    /// `claude daemon status`. **Not spawned anywhere**; see the type
    /// docs for why the floor is kept.
    Daemon,
}

impl GatedSubcommand {
    /// Every variant, for tests that must cover the whole registry.
    pub const ALL: &'static [GatedSubcommand] = &[Self::Auth, Self::Daemon];

    /// The positional as it appears on the command line.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Daemon => "daemon",
        }
    }

    /// First CC release that has the subcommand. A total match, so a
    /// new variant cannot compile without one.
    pub const fn since(self) -> CcVersion {
        match self {
            Self::Auth => CcVersion::new(2, 1, 41),
            Self::Daemon => CcVersion::new(2, 1, 139),
        }
    }

    /// How the floor was established, so a later reader can re-check it
    /// instead of trusting the number. Every arm cites CC's own
    /// changelog — the authority `.claude/rules/cc-upstream-watch.md`
    /// permits alongside the installed binary, never the abandoned
    /// source mirror.
    pub const fn evidence(self) -> &'static str {
        match self {
            Self::Auth => {
                "CC changelog 2.1.41: \"Added `claude auth login`, `claude auth status`, \
                 and `claude auth logout` CLI subcommands\""
            }
            Self::Daemon => {
                "CC changelog 2.1.139 introduced the background-agent surface (\"Added agent \
                 view (Research Preview) … Run `claude agents`\"); the first \
                 `claude daemon status` fix lands at 2.1.141"
            }
        }
    }
}

/// Verdict for one (subcommand, binary) pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    /// Safe to spawn.
    Supported,
    /// The binary predates the subcommand. Spawning it would be
    /// charged as a prompt.
    TooOld {
        installed: CcVersion,
        since: CcVersion,
    },
    /// The version could not be established. Treat as *do not spawn*.
    ///
    /// Carries `since` so a caller rendering a message never has to
    /// look the floor up again — an earlier revision did, through a
    /// fallible lookup whose only failure branch produced an empty
    /// version string in user-facing copy.
    Unknown { since: CcVersion, reason: String },
}

impl Capability {
    /// The single question every call site asks. A method rather than
    /// `matches!` at each site so the fail-closed reading of
    /// [`Capability::Unknown`] is decided once, here, and cannot be got
    /// wrong by a new caller.
    pub fn may_spawn(&self) -> bool {
        matches!(self, Capability::Supported)
    }
}

/// Pure decision: does `installed` clear the floor for `subcommand`?
///
/// Total — every [`GatedSubcommand`] has a floor by construction, so
/// there is no "unrecognised name" branch to fall open through.
pub fn evaluate(subcommand: GatedSubcommand, installed: Option<CcVersion>) -> Capability {
    let since = subcommand.since();
    match installed {
        Some(installed) if installed >= since => Capability::Supported,
        Some(installed) => Capability::TooOld { installed, since },
        None => Capability::Unknown {
            since,
            reason: format!(
                "could not read `claude --version`, so cannot tell whether this build \
                 has the `{}` subcommand",
                subcommand.as_str()
            ),
        },
    }
}

/// Probe the binary and evaluate in one step.
///
/// Async and bounded: `.claude/rules/rust-conventions.md` requires
/// subprocesses to go through `tokio::process::Command`, and this runs
/// on the sign-in path, where a synchronous probe would block the
/// runtime for up to [`VERSION_PROBE_TIMEOUT`]. `kill_on_drop` matters
/// as much as the timeout — dropping the future at the deadline must
/// actually reap the child, not leave a hung `claude` behind.
pub async fn check(subcommand: GatedSubcommand, binary: &Path) -> Capability {
    evaluate(subcommand, probe_version(binary).await)
}

/// `None` on every failure — spawn error, non-zero exit, unparseable
/// output, or timeout. The caller turns that into
/// [`Capability::Unknown`], which refuses. Collapsing the causes is
/// deliberate: they differ in diagnosis and not in what we may do next.
async fn probe_version(binary: &Path) -> Option<CcVersion> {
    let output = tokio::process::Command::new(binary)
        .arg("--version")
        // Inherit nothing — the probe must not adopt a parent's pty.
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .no_window()
        .output();

    let output = tokio::time::timeout(VERSION_PROBE_TIMEOUT, output)
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    CcVersion::parse(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_shape_claude_actually_prints() {
        assert_eq!(
            CcVersion::parse("2.1.251 (Claude Code)"),
            Some(CcVersion::new(2, 1, 251))
        );
        assert_eq!(CcVersion::parse("2.1.39\n"), Some(CcVersion::new(2, 1, 39)));
    }

    /// A version we cannot parse must not compare as anything. Both
    /// wrong answers are silent: "very old" disables a working
    /// feature, "very new" re-opens the billed spawn.
    #[test]
    fn refuses_shapes_that_are_not_three_numbers() {
        for bad in ["unknown", "2.1", "2.1.x", "2.1.3.4", "", "v2.1.3", "2..3"] {
            assert_eq!(CcVersion::parse(bad), None, "should refuse {bad:?}");
        }
    }

    #[test]
    fn orders_by_component_not_lexically() {
        // The string compare that "2.1.9" > "2.1.139" would produce is
        // exactly the bug this ordering exists to avoid.
        assert!(CcVersion::new(2, 1, 139) > CcVersion::new(2, 1, 9));
        assert!(CcVersion::new(2, 2, 0) > CcVersion::new(2, 1, 999));
        assert!(CcVersion::new(3, 0, 0) > CcVersion::new(2, 9, 9));
    }

    #[test]
    fn the_reported_binary_is_refused_for_every_gated_subcommand() {
        // CC 2.1.39 is the binary from issue #94. It predates `daemon`
        // (2.1.139) and also `auth` (2.1.41) — which is the finding the
        // original report missed.
        let v = CcVersion::parse("2.1.39").unwrap();
        for cmd in GatedSubcommand::ALL {
            assert!(!evaluate(*cmd, Some(v)).may_spawn(), "{cmd:?}");
        }
    }

    #[test]
    fn the_floor_release_itself_is_supported() {
        // `since` is the release that ADDED the subcommand, so it must
        // compare as supported. An off-by-one here would disable the
        // feature on the exact version that introduced it.
        for cmd in GatedSubcommand::ALL {
            assert!(evaluate(*cmd, Some(cmd.since())).may_spawn(), "{cmd:?}");
        }
    }

    #[test]
    fn current_generation_clears_every_floor() {
        let v = CcVersion::parse("2.1.251").unwrap();
        for cmd in GatedSubcommand::ALL {
            assert!(evaluate(*cmd, Some(v)).may_spawn(), "{cmd:?}");
        }
    }

    #[test]
    fn unknown_version_fails_closed_and_carries_the_floor() {
        for cmd in GatedSubcommand::ALL {
            let c = evaluate(*cmd, None);
            assert!(
                !c.may_spawn(),
                "an unreadable version must never authorise a spawn — that is the \
                 branch that bills"
            );
            // The floor rides along so a caller rendering a message
            // never needs a second, fallible lookup.
            assert!(matches!(c, Capability::Unknown { since, .. } if since == cmd.since()));
        }
    }

    #[test]
    fn every_floor_records_how_it_was_established() {
        for cmd in GatedSubcommand::ALL {
            assert!(
                cmd.evidence().contains("changelog"),
                "{cmd:?} must cite the changelog so the number can be re-checked"
            );
        }
    }

    /// The subprocess half. Unix-only: the stubs are shell scripts, and
    /// `.claude/rules/paths.md` says OS-specific behaviour is gated
    /// rather than faked. The pure `evaluate` half above runs
    /// everywhere, including the Windows CI leg.
    #[cfg(unix)]
    mod probe {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        /// Write an executable `/bin/sh` stub and return its path.
        fn stub(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        }

        #[tokio::test]
        async fn a_modern_binary_is_supported() {
            let d = tempfile::tempdir().unwrap();
            let bin = stub(d.path(), "claude", "echo '2.1.251 (Claude Code)'");
            assert_eq!(
                check(GatedSubcommand::Auth, &bin).await,
                Capability::Supported
            );
        }

        #[tokio::test]
        async fn the_issue_94_binary_is_refused() {
            let d = tempfile::tempdir().unwrap();
            let bin = stub(d.path(), "claude", "echo '2.1.39 (Claude Code)'");
            let c = check(GatedSubcommand::Auth, &bin).await;
            assert!(!c.may_spawn());
            assert!(matches!(c, Capability::TooOld { .. }), "got {c:?}");
        }

        /// Garbage on stdout, a non-zero exit, a binary that is not
        /// there, and one that hangs past the deadline are four
        /// different faults with one correct answer: refuse.
        #[tokio::test]
        async fn every_probe_failure_fails_closed() {
            let d = tempfile::tempdir().unwrap();
            let cases = [
                ("garbage", stub(d.path(), "garbage", "echo 'not a version'")),
                ("empty", stub(d.path(), "empty", "true")),
                ("nonzero", stub(d.path(), "nonzero", "echo 2.1.251; exit 3")),
                ("missing", d.path().join("does-not-exist")),
            ];
            for (name, bin) in cases {
                let c = check(GatedSubcommand::Auth, &bin).await;
                assert!(!c.may_spawn(), "{name} must refuse, got {c:?}");
                assert!(
                    matches!(c, Capability::Unknown { .. }),
                    "{name} should be Unknown, got {c:?}"
                );
            }
        }

        /// A hung probe must be bounded AND reaped. Without
        /// `kill_on_drop` the timeout returns while `claude` keeps
        /// running — invisible to this assertion but a leaked process
        /// per sign-in attempt.
        #[tokio::test(start_paused = true)]
        async fn a_hanging_binary_is_refused_at_the_deadline() {
            let d = tempfile::tempdir().unwrap();
            let bin = stub(d.path(), "claude", "sleep 600");
            let c = check(GatedSubcommand::Auth, &bin).await;
            assert!(!c.may_spawn(), "got {c:?}");
            assert!(matches!(c, Capability::Unknown { .. }), "got {c:?}");
        }
    }
}

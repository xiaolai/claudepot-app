//! Merge a probe result with a pty scrape result into a final
//! `DoctorSnapshot`.
//!
//! ## The composition rule
//!
//! Probe values are *more trustworthy* than scrape values for the
//! fields where both exist:
//!
//! - The probe sources `version` from `claude --version` — a
//!   non-interactive subprocess that exits in ~50 ms with a single
//!   plain-text line. No Ink, no cursor-up replay, no timing race.
//! - The probe sources `install_path` from `fs::canonicalize` on
//!   the resolved binary — strictly more accurate than CC's
//!   self-reported `Path:` row (which is just `argv[0]` for the
//!   running process and can differ from the symlink target).
//! - The probe sources `install_type` by classifying the resolved
//!   path. The scrape's `Currently running:` row also has it, but
//!   when the scrape failed there's no row.
//!
//! Therefore: probe values overwrite scrape values for those three
//! fields. The scrape's `sections`, `parse_status`, and `raw_bytes`
//! always pass through unchanged — sections carry signals the
//! probes don't replicate (Updates, Background server, Plugin
//! errors), and `parse_status` is the developer-facing diagnostic
//! that the pane uses to render the parse-failures banner.
//!
//! ## Severity is NOT promoted by a successful probe
//!
//! Earlier drafts of this module promoted [`DoctorSeverity::Unknown`]
//! to `Healthy` when the probe succeeded. That was wrong: knowing
//! `claude --version` returns `2.1.140` tells us *the binary
//! exists and reports a version*. It tells us nothing about plugin
//! errors, background-server state, or the `Updates` section —
//! all of which live in the scrape's parsed sections. Promoting
//! Unknown→Healthy would have flipped the pill green for a user
//! whose plugins are misconfigured purely because their scrape
//! parser tripped.
//!
//! So: severity stays `Unknown` when the scrape gave up. The
//! Health pane handles the resulting `(ccVersion: Some, severity:
//! Unknown)` state by rendering the version + install header
//! normally (the user sees identity facts) while keeping the dot
//! grey and surfacing the parse-failure banner as the action
//! affordance. Honest signal, no false green.

use crate::cc_doctor::probes::{probe_version, VersionProbe};
use crate::cc_doctor::scrape::{scrape_doctor, DoctorSnapshot};

/// Public entry point used by the IPC command and the background
/// watcher. Equivalent to [`scrape_doctor`] but with the probe
/// overlay layered on top.
///
/// Blocking on the calling thread for the same reason
/// `scrape_doctor` blocks — the pty read is synchronous. Callers
/// should run this from `spawn_blocking`.
///
/// ### Probe-after-scrape ordering
///
/// The probe runs AFTER the scrape, not before. The scrape takes
/// 6–10 s (CC's npm dist-tag fetch); if CC self-updated during
/// that window, a pre-scrape probe would record the pre-update
/// version and the post-scrape compose would then overwrite the
/// scrape's fresher `Currently running:` row with the stale
/// probe data. Running the probe after the scrape captures the
/// post-update state on both sides, so the merged snapshot is
/// internally consistent.
pub fn scrape_with_probes() -> DoctorSnapshot {
    let scrape = scrape_doctor();
    let probe = probe_version();
    compose(scrape, probe)
}

/// Pure composition function — given a scrape result and an
/// optional probe, return the merged snapshot. Extracted from
/// `scrape_with_probes` so tests can stub both inputs without
/// running a real pty or fork-exec.
pub fn compose(mut scrape: DoctorSnapshot, probe: Option<VersionProbe>) -> DoctorSnapshot {
    let Some(p) = probe else {
        // No probe — return the scrape unchanged. If the scrape
        // itself was Unknown, it stays Unknown; the pane will
        // render grey and the user can click Refresh.
        return scrape;
    };

    // Probe wins for identity fields. Even when the scrape already
    // had a version, the probe's version is the more authoritative
    // value — it's the *currently running* binary, while the scrape's
    // `Currently running:` row can lag by one self-update cycle if
    // CC restarted mid-scrape.
    scrape.cc_version = Some(p.version);
    if let Some(ty) = p.install_type {
        scrape.install_type = Some(ty);
    }
    scrape.install_path = Some(p.binary_path.to_string_lossy().to_string());

    // Severity is NOT promoted. See the module docs above: identity
    // ≠ health. When the scrape returned `Unknown` because it
    // couldn't parse sections, we keep the dot grey even though
    // the probe gave us a version; the pane reads the version from
    // `cc_version` and the dot from `severity` independently.
    scrape
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_doctor::scrape::{DoctorSection, DoctorSeverity, ParseStatus, SectionEntry};
    use std::path::PathBuf;

    fn snap_with(
        version: Option<String>,
        severity: DoctorSeverity,
        status: ParseStatus,
        sections: Vec<DoctorSection>,
    ) -> DoctorSnapshot {
        DoctorSnapshot {
            cc_version: version,
            install_type: None,
            install_path: None,
            severity,
            sections,
            raw_bytes: 0,
            parse_status: status,
            captured_at_ms: 0,
        }
    }

    fn probe(version: &str, install_type: Option<&str>, path: &str) -> VersionProbe {
        VersionProbe {
            version: version.to_string(),
            binary_path: PathBuf::from(path),
            install_type: install_type.map(str::to_string),
        }
    }

    #[test]
    fn compose_without_probe_passes_scrape_through() {
        let s = snap_with(
            Some("2.1.140".into()),
            DoctorSeverity::Healthy,
            ParseStatus::Ok,
            vec![],
        );
        let out = compose(s.clone(), None);
        assert_eq!(out.cc_version, s.cc_version);
        assert_eq!(out.severity, s.severity);
    }

    #[test]
    fn compose_overwrites_version_with_probe_value() {
        // Scrape said one version; probe says another — probe wins.
        // The realistic case is a CC self-update that finished
        // between the scrape's `Currently running:` row rendering
        // and the user opening the pane.
        let s = snap_with(
            Some("2.1.128".into()),
            DoctorSeverity::Healthy,
            ParseStatus::Ok,
            vec![],
        );
        let p = probe(
            "2.1.140",
            Some("native"),
            "/Users/me/.local/share/claude/versions/2.1.140",
        );
        let out = compose(s, Some(p));
        assert_eq!(out.cc_version.as_deref(), Some("2.1.140"));
        assert_eq!(out.install_type.as_deref(), Some("native"));
        assert_eq!(
            out.install_path.as_deref(),
            Some("/Users/me/.local/share/claude/versions/2.1.140")
        );
    }

    #[test]
    fn compose_keeps_unknown_severity_even_when_probe_succeeds() {
        // Identity ≠ health. Knowing the binary path tells us
        // nothing about plugin errors or background-server state —
        // both of which would have appeared as scrape sections if
        // the parser hadn't failed. Severity stays Unknown so the
        // pill stays grey and the parse-failure banner remains the
        // actionable signal.
        let s = snap_with(
            None,
            DoctorSeverity::Unknown,
            ParseStatus::Failed {
                reason: "no sections parsed".into(),
            },
            vec![],
        );
        let p = probe(
            "2.1.140",
            Some("native"),
            "/Users/me/.local/share/claude/versions/2.1.140",
        );
        let out = compose(s, Some(p));
        // Identity fields are populated from the probe.
        assert_eq!(out.cc_version.as_deref(), Some("2.1.140"));
        assert_eq!(out.install_type.as_deref(), Some("native"));
        // But severity DOES NOT flip to Healthy — we still can't
        // see the sections that would carry plugin/server status.
        assert_eq!(out.severity, DoctorSeverity::Unknown);
    }

    #[test]
    fn compose_keeps_warning_severity_intact() {
        // Scrape returned Warning (some section flagged) — probe
        // info doesn't override real diagnostic content.
        let s = snap_with(
            Some("2.1.140".into()),
            DoctorSeverity::Warning,
            ParseStatus::Ok,
            vec![DoctorSection {
                title: "Background server".into(),
                severity: DoctorSeverity::Warning,
                entries: vec![SectionEntry {
                    text: "not running".into(),
                    tree_prefix: "└".into(),
                }],
            }],
        );
        let p = probe("2.1.140", Some("native"), "/path");
        let out = compose(s, Some(p));
        assert_eq!(out.severity, DoctorSeverity::Warning);
    }

    #[test]
    fn compose_keeps_error_severity_intact() {
        let s = snap_with(
            None,
            DoctorSeverity::Error,
            ParseStatus::Ok,
            vec![DoctorSection {
                title: "Plugin errors".into(),
                severity: DoctorSeverity::Error,
                entries: vec![],
            }],
        );
        let p = probe("2.1.140", Some("native"), "/path");
        let out = compose(s, Some(p));
        assert_eq!(out.severity, DoctorSeverity::Error);
    }

    #[test]
    fn compose_preserves_parse_status_failed_even_with_probe() {
        // The parse-failures banner in the UI is driven by
        // parse_status, not severity. A successful probe must NOT
        // hide the fact that the scrape parser failed — we still
        // want the dev-alert log link visible.
        let s = snap_with(
            None,
            DoctorSeverity::Unknown,
            ParseStatus::Failed {
                reason: "no sections parsed".into(),
            },
            vec![],
        );
        let p = probe("2.1.140", Some("native"), "/path");
        let out = compose(s, Some(p));
        assert!(matches!(out.parse_status, ParseStatus::Failed { .. }));
    }

    #[test]
    fn compose_keeps_scrape_install_type_when_probe_has_none() {
        // Probe couldn't classify the install path (unknown
        // location), but the scrape's `Currently running:` row had
        // an install_type. Keep the scrape value.
        let s = snap_with(
            Some("2.1.140".into()),
            DoctorSeverity::Healthy,
            ParseStatus::Ok,
            vec![],
        );
        let mut s = s;
        s.install_type = Some("homebrew".into());
        let p = probe("2.1.140", None, "/oddball/path/claude");
        let out = compose(s, Some(p));
        assert_eq!(out.install_type.as_deref(), Some("homebrew"));
    }
}

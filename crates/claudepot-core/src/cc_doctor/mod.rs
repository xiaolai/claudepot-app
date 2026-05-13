//! `claude doctor` output scraping.
//!
//! Surfaces CC's *own* self-diagnostic — installation type, version,
//! warnings, errors — to Claudepot's UI. Distinct from
//! [`crate::services::doctor_service`], which is Claudepot's own
//! health report (accounts, API reachability, proxy).
//!
//! The architecture rule allows read-only introspection over CC's
//! filesystem (Sessions, Config); this extends that to CC's TUI
//! output. The CLI is the authoritative source — re-implementing the
//! 800+ lines of `doctorDiagnostic.ts` / `doctorContextWarnings.ts`
//! in Rust would be a maintenance treadmill. Scraping trades that for
//! parser fragility, which we mitigate with the
//! [`parse_failures`] ring-buffer and [`dev_alert`] developer-only
//! notification path.
//!
//! Submodules:
//! - [`scrape`] — pty spawn, ANSI translation, section parser.
//! - [`probes`] — cheap, deterministic filesystem + subprocess signals
//!   (version, install path, install type) that don't depend on
//!   parsing Ink's TUI output. Used to (a) invalidate the snapshot
//!   cache when the running CC version changes mid-session, and
//!   (b) populate identity fields when the scrape parser fails so
//!   the Health pane still has something to render.
//! - [`compose`] — merge the scrape result with the probe output
//!   into the final `DoctorSnapshot`. Probe values win over scrape
//!   values where they overlap; scrape sections pass through.
//! - [`parse_failures`] — persistent ring buffer of parse failures
//!   for forensics (`~/.claudepot/doctor-parse-failures.jsonl`).
//! - [`dev_alert`] — OS-notification dispatch on parse failure when
//!   `CLAUDEPOT_DEV=1` or compiled with `debug_assertions`.

pub mod compose;
pub mod dev_alert;
pub mod parse_failures;
pub mod probes;
pub mod scrape;

pub use compose::scrape_with_probes;
pub use probes::{probe_version, VersionProbe};
pub use scrape::{
    scrape_doctor, DoctorSection, DoctorSeverity, DoctorSnapshot, ParseStatus, SectionEntry,
};

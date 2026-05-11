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
//! - [`parse_failures`] — persistent ring buffer of parse failures
//!   for forensics (`~/.claudepot/doctor-parse-failures.jsonl`).
//! - [`dev_alert`] — OS-notification dispatch on parse failure when
//!   `CLAUDEPOT_DEV=1` or compiled with `debug_assertions`.

pub mod dev_alert;
pub mod parse_failures;
pub mod scrape;

pub use scrape::{
    scrape_doctor, DoctorSection, DoctorSeverity, DoctorSnapshot, ParseStatus, SectionEntry,
};

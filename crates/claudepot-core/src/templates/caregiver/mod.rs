//! Caregiver bundle — consent records, SMTP delivery, and the
//! structured report schema that the LLM produces against (so
//! the email body never includes free-form text outside a
//! bounded set of fields).
//!
//! See `dev-docs/caregiver-weekly-report-template.md` and
//! `dev-docs/templates-implementation-plan.md` §12.
//!
//! **Highest-trust feature in the product**. Read these design
//! invariants before changing anything in this module:
//!
//! 1. The dependent must affirmatively consent at install time,
//!    in person, by typing their full name into the install
//!    dialog. Consent records are mode 0600.
//! 2. The report body is rendered deterministically from a
//!    typed schema. The LLM emits JSON; we validate against the
//!    `CaregiverReport` struct; only fields in that struct can
//!    survive into the email. Hallucinated fields are dropped
//!    at parse time.
//! 3. The SMTP credential lives in macOS Keychain (via the
//!    `keyring` crate already used elsewhere in claudepot-core).
//!    It never crosses IPC after `smtp_save_credential`.
//! 4. The revoke button is always visible in the tray icon.
//!    Revocation is a single click and emails the caregiver
//!    automatically.

pub mod consent;
pub mod report;
pub mod smtp;

pub use consent::{ConsentRecord, ConsentStore, RevokeReason, SmtpProvider};
pub use report::{render_email, CaregiverReport};
pub use smtp::{send_email, SmtpConfig, SmtpError};

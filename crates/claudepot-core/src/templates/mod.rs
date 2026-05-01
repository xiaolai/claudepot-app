//! Bundled automation templates.
//!
//! A template is a curated, parameterized blueprint for an
//! [`crate::automations::types::Automation`]. The user picks a
//! template from the gallery, fills any placeholders, and the
//! template is materialized into the existing automations runtime.
//!
//! See `dev-docs/templates-implementation-plan.md` for the
//! authoritative spec.
//!
//! ## Module layout
//!
//! - [`blueprint`] — `Blueprint` types + TOML parser, with
//!   cross-field constraint validation (privacy × fallback,
//!   caregiver × consent, etc.).
//! - [`capabilities`] — default capability lookup for routes by
//!   model-name prefix. A *hint*; `Route.capabilities_override` is
//!   the enforcement boundary.
//! - [`registry`] — bundled blueprint loader. Embeds blueprint
//!   TOMLs and sample reports into the binary at build time.
//! - [`error`] — `TemplateError`, the one boundary error type for
//!   this module.
//!
//! Runtime integration (instantiation into an
//! `AutomationCreateDto`, the `_prerun` subcommand, the
//! `record_run` extension for output-artifact discovery, and the
//! apply pipeline) lives in later tiers per the build plan and
//! is not exposed here yet.

pub mod apply;
pub mod blueprint;
pub mod capabilities;
pub mod error;
pub mod instantiate;
pub mod registry;
pub mod routing;

pub use blueprint::{
    ApplyConfig, ApplyOperation, ApplyScope, Blueprint, Capability, Category, CostClass,
    FallbackPolicy, ItemIdStrategy, ModelClass, OutputConfig, Placeholder, PlaceholderType,
    PlaceholderValidation, PrivacyClass, RuntimeConfig, ScheduleConfig, ScheduleShape, Scope,
    TemplateId, Tier,
};
pub use capabilities::{default_capabilities_for, CapabilitySet};
pub use error::TemplateError;
pub use instantiate::{
    instantiate, schedule_to_cron, PlaceholderValue, ResolvedAutomation, ResolvedSchedule,
    ScheduleDto, TemplateInstance, Weekday,
};
pub use registry::TemplateRegistry;

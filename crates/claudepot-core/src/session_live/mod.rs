//! Real-time Claude session monitoring — data plane.
//!
//! See `dev-docs/activity-implementation-plan.md` for the full design.
//! This module is the in-memory live-observability layer; it never
//! writes to `sessions.db` or any other durable store.
//!
//! M1 scope ships only the pure-Rust data plane (WI-0 through WI-5):
//! golden fixtures, redaction floor, types + bus topology, status
//! state machine, tail reader, and PID registry poller. Watcher,
//! runtime, and Tauri/React surfaces land in follow-on milestones.

pub mod bus;
pub mod metrics_store;
pub mod pricing;
pub mod redact;
pub mod registry;
pub mod runtime;
pub mod status;
mod status_helpers;
mod status_types;
pub mod tail;
pub mod transcript_resolver;
pub mod types;

pub use runtime::LiveRuntime;

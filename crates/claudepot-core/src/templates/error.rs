//! Error type for the `templates` module.
//!
//! One enum per module boundary, per `.claude/rules/rust-conventions.md`.
//! `thiserror` derives Display; `From<toml::de::Error>` etc. let callers
//! propagate parser failures with `?`.

use thiserror::Error;

/// All errors that can arise from parsing, registry lookup, capability
/// lookup, or instantiation of a template blueprint.
#[derive(Debug, Error)]
pub enum TemplateError {
    /// The bundled or supplied TOML failed to parse, or violated a
    /// schema-level constraint enforced at parse time (e.g.,
    /// `privacy: local` paired with `fallback_policy: use_default_route`,
    /// which is rejected because a local-only template cannot fall
    /// back to a cloud route).
    #[error("blueprint {id} is malformed: {reason}")]
    MalformedBlueprint { id: String, reason: String },

    /// The bundled blueprint table contains two entries that share an
    /// `id`. Caught by the build-time validation test, never expected
    /// at runtime.
    #[error("duplicate blueprint id: {id}")]
    DuplicateId { id: String },

    /// Referenced template id is not in the registry.
    #[error("unknown blueprint id: {id}")]
    UnknownId { id: String },

    /// A bundled sample report couldn't be located. Build-time check.
    #[error("missing sample report for blueprint {id}: {path}")]
    MissingSample { id: String, path: String },

    /// `toml` parser returned an error. The toml error's own message
    /// is stable and human-readable; we surface it verbatim.
    #[error("toml parse error in blueprint {id}: {source}")]
    Toml {
        id: String,
        #[source]
        source: toml::de::Error,
    },
}

impl TemplateError {
    /// Build a `MalformedBlueprint` from a free-form reason. Used for
    /// constraint violations that survive `serde::Deserialize` but
    /// must be rejected at parse time (privacy × fallback combos,
    /// mutually exclusive fields, etc.).
    pub fn malformed(id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::MalformedBlueprint {
            id: id.into(),
            reason: reason.into(),
        }
    }
}

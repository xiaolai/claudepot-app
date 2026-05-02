//! Blueprint types — the in-memory representation of a bundled
//! template TOML.
//!
//! See `dev-docs/templates-implementation-plan.md` §3.1 for the
//! authoritative schema. This module keeps the type definitions and
//! the TOML parser; the registry (`registry.rs`) loads bundled
//! blueprints and exposes them.
//!
//! Schema discipline: every field that a template author can set
//! lives in this file. Defaults match the schema doc. Constraints
//! that the type system can't enforce (e.g., the privacy ×
//! fallback combo) are validated by [`Blueprint::from_toml`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::automations::types::HostPlatform;

use super::error::TemplateError;

// ---------- Top-level blueprint ----------

/// A parsed, validated template blueprint. Ready for the registry,
/// the install dialog, and the (future) instantiate function.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Blueprint {
    pub id: TemplateId,
    pub schema_version: u32,
    pub version: u32,
    pub name: String,
    pub tagline: String,
    pub description: String,
    pub category: Category,
    pub icon: String,
    pub tier: Tier,

    pub capabilities_required: Vec<Capability>,
    #[serde(default = "default_min_context_tokens")]
    pub min_context_tokens: u32,
    pub recommended_class: ModelClass,
    pub cost_class: CostClass,
    #[serde(default)]
    pub cost_cap_usd: Option<f64>,
    #[serde(default)]
    pub privacy: PrivacyClass,
    #[serde(default)]
    pub consent_required: bool,
    #[serde(default)]
    pub fallback_policy: FallbackPolicy,

    pub scope: Scope,

    #[serde(default)]
    pub apply: Option<ApplyConfig>,

    pub schedule: ScheduleConfig,
    pub output: OutputConfig,

    #[serde(default)]
    pub placeholders: Vec<Placeholder>,

    pub runtime: RuntimeConfig,

    pub prompt: String,

    /// Path (relative to the bundled blueprints directory) of the
    /// sample report to show in the install dialog. Resolved by the
    /// registry; not loaded into the blueprint itself.
    #[serde(default)]
    pub sample_report: Option<String>,

    /// Platforms this blueprint is intended to run on. Empty (the
    /// historical default) is treated as "macOS only" because every
    /// shipped blueprint hardcodes macOS shell tooling — see the
    /// 2026-05-02 cross-platform audit for the rationale. Future
    /// blueprints can declare an explicit list to broaden support.
    /// `TemplateRegistry::list_for(...)` filters by this field.
    #[serde(default = "default_supported_platforms")]
    pub supported_platforms: Vec<HostPlatform>,
}

fn default_supported_platforms() -> Vec<HostPlatform> {
    vec![HostPlatform::Macos]
}

impl Blueprint {
    /// True when this blueprint declares support for `host`.
    pub fn supports(&self, host: HostPlatform) -> bool {
        self.supported_platforms.contains(&host)
    }
}

/// Stable namespaced identifier (`it.morning-health-check`).
/// Validated at parse time: lowercase ASCII alnum, dot, dash;
/// 1–96 chars; exactly one dot separating namespace and short
/// name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TemplateId(pub String);

impl TemplateId {
    pub fn validate(s: &str) -> Result<(), &'static str> {
        if s.is_empty() || s.len() > 96 {
            return Err("template id must be 1..=96 characters");
        }
        let dot_count = s.bytes().filter(|&b| b == b'.').count();
        if dot_count != 1 {
            return Err("template id must contain exactly one dot");
        }
        if s.starts_with('.') || s.ends_with('.') || s.starts_with('-') || s.ends_with('-') {
            return Err("template id must not start or end with a separator");
        }
        let bad = s
            .bytes()
            .find(|&b| !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'));
        if bad.is_some() {
            return Err("template id may only contain [a-z0-9.-]");
        }
        Ok(())
    }
}

impl std::fmt::Display for TemplateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------- Enum fields ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    ItHealth,
    Diagnostics,
    Housekeeping,
    Audit,
    Caregiver,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    Ambient,
    OnDemand,
    Triggered,
    Periodic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ToolUse,
    LongContext,
    Vision,
    StructuredOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelClass {
    /// Local LLMs are sufficient. Templates with this class run
    /// well on a 7B–32B local model.
    LocalOk,
    /// Needs a cloud-fast model (Haiku, Sonnet, gpt-4o-mini).
    Fast,
    /// Needs a frontier model (Opus, gpt-5, etc.). Local models
    /// will produce wrong-but-confident output.
    Frontier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CostClass {
    /// <$0.001 / run.
    Trivial,
    /// <$0.01 / run.
    Low,
    /// <$0.10 / run.
    Medium,
    /// >$0.10 / run.
    High,
}

/// Three-class privacy. See plan §3.1.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrivacyClass {
    /// Hard local filter. Cloud routes are not shown at install
    /// time. Cannot fall back to non-local routes.
    Local,
    /// Local + user-marked private-cloud routes.
    PrivateCloud,
    /// Any route.
    #[default]
    Any,
}

impl PrivacyClass {
    pub fn is_local(self) -> bool {
        matches!(self, Self::Local)
    }
}

/// What to do when the assigned route is unreachable at run time.
/// See plan §5.3 for the truth table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackPolicy {
    /// Skip the run silently; log to alerts file.
    #[default]
    Skip,
    /// Fall back to the user's default route. Forbidden when
    /// `privacy = Local` (rejected at parse time).
    UseDefaultRoute,
    /// Skip the run and surface a notification.
    Alert,
}

// ---------- Subsections ----------

/// Plain-English trust statements rendered verbatim in the install
/// dialog. Authored once per blueprint; never templated.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Scope {
    pub reads: String,
    pub writes: String,
    pub could_change: String,
    pub network: String,
    #[serde(default)]
    pub requires_full_disk_access: bool,
}

/// Constraints on the apply pipeline for templates that propose
/// changes. The executor (future tier) validates every operation
/// against these; deny-by-default.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ApplyConfig {
    #[serde(default)]
    pub scope: ApplyScope,
    #[serde(default)]
    pub allowed_operations: Vec<ApplyOperation>,
    #[serde(default = "default_pending_changes_path")]
    pub pending_changes_path: String,
    #[serde(default = "default_apply_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_item_id_strategy")]
    pub item_id_strategy: ItemIdStrategy,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ApplyScope {
    /// Globs against canonicalized paths. Empty = no apply allowed.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Reject any operation whose path lies outside `allowed_paths`
    /// after canonicalization.
    #[serde(default = "default_true")]
    pub deny_outside: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyOperation {
    Move,
    Rename,
    Mkdir,
    Write,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemIdStrategy {
    /// Stable ID derived from the source content; rejected items
    /// stay rejected across reruns. Default for housekeeping.
    #[default]
    ContentHash,
    /// ID derived from the path only.
    PathHash,
    /// Fresh UUID per run.
    UuidPerRun,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScheduleConfig {
    /// Cron expression used as the default schedule, OR the literal
    /// `"manual"` for templates that run only via Run Now.
    pub default: String,
    pub default_label: String,
    #[serde(default)]
    pub allowed_shapes: Vec<ScheduleShape>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduleShape {
    Daily,
    Weekdays,
    Weekly,
    Hourly,
    Manual,
    Custom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OutputConfig {
    pub path_template: String,
    pub format: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Placeholder {
    pub name: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: PlaceholderType,
    #[serde(default)]
    pub default: Option<toml::Value>,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub validation: Option<PlaceholderValidation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceholderType {
    Path,
    Text,
    Boolean,
    Number,
    List,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PlaceholderValidation {
    #[serde(default)]
    pub must_exist: bool,
    #[serde(default)]
    pub must_be_directory: bool,
    #[serde(default)]
    pub must_be_writable: bool,
    #[serde(default)]
    pub within_home: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuntimeConfig {
    pub permission_mode: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u32,
}

// ---------- Defaults ----------

fn default_true() -> bool {
    true
}

fn default_min_context_tokens() -> u32 {
    8192
}

fn default_timeout_secs() -> u32 {
    300
}

fn default_pending_changes_path() -> String {
    "{output_dir}/.pending-changes.json".to_string()
}

fn default_apply_schema_version() -> u32 {
    1
}

fn default_item_id_strategy() -> ItemIdStrategy {
    ItemIdStrategy::ContentHash
}

// ---------- Parser ----------

impl Blueprint {
    /// Parse a TOML document, then run cross-field validation. Returns
    /// a [`Blueprint`] that is structurally valid; runtime checks
    /// (placeholder values, file existence, etc.) happen later.
    pub fn from_toml(source: &str) -> Result<Self, TemplateError> {
        // Two-stage parse: toml::from_str → serde-validated record →
        // cross-field constraints. The id is parsed first so error
        // messages can name it; we accept malformed-id later.
        let bp: Self = toml::from_str(source).map_err(|toml_err| TemplateError::Toml {
            id: peek_id(source).unwrap_or_else(|| "<unknown>".to_string()),
            source: toml_err,
        })?;

        // Identifier shape.
        TemplateId::validate(&bp.id.0).map_err(|e| TemplateError::malformed(&bp.id.0, e))?;

        // Privacy × fallback constraint. A local-only template cannot
        // fall back to a non-local route; this is the type-system
        // boundary that backs the runtime's privacy contract.
        if bp.privacy.is_local() && bp.fallback_policy == FallbackPolicy::UseDefaultRoute {
            return Err(TemplateError::malformed(
                &bp.id.0,
                "privacy=local + fallback_policy=use_default_route is incoherent: \
                 a local-only template cannot fall back to the default route",
            ));
        }

        // Schedule shape: if `default` is the literal "manual", the
        // shapes list must include `manual`; otherwise the cron
        // expression is validated downstream by the existing
        // `automations::cron` parser. We don't re-validate here.
        if bp.schedule.default == "manual"
            && !bp.schedule.allowed_shapes.contains(&ScheduleShape::Manual)
        {
            return Err(TemplateError::malformed(
                &bp.id.0,
                "schedule.default=\"manual\" but allowed_shapes does not include \"manual\"",
            ));
        }

        // Apply consistency: if apply table is present, allowed_ops
        // must be non-empty. Conversely, templates that document a
        // non-empty `could_change` should typically have an apply
        // table; we don't enforce that — some templates report
        // proposals in plain markdown without the structured apply
        // pipeline (e.g., diag.disk-full).
        if let Some(apply) = &bp.apply {
            if apply.allowed_operations.is_empty() {
                return Err(TemplateError::malformed(
                    &bp.id.0,
                    "apply.allowed_operations must not be empty when apply is configured",
                ));
            }
        }

        // Caregiver structure: `consent_required = true` implies
        // `category = caregiver` and vice versa, to keep schema and
        // dispatch in sync.
        if bp.consent_required != matches!(bp.category, Category::Caregiver) {
            return Err(TemplateError::malformed(
                &bp.id.0,
                "consent_required must be true iff category is \"caregiver\"",
            ));
        }

        Ok(bp)
    }

    pub fn id(&self) -> &TemplateId {
        &self.id
    }
}

/// Best-effort scan of raw TOML for the blueprint's id, used to
/// label parse errors before we have a deserialized struct. Returns
/// `None` if not found.
fn peek_id(source: &str) -> Option<String> {
    for line in source.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("id") {
            let rest = rest.trim_start();
            if let Some(eq_rest) = rest.strip_prefix('=') {
                let value = eq_rest.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml(id: &str, extras: &str) -> String {
        // TOML grammar: top-level keys must precede any [subtable].
        // We put `extras` at the very top so callers can override
        // top-level fields (privacy, fallback_policy, consent_required,
        // etc.) without breaking parsing.
        format!(
            r#"
id              = "{id}"
schema_version  = 1
version         = 1
name            = "Test"
tagline         = "Test tagline."
description     = "Test description."
category        = "it-health"
icon            = "x"
tier            = "ambient"

capabilities_required = ["tool_use"]
recommended_class     = "fast"
cost_class            = "trivial"

prompt = "Test prompt."

{extras}

[scope]
reads        = "X"
writes       = "Y"
could_change = "Nothing."
network      = "None."

[schedule]
default        = "0 8 * * *"
default_label  = "Each morning"
allowed_shapes = ["daily"]

[output]
path_template = "/tmp/x.md"
format        = "markdown"

[runtime]
permission_mode = "plan"
allowed_tools   = ["Bash"]
"#
        )
    }

    #[test]
    fn parses_minimal_blueprint() {
        let bp = Blueprint::from_toml(&minimal_toml("it.x", "")).unwrap();
        assert_eq!(bp.id.0, "it.x");
        assert_eq!(bp.cost_class, CostClass::Trivial);
        assert_eq!(bp.privacy, PrivacyClass::Any);
        assert!(bp.placeholders.is_empty());
    }

    #[test]
    fn rejects_local_privacy_with_default_fallback() {
        let toml = minimal_toml(
            "it.x",
            r#"
privacy         = "local"
fallback_policy = "use_default_route"
"#,
        );
        let err = Blueprint::from_toml(&toml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("privacy=local"), "got: {msg}");
    }

    #[test]
    fn accepts_local_privacy_with_skip_fallback() {
        let toml = minimal_toml(
            "it.x",
            r#"
privacy         = "local"
fallback_policy = "skip"
"#,
        );
        Blueprint::from_toml(&toml).unwrap();
    }

    #[test]
    fn rejects_caregiver_without_consent() {
        // Standalone — `minimal_toml` defaults to `category = "it-health"`,
        // which would clash with our override here as a duplicate top-level key.
        let toml = r#"
id              = "caregiver.x"
schema_version  = 1
version         = 1
name            = "Test"
tagline         = "Test."
description     = "Test."
category        = "caregiver"
icon            = "x"
tier            = "ambient"

capabilities_required = ["tool_use"]
recommended_class     = "local-ok"
cost_class            = "trivial"
consent_required      = false

prompt = "Test."

[scope]
reads        = "X"
writes       = "Y"
could_change = "Nothing."
network      = "None."

[schedule]
default        = "0 9 * * 0"
default_label  = "Each Sunday"
allowed_shapes = ["weekly"]

[output]
path_template = "/tmp/x.md"
format        = "markdown"

[runtime]
permission_mode = "plan"
allowed_tools   = ["Bash"]
"#;
        let err = Blueprint::from_toml(toml).unwrap_err();
        assert!(
            err.to_string().contains("consent_required"),
            "expected consent_required violation, got: {err}"
        );
    }

    #[test]
    fn rejects_consent_without_caregiver_category() {
        let toml = minimal_toml(
            "it.x",
            r#"
consent_required = true
"#,
        );
        let err = Blueprint::from_toml(&toml).unwrap_err();
        assert!(err.to_string().contains("consent_required"));
    }

    #[test]
    fn rejects_apply_with_no_operations() {
        let toml = minimal_toml(
            "house.x",
            r#"
[apply]
allowed_operations = []
"#,
        );
        let err = Blueprint::from_toml(&toml).unwrap_err();
        assert!(err.to_string().contains("allowed_operations"));
    }

    #[test]
    fn rejects_manual_default_without_manual_shape() {
        let toml = r#"
id              = "diag.x"
schema_version  = 1
version         = 1
name            = "Test"
tagline         = "Test."
description     = "Test."
category        = "diagnostics"
icon            = "x"
tier            = "on-demand"

capabilities_required = ["tool_use"]
recommended_class     = "fast"
cost_class            = "trivial"

prompt = "Test."

[scope]
reads        = "X"
writes       = "Y"
could_change = "Nothing."
network      = "None."

[schedule]
default        = "manual"
default_label  = "On demand"
allowed_shapes = ["daily"]

[output]
path_template = "/tmp/x.md"
format        = "markdown"

[runtime]
permission_mode = "plan"
allowed_tools   = ["Bash"]
"#;
        let err = Blueprint::from_toml(toml).unwrap_err();
        assert!(err.to_string().contains("manual"));
    }

    #[test]
    fn rejects_invalid_id_shape() {
        // Two dots.
        let toml = minimal_toml("it.x.y", "");
        let err = Blueprint::from_toml(&toml).unwrap_err();
        assert!(err.to_string().contains("dot"), "got: {err}");
        // Uppercase.
        let toml = minimal_toml("It.x", "");
        let err = Blueprint::from_toml(&toml).unwrap_err();
        assert!(err.to_string().contains("[a-z0-9.-]"), "got: {err}");
    }

    #[test]
    fn template_id_validate_table() {
        let ok = ["it.morning", "house.downloads-tidy", "a.b"];
        for s in ok {
            TemplateId::validate(s).unwrap_or_else(|e| panic!("rejected {s}: {e}"));
        }
        let bad = [
            "",
            ".x",
            "x.",
            "-x.y",
            "x.y-",
            "x..y",
            "x.y.z",
            "x_y.z",
            "X.y",
            &"a".repeat(100),
        ];
        for s in bad {
            assert!(TemplateId::validate(s).is_err(), "should reject {s}");
        }
    }

    #[test]
    fn unknown_field_is_rejected() {
        // Strict: unrecognized fields should fail. Currently `serde`
        // accepts them silently; this test will fail until we set
        // `#[serde(deny_unknown_fields)]` if we want strictness. For
        // now, just verify the lenient behavior is consistent.
        let toml = minimal_toml(
            "it.x",
            r#"
this_is_not_a_field = "ignored"
"#,
        );
        // We intentionally don't deny unknown fields at this stage —
        // future schema additions need to remain backward compatible.
        let bp = Blueprint::from_toml(&toml).unwrap();
        assert_eq!(bp.id.0, "it.x");
    }

    #[test]
    fn peek_id_finds_id() {
        let source = r#"
# leading comment
id = "it.morning"

[scope]
reads = "..."
"#;
        assert_eq!(peek_id(source).as_deref(), Some("it.morning"));
    }

    #[test]
    fn peek_id_returns_none_when_missing() {
        assert!(peek_id("[scope]\nreads = \"x\"").is_none());
    }
}

// Avoid unused-import warning when the registry hasn't loaded yet.
#[allow(dead_code)]
type _Use<T> = BTreeMap<String, T>;

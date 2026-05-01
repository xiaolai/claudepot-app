//! DTOs for the templates Tauri command surface.
//!
//! These mirror the typed shapes in
//! `claudepot_core::templates` but flatten where convenient for
//! TS round-trips. The renderer never sees the raw Blueprint —
//! it works with [`TemplateSummaryDto`] in the gallery and
//! [`TemplateDetailsDto`] in the install dialog.

use serde::{Deserialize, Serialize};

use claudepot_core::templates::{
    instantiate as inst, Blueprint, Capability, Category, CostClass, FallbackPolicy, ModelClass,
    PrivacyClass, Tier,
};

/// One card in the gallery. Compact; the install dialog fetches
/// the full details on demand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateSummaryDto {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub category: String,
    pub icon: String,
    pub tier: String,
    pub cost_class: String,
    pub privacy: String,
    pub recommended_class: String,
    pub consent_required: bool,
    pub apply_supported: bool,
    pub default_schedule_label: String,
}

impl From<&Blueprint> for TemplateSummaryDto {
    fn from(bp: &Blueprint) -> Self {
        Self {
            id: bp.id().0.clone(),
            name: bp.name.clone(),
            tagline: bp.tagline.clone(),
            category: render_category(bp.category),
            icon: bp.icon.clone(),
            tier: render_tier(bp.tier),
            cost_class: render_cost_class(bp.cost_class),
            privacy: render_privacy(bp.privacy),
            recommended_class: render_model_class(bp.recommended_class),
            consent_required: bp.consent_required,
            apply_supported: bp.apply.is_some(),
            default_schedule_label: bp.schedule.default_label.clone(),
        }
    }
}

/// All the data the install dialog needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateDetailsDto {
    pub summary: TemplateSummaryDto,
    pub schema_version: u32,
    pub version: u32,
    pub description: String,
    pub scope: ScopeDto,
    pub capabilities_required: Vec<String>,
    pub min_context_tokens: u32,
    pub fallback_policy: String,
    pub default_schedule_cron: String,
    pub allowed_schedule_shapes: Vec<String>,
    pub output_path_template: String,
    pub output_format: String,
    pub placeholders: Vec<PlaceholderDto>,
    /// True when the template's runtime config requires the
    /// shell to have macOS Full Disk Access.
    pub requires_full_disk_access: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeDto {
    pub reads: String,
    pub writes: String,
    pub could_change: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceholderDto {
    pub name: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl From<&Blueprint> for TemplateDetailsDto {
    fn from(bp: &Blueprint) -> Self {
        let placeholders = bp
            .placeholders
            .iter()
            .map(|ph| PlaceholderDto {
                name: ph.name.clone(),
                label: ph.label.clone(),
                kind: render_placeholder_type(ph.kind),
                required: ph.required,
                default: ph
                    .default
                    .as_ref()
                    .and_then(|v| serde_json::to_value(v).ok()),
                help: ph.help.clone(),
            })
            .collect();
        Self {
            summary: bp.into(),
            schema_version: bp.schema_version,
            version: bp.version,
            description: bp.description.clone(),
            scope: ScopeDto {
                reads: bp.scope.reads.clone(),
                writes: bp.scope.writes.clone(),
                could_change: bp.scope.could_change.clone(),
                network: bp.scope.network.clone(),
            },
            capabilities_required: bp
                .capabilities_required
                .iter()
                .map(|c| render_capability(*c))
                .collect(),
            min_context_tokens: bp.min_context_tokens,
            fallback_policy: render_fallback(bp.fallback_policy),
            default_schedule_cron: bp.schedule.default.clone(),
            allowed_schedule_shapes: bp
                .schedule
                .allowed_shapes
                .iter()
                .map(|s| render_schedule_shape(*s))
                .collect(),
            output_path_template: bp.output.path_template.clone(),
            output_format: bp.output.format.clone(),
            placeholders,
            requires_full_disk_access: bp.scope.requires_full_disk_access,
        }
    }
}

/// Wire shape for installing a template. Pairs with
/// `claudepot_core::templates::TemplateInstance`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInstanceDto {
    pub blueprint_id: String,
    pub blueprint_schema_version: u32,
    #[serde(default)]
    pub placeholder_values: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub route_id: Option<String>,
    pub schedule: ScheduleDto,
    #[serde(default)]
    pub name_override: Option<String>,
}

/// Schedule shape on the wire. TS `kind` discriminator matches
/// the Rust enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleDto {
    Daily { time: String },
    Weekdays { time: String },
    Weekly { day: String, time: String },
    Hourly { every_n_hours: u32 },
    Manual,
    Custom { cron: String },
}

impl ScheduleDto {
    pub fn into_core(self) -> Result<inst::ScheduleDto, String> {
        Ok(match self {
            ScheduleDto::Daily { time } => inst::ScheduleDto::Daily { time },
            ScheduleDto::Weekdays { time } => inst::ScheduleDto::Weekdays { time },
            ScheduleDto::Weekly { day, time } => {
                let weekday = match day.to_lowercase().as_str() {
                    "sun" | "sunday" | "0" => inst::Weekday::Sun,
                    "mon" | "monday" | "1" => inst::Weekday::Mon,
                    "tue" | "tuesday" | "2" => inst::Weekday::Tue,
                    "wed" | "wednesday" | "3" => inst::Weekday::Wed,
                    "thu" | "thursday" | "4" => inst::Weekday::Thu,
                    "fri" | "friday" | "5" => inst::Weekday::Fri,
                    "sat" | "saturday" | "6" => inst::Weekday::Sat,
                    other => return Err(format!("unknown weekday: {other}")),
                };
                inst::ScheduleDto::Weekly { day: weekday, time }
            }
            ScheduleDto::Hourly { every_n_hours } => {
                inst::ScheduleDto::Hourly { every_n_hours }
            }
            ScheduleDto::Manual => inst::ScheduleDto::Manual,
            ScheduleDto::Custom { cron } => inst::ScheduleDto::Custom { cron },
        })
    }
}

// ---------- Renderers ----------

fn render_category(c: Category) -> String {
    match c {
        Category::ItHealth => "it-health",
        Category::Diagnostics => "diagnostics",
        Category::Housekeeping => "housekeeping",
        Category::Audit => "audit",
        Category::Caregiver => "caregiver",
        Category::Network => "network",
    }
    .to_string()
}

fn render_tier(t: Tier) -> String {
    match t {
        Tier::Ambient => "ambient",
        Tier::OnDemand => "on-demand",
        Tier::Triggered => "triggered",
        Tier::Periodic => "periodic",
    }
    .to_string()
}

fn render_cost_class(c: CostClass) -> String {
    match c {
        CostClass::Trivial => "trivial",
        CostClass::Low => "low",
        CostClass::Medium => "medium",
        CostClass::High => "high",
    }
    .to_string()
}

fn render_privacy(p: PrivacyClass) -> String {
    match p {
        PrivacyClass::Local => "local",
        PrivacyClass::PrivateCloud => "private-cloud",
        PrivacyClass::Any => "any",
    }
    .to_string()
}

fn render_model_class(m: ModelClass) -> String {
    match m {
        ModelClass::LocalOk => "local-ok",
        ModelClass::Fast => "fast",
        ModelClass::Frontier => "frontier",
    }
    .to_string()
}

fn render_capability(c: Capability) -> String {
    match c {
        Capability::ToolUse => "tool_use",
        Capability::LongContext => "long_context",
        Capability::Vision => "vision",
        Capability::StructuredOutput => "structured_output",
    }
    .to_string()
}

fn render_fallback(f: FallbackPolicy) -> String {
    match f {
        FallbackPolicy::Skip => "skip",
        FallbackPolicy::UseDefaultRoute => "use_default_route",
        FallbackPolicy::Alert => "alert",
    }
    .to_string()
}

fn render_schedule_shape(s: claudepot_core::templates::ScheduleShape) -> String {
    use claudepot_core::templates::ScheduleShape::*;
    match s {
        Daily => "daily",
        Weekdays => "weekdays",
        Weekly => "weekly",
        Hourly => "hourly",
        Manual => "manual",
        Custom => "custom",
    }
    .to_string()
}

fn render_placeholder_type(t: claudepot_core::templates::PlaceholderType) -> String {
    use claudepot_core::templates::PlaceholderType::*;
    match t {
        Path => "path",
        Text => "text",
        Boolean => "boolean",
        Number => "number",
        List => "list",
    }
    .to_string()
}

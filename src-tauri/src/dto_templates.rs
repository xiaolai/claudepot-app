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

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire-format pinning: the renderer's TypeScript discriminated
    /// unions hard-code these exact strings (kebab-case for the user-
    /// facing classes; snake_case for capability tags). If a Rust
    /// enum gains a variant or a render arm is renamed, this test
    /// fails before the bug reaches the renderer's `switch` statement.
    #[test]
    fn category_renders_kebab_case() {
        assert_eq!(render_category(Category::ItHealth), "it-health");
        assert_eq!(render_category(Category::Diagnostics), "diagnostics");
        assert_eq!(render_category(Category::Housekeeping), "housekeeping");
        assert_eq!(render_category(Category::Audit), "audit");
        assert_eq!(render_category(Category::Caregiver), "caregiver");
        assert_eq!(render_category(Category::Network), "network");
    }

    #[test]
    fn tier_renders_kebab_case_on_demand() {
        // OnDemand is the canonical drift trap: snake_case in Rust,
        // kebab-case on the wire.
        assert_eq!(render_tier(Tier::OnDemand), "on-demand");
        assert_eq!(render_tier(Tier::Ambient), "ambient");
        assert_eq!(render_tier(Tier::Triggered), "triggered");
        assert_eq!(render_tier(Tier::Periodic), "periodic");
    }

    #[test]
    fn privacy_class_renders_kebab_case() {
        assert_eq!(render_privacy(PrivacyClass::PrivateCloud), "private-cloud");
        assert_eq!(render_privacy(PrivacyClass::Local), "local");
        assert_eq!(render_privacy(PrivacyClass::Any), "any");
    }

    #[test]
    fn model_class_renders_kebab_case() {
        assert_eq!(render_model_class(ModelClass::LocalOk), "local-ok");
        assert_eq!(render_model_class(ModelClass::Fast), "fast");
        assert_eq!(render_model_class(ModelClass::Frontier), "frontier");
    }

    #[test]
    fn cost_class_renders_lowercase_words() {
        assert_eq!(render_cost_class(CostClass::Trivial), "trivial");
        assert_eq!(render_cost_class(CostClass::Low), "low");
        assert_eq!(render_cost_class(CostClass::Medium), "medium");
        assert_eq!(render_cost_class(CostClass::High), "high");
    }

    #[test]
    fn capability_renders_snake_case() {
        // Capability tags travel as snake_case to match the
        // blueprint TOML format and the validator's literal matches.
        assert_eq!(render_capability(Capability::ToolUse), "tool_use");
        assert_eq!(render_capability(Capability::LongContext), "long_context");
        assert_eq!(render_capability(Capability::Vision), "vision");
        assert_eq!(
            render_capability(Capability::StructuredOutput),
            "structured_output",
        );
    }

    #[test]
    fn fallback_policy_renders_snake_case() {
        assert_eq!(render_fallback(FallbackPolicy::Skip), "skip");
        assert_eq!(
            render_fallback(FallbackPolicy::UseDefaultRoute),
            "use_default_route",
        );
        assert_eq!(render_fallback(FallbackPolicy::Alert), "alert");
    }

    #[test]
    fn schedule_dto_into_core_accepts_short_lowercase_weekdays() {
        let s = ScheduleDto::Weekly { day: "mon".into(), time: "09:00".into() };
        let core = s.into_core().expect("mon should parse");
        match core {
            inst::ScheduleDto::Weekly { day, time } => {
                assert_eq!(day, inst::Weekday::Mon);
                assert_eq!(time, "09:00");
            }
            other => panic!("expected Weekly, got {other:?}"),
        }
    }

    #[test]
    fn schedule_dto_into_core_accepts_full_weekdays() {
        let s = ScheduleDto::Weekly { day: "Sunday".into(), time: "09:00".into() };
        match s.into_core().unwrap() {
            inst::ScheduleDto::Weekly { day, .. } => {
                assert_eq!(day, inst::Weekday::Sun);
            }
            _ => panic!("expected Weekly"),
        }
    }

    #[test]
    fn schedule_dto_into_core_accepts_numeric_weekdays() {
        // The TS picker emits short names today, but accepting "0".."6"
        // keeps a valid escape hatch for cron-savvy users / future
        // surfaces. This test pins that intent.
        let s = ScheduleDto::Weekly { day: "5".into(), time: "07:30".into() };
        match s.into_core().unwrap() {
            inst::ScheduleDto::Weekly { day, .. } => {
                assert_eq!(day, inst::Weekday::Fri);
            }
            _ => panic!("expected Weekly"),
        }
    }

    #[test]
    fn schedule_dto_into_core_rejects_unknown_weekday() {
        let s = ScheduleDto::Weekly { day: "wendsdayy".into(), time: "09:00".into() };
        let err = s.into_core().expect_err("typo should not parse");
        assert!(err.contains("unknown weekday"));
    }

    #[test]
    fn schedule_dto_serializes_with_tagged_kind() {
        // Wire format: { "kind": "daily", "time": "08:00" }
        let s = ScheduleDto::Daily { time: "08:00".into() };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["kind"], "daily");
        assert_eq!(json["time"], "08:00");
    }

    #[test]
    fn schedule_dto_manual_serializes_with_just_kind() {
        let s = ScheduleDto::Manual;
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["kind"], "manual");
    }

    #[test]
    fn schedule_shape_renders_lowercase() {
        use claudepot_core::templates::ScheduleShape::*;
        assert_eq!(render_schedule_shape(Daily), "daily");
        assert_eq!(render_schedule_shape(Weekdays), "weekdays");
        assert_eq!(render_schedule_shape(Weekly), "weekly");
        assert_eq!(render_schedule_shape(Hourly), "hourly");
        assert_eq!(render_schedule_shape(Manual), "manual");
        assert_eq!(render_schedule_shape(Custom), "custom");
    }

    #[test]
    fn placeholder_type_renders_lowercase() {
        use claudepot_core::templates::PlaceholderType::*;
        assert_eq!(render_placeholder_type(Path), "path");
        assert_eq!(render_placeholder_type(Text), "text");
        assert_eq!(render_placeholder_type(Boolean), "boolean");
        assert_eq!(render_placeholder_type(Number), "number");
        assert_eq!(render_placeholder_type(List), "list");
    }

    #[test]
    fn placeholder_dto_serializes_kind_as_type_field() {
        // The Rust struct member is `kind` (avoiding the `type`
        // reserved word) but the wire shape MUST be `"type"` —
        // pin it.
        let p = PlaceholderDto {
            name: "p".into(),
            label: "P".into(),
            kind: "path".into(),
            required: true,
            default: None,
            help: None,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert!(json.get("type").is_some(), "wire field must be `type`");
        assert!(json.get("kind").is_none(), "no `kind` leak on the wire");
    }

    #[test]
    fn template_instance_dto_round_trips_with_optional_fields_omitted() {
        // The renderer omits placeholder_values, route_id, and
        // name_override when they aren't set. The serde defaults
        // make those round-trip safe.
        let json = serde_json::json!({
            "blueprint_id": "it.morning-health-check",
            "blueprint_schema_version": 1,
            "schedule": { "kind": "daily", "time": "08:00" },
        });
        let parsed: TemplateInstanceDto = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.blueprint_id, "it.morning-health-check");
        assert_eq!(parsed.blueprint_schema_version, 1);
        assert!(parsed.placeholder_values.is_empty());
        assert!(parsed.route_id.is_none());
        assert!(parsed.name_override.is_none());
        assert!(matches!(parsed.schedule, ScheduleDto::Daily { .. }));
    }
}

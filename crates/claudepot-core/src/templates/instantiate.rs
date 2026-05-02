//! Materialize a [`Blueprint`] + user input into an
//! `AutomationCreateDto` ready for the existing automations
//! runtime.
//!
//! Per `dev-docs/templates-implementation-plan.md` §5.4.
//!
//! Flow:
//!
//! 1. Validate every placeholder value against the blueprint's
//!    placeholder schema (type match, required-ness, path
//!    sanity).
//! 2. Substitute placeholders into the prompt and the output
//!    path template (`{name}` → user value).
//! 3. Convert the user-chosen schedule shape to a cron string
//!    (or to `manual` for `Trigger::Manual` automations).
//! 4. Project blueprint runtime fields into the DTO shape.
//!
//! Routes are not resolved here — the DTO carries `binary_kind`
//! and `binary_route_id`, and the install command on the Tauri
//! side translates that to `AutomationBinary::FirstParty` or
//! `AutomationBinary::Route { route_id }`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::blueprint::{
    Blueprint, Placeholder, PlaceholderType, PlaceholderValidation, ScheduleShape,
};
use super::error::TemplateError;

/// User-supplied input for instantiating one template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInstance {
    pub blueprint_id: String,
    pub blueprint_schema_version: u32,
    #[serde(default)]
    pub placeholder_values: BTreeMap<String, PlaceholderValue>,
    /// Stringified UUID of the assigned route. `None` falls back
    /// to the user's primary `claude` binary.
    #[serde(default)]
    pub route_id: Option<String>,
    pub schedule: ScheduleDto,
    #[serde(default)]
    pub name_override: Option<String>,
}

/// Type-tagged placeholder value. Mirrors the blueprint's
/// `placeholders[].type` enum.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlaceholderValue {
    Path { value: String },
    Text { value: String },
    Boolean { value: bool },
    Number { value: f64 },
    List { value: Vec<String> },
}

impl PlaceholderValue {
    fn matches_kind(&self, kind: PlaceholderType) -> bool {
        matches!(
            (self, kind),
            (PlaceholderValue::Path { .. }, PlaceholderType::Path)
                | (PlaceholderValue::Text { .. }, PlaceholderType::Text)
                | (PlaceholderValue::Boolean { .. }, PlaceholderType::Boolean)
                | (PlaceholderValue::Number { .. }, PlaceholderType::Number)
                | (PlaceholderValue::List { .. }, PlaceholderType::List)
        )
    }

    /// Render as a string for prompt / path substitution. Lists
    /// join with commas; Booleans render as `true` / `false`;
    /// Numbers use Rust's default `{}` format.
    fn render(&self) -> String {
        match self {
            PlaceholderValue::Path { value } | PlaceholderValue::Text { value } => value.clone(),
            PlaceholderValue::Boolean { value } => value.to_string(),
            PlaceholderValue::Number { value } => value.to_string(),
            PlaceholderValue::List { value } => value.join(", "),
        }
    }
}

/// User-chosen schedule. Cron-shaped variants serialize to a
/// concrete cron string at instantiation time. `Manual` is the
/// only path that produces a `Trigger::Manual` automation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleDto {
    Daily {
        time: String,
    },
    Weekdays {
        time: String,
    },
    Weekly {
        day: Weekday,
        time: String,
    },
    Hourly {
        every_n_hours: u32,
    },
    Manual,
    /// Power-user escape hatch. UI surfaces this only behind an
    /// "Advanced" disclosure.
    Custom {
        cron: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Weekday {
    Sun,
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
}

impl Weekday {
    fn cron_field(self) -> u8 {
        match self {
            Weekday::Sun => 0,
            Weekday::Mon => 1,
            Weekday::Tue => 2,
            Weekday::Wed => 3,
            Weekday::Thu => 4,
            Weekday::Fri => 5,
            Weekday::Sat => 6,
        }
    }
}

/// Resolved cron expression and associated trigger kind. Returned
/// by [`schedule_to_cron`] for the install path to feed into the
/// existing [`AutomationCreateDto`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSchedule {
    /// `"cron"` or `"manual"`.
    pub trigger_kind: String,
    /// Empty when `trigger_kind == "manual"`.
    pub cron: String,
}

/// Convert a [`ScheduleDto`] into a concrete cron string (or
/// signal Manual). Validates `time` format `HH:MM` and
/// `every_n_hours` range `1..=23`.
pub fn schedule_to_cron(s: &ScheduleDto) -> Result<ResolvedSchedule, TemplateError> {
    fn parse_hhmm(s: &str) -> Result<(u8, u8), String> {
        let (h, m) = s
            .split_once(':')
            .ok_or_else(|| format!("expected HH:MM, got {s:?}"))?;
        let h: u8 = h.parse().map_err(|_| format!("bad hour: {h}"))?;
        let m: u8 = m.parse().map_err(|_| format!("bad minute: {m}"))?;
        if h >= 24 {
            return Err(format!("hour out of range: {h}"));
        }
        if m >= 60 {
            return Err(format!("minute out of range: {m}"));
        }
        Ok((h, m))
    }

    let resolved = match s {
        ScheduleDto::Daily { time } => {
            let (h, m) = parse_hhmm(time).map_err(|e| TemplateError::malformed("<schedule>", e))?;
            ResolvedSchedule {
                trigger_kind: "cron".into(),
                cron: format!("{m} {h} * * *"),
            }
        }
        ScheduleDto::Weekdays { time } => {
            let (h, m) = parse_hhmm(time).map_err(|e| TemplateError::malformed("<schedule>", e))?;
            ResolvedSchedule {
                trigger_kind: "cron".into(),
                cron: format!("{m} {h} * * 1-5"),
            }
        }
        ScheduleDto::Weekly { day, time } => {
            let (h, m) = parse_hhmm(time).map_err(|e| TemplateError::malformed("<schedule>", e))?;
            ResolvedSchedule {
                trigger_kind: "cron".into(),
                cron: format!("{m} {h} * * {}", day.cron_field()),
            }
        }
        ScheduleDto::Hourly { every_n_hours } => {
            if !(1..=23).contains(every_n_hours) {
                return Err(TemplateError::malformed(
                    "<schedule>",
                    format!("every_n_hours out of range 1..=23: {every_n_hours}"),
                ));
            }
            ResolvedSchedule {
                trigger_kind: "cron".into(),
                cron: format!("0 */{every_n_hours} * * *"),
            }
        }
        ScheduleDto::Manual => ResolvedSchedule {
            trigger_kind: "manual".into(),
            cron: String::new(),
        },
        ScheduleDto::Custom { cron } => {
            // The runtime cron parser will have its own validation;
            // we don't duplicate it here. Empty strings are rejected.
            if cron.trim().is_empty() {
                return Err(TemplateError::malformed(
                    "<schedule>",
                    "custom cron must be non-empty",
                ));
            }
            ResolvedSchedule {
                trigger_kind: "cron".into(),
                cron: cron.clone(),
            }
        }
    };
    Ok(resolved)
}

/// Resolved DTO that mirrors the Tauri-side
/// `AutomationCreateDto`. Returned by [`instantiate`] as a
/// transport-agnostic shape; the Tauri command translates this
/// into the wire DTO and creates the automation.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAutomation {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// `"first_party"` or `"route"`.
    pub binary_kind: String,
    pub binary_route_id: Option<String>,
    pub model: Option<String>,
    pub cwd: String,
    pub prompt: String,
    pub permission_mode: String,
    pub allowed_tools: Vec<String>,
    pub trigger_kind: String,
    pub cron: String,
    pub timezone: Option<String>,
    pub max_budget_usd: Option<f64>,
    pub log_retention_runs: u32,
    pub template_id: String,
    /// Resolved output path with placeholders + `{date}`-style
    /// slots already substituted.
    pub output_path: String,
}

/// Resolve a blueprint + instance into a concrete automation
/// shape. Pure function — no I/O. Validation errors carry the
/// blueprint id for context.
pub fn instantiate(
    blueprint: &Blueprint,
    instance: &TemplateInstance,
) -> Result<ResolvedAutomation, TemplateError> {
    // 1. id + version sanity
    if instance.blueprint_id != blueprint.id().0 {
        return Err(TemplateError::malformed(
            blueprint.id().0.clone(),
            format!(
                "instance id {:?} does not match blueprint id {:?}",
                instance.blueprint_id,
                blueprint.id().0
            ),
        ));
    }
    if instance.blueprint_schema_version != blueprint.schema_version {
        return Err(TemplateError::malformed(
            blueprint.id().0.clone(),
            format!(
                "instance schema_version {} does not match blueprint schema_version {}",
                instance.blueprint_schema_version, blueprint.schema_version
            ),
        ));
    }

    // 2. Validate placeholder values against schema.
    let resolved_values = resolve_placeholders(blueprint, &instance.placeholder_values)?;

    // 3. Substitute placeholders into prompt + output path. Both
    //    use the same `{name}` syntax. Unknown placeholders are
    //    left in place (they may be `{date}`-style runtime tokens
    //    the prompt or shim handles).
    let prompt = substitute(&blueprint.prompt, &resolved_values);
    let output_path = substitute(&blueprint.output.path_template, &resolved_values);

    // 4. Schedule.
    let resolved_schedule = schedule_to_cron(&instance.schedule)?;

    // 5. Manual triggers and the blueprint's allowed_shapes must
    //    agree. Honor the blueprint contract.
    if matches!(instance.schedule, ScheduleDto::Manual)
        && !blueprint
            .schedule
            .allowed_shapes
            .contains(&ScheduleShape::Manual)
    {
        return Err(TemplateError::malformed(
            blueprint.id().0.clone(),
            "instance.schedule is Manual but blueprint does not allow Manual shape",
        ));
    }

    // 6. Binary selection.
    let (binary_kind, binary_route_id) = match instance.route_id.as_deref() {
        Some(rid) => ("route".to_string(), Some(rid.to_string())),
        None => ("first_party".to_string(), None),
    };

    // 7. Display + slug.
    //
    // `name` is the internal slug used for uniqueness and path
    // derivation — `validate_name` requires lowercase ASCII
    // alphanumerics + dashes. Blueprint names like "Disk is full —
    // what's eating space?" can't be slugs, so derive one from the
    // blueprint id's suffix (the part after the dot). The
    // user-facing label lives in `display_name`.
    let derived_slug = blueprint
        .id()
        .0
        .split_once('.')
        .map(|(_, s)| s.to_string())
        .unwrap_or_else(|| blueprint.id().0.clone());
    let name = instance
        .name_override
        .as_ref()
        .map(|n| slugify(n))
        .unwrap_or(derived_slug);
    let display_name = instance
        .name_override
        .clone()
        .or_else(|| Some(blueprint.name.clone()));

    Ok(ResolvedAutomation {
        name,
        display_name,
        description: Some(blueprint.tagline.clone()),
        binary_kind,
        binary_route_id,
        model: None, // route-default; user can override via the wrapper script
        cwd: home_dir_string(),
        prompt,
        permission_mode: blueprint.runtime.permission_mode.clone(),
        allowed_tools: blueprint.runtime.allowed_tools.clone(),
        trigger_kind: resolved_schedule.trigger_kind,
        cron: resolved_schedule.cron,
        timezone: None,
        max_budget_usd: blueprint.cost_cap_usd,
        log_retention_runs: 30,
        template_id: blueprint.id().0.clone(),
        output_path,
    })
}

fn home_dir_string() -> String {
    dirs::home_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "/".to_string())
}

/// Slugify a free-form name into a `validate_name`-safe form.
/// Lowercases ASCII letters and digits, collapses anything else
/// to a single dash, trims leading/trailing dashes, and caps at
/// 64 chars (matching the slug rule). The result must still be
/// non-empty and start with `[a-z0-9]`; if the input collapses
/// to nothing useful, returns `template`.
fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_was_dash = false;
    for c in input.chars() {
        let lo = c.to_ascii_lowercase();
        if lo.is_ascii_lowercase() || lo.is_ascii_digit() {
            out.push(lo);
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return "template".to_string();
    }
    if out.len() > 64 {
        out.truncate(64);
        while out.ends_with('-') {
            out.pop();
        }
    }
    // Slug must start with [a-z0-9]; if the first byte after
    // dash-trimming is somehow not, prefix `t-`.
    if let Some(first) = out.chars().next() {
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return format!("t-{out}");
        }
    }
    out
}

/// Validate user-supplied placeholder values against the
/// blueprint's schema. Returns a flat `name → rendered string`
/// map for substitution.
fn resolve_placeholders(
    blueprint: &Blueprint,
    values: &BTreeMap<String, PlaceholderValue>,
) -> Result<BTreeMap<String, String>, TemplateError> {
    let mut out = BTreeMap::new();

    for ph in &blueprint.placeholders {
        match (values.get(&ph.name), ph.required) {
            (None, true) => {
                if let Some(default) = build_default(ph)? {
                    out.insert(ph.name.clone(), default);
                } else {
                    return Err(TemplateError::malformed(
                        blueprint.id().0.clone(),
                        format!("missing required placeholder {:?}", ph.name),
                    ));
                }
            }
            (None, false) => {
                if let Some(default) = build_default(ph)? {
                    out.insert(ph.name.clone(), default);
                }
            }
            (Some(v), _) => {
                if !v.matches_kind(ph.kind) {
                    return Err(TemplateError::malformed(
                        blueprint.id().0.clone(),
                        format!(
                            "placeholder {:?} expected type {:?} but got value of a different kind",
                            ph.name, ph.kind
                        ),
                    ));
                }
                if let PlaceholderValue::Path { value } = v {
                    if let Some(rule) = ph.validation.as_ref() {
                        validate_path_placeholder(blueprint, &ph.name, value, rule)?;
                    }
                }
                out.insert(ph.name.clone(), v.render());
            }
        }
    }
    Ok(out)
}

fn build_default(ph: &Placeholder) -> Result<Option<String>, TemplateError> {
    match ph.default.as_ref() {
        None => Ok(None),
        Some(toml::Value::String(s)) => Ok(Some(s.clone())),
        Some(toml::Value::Boolean(b)) => Ok(Some(b.to_string())),
        Some(toml::Value::Integer(i)) => Ok(Some(i.to_string())),
        Some(toml::Value::Float(f)) => Ok(Some(f.to_string())),
        Some(toml::Value::Array(arr)) => {
            // Coerce a list of strings; reject non-strings for now.
            let mut items = Vec::with_capacity(arr.len());
            for item in arr {
                match item {
                    toml::Value::String(s) => items.push(s.clone()),
                    other => {
                        return Err(TemplateError::malformed(
                            "<placeholder>",
                            format!(
                                "list-default entries must be strings; got {other:?} in placeholder {:?}",
                                ph.name
                            ),
                        ));
                    }
                }
            }
            Ok(Some(items.join(", ")))
        }
        Some(other) => Err(TemplateError::malformed(
            "<placeholder>",
            format!(
                "unsupported default-value type for placeholder {:?}: {other:?}",
                ph.name
            ),
        )),
    }
}

/// Apply the validation rules a path-typed placeholder declared.
/// `must_exist` and `must_be_directory` require I/O; we honor
/// both. `within_home` walks the path's canonical form and
/// rejects anything outside `$HOME`. `must_be_writable` is
/// approximated by checking that the path's parent is writable
/// when the path itself doesn't exist yet.
fn validate_path_placeholder(
    blueprint: &Blueprint,
    name: &str,
    raw: &str,
    rule: &PlaceholderValidation,
) -> Result<(), TemplateError> {
    let path = expand_user(raw);

    if rule.must_exist && !path.exists() {
        return Err(TemplateError::malformed(
            blueprint.id().0.clone(),
            format!("placeholder {name:?} requires an existing path: {raw}"),
        ));
    }
    if rule.must_be_directory && path.exists() && !path.is_dir() {
        return Err(TemplateError::malformed(
            blueprint.id().0.clone(),
            format!("placeholder {name:?} requires a directory: {raw}"),
        ));
    }
    if rule.within_home {
        let home = dirs::home_dir().ok_or_else(|| {
            TemplateError::malformed(
                blueprint.id().0.clone(),
                "cannot resolve home directory for `within_home` validation",
            )
        })?;
        let canonical = path.canonicalize().unwrap_or(path.clone());
        let canonical_home = home.canonicalize().unwrap_or(home);
        if !canonical.starts_with(&canonical_home) {
            return Err(TemplateError::malformed(
                blueprint.id().0.clone(),
                format!(
                    "placeholder {name:?} must be within $HOME; got {} (resolved {})",
                    raw,
                    canonical.display()
                ),
            ));
        }
    }
    if rule.must_be_writable {
        // Approximation: if the path exists, check write perms via
        // a metadata query. Otherwise check the parent. We don't
        // actually try to open() — this is a pre-flight gate.
        let target = if path.exists() {
            path.clone()
        } else {
            path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(path.clone())
        };
        let meta = std::fs::metadata(&target).map_err(|e| {
            TemplateError::malformed(
                blueprint.id().0.clone(),
                format!(
                    "placeholder {name:?} is not writable (cannot stat {}): {e}",
                    target.display()
                ),
            )
        })?;
        if meta.permissions().readonly() {
            return Err(TemplateError::malformed(
                blueprint.id().0.clone(),
                format!(
                    "placeholder {name:?} target is read-only: {}",
                    target.display()
                ),
            ));
        }
    }
    Ok(())
}

fn expand_user(raw: &str) -> std::path::PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    std::path::PathBuf::from(raw)
}

/// Replace `{name}` tokens with their resolved values. Unknown
/// names pass through verbatim — runtime tokens like `{date}`
/// or `{run_id}` are filled in by downstream code.
fn substitute(template: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = template[i..].find('}') {
                let inner = &template[i + 1..i + end];
                if let Some(v) = values.get(inner) {
                    out.push_str(v);
                    i += end + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::registry::TemplateRegistry;

    fn morning() -> Blueprint {
        let r = TemplateRegistry::load_bundled().unwrap();
        r.get("it.morning-health-check").unwrap().clone()
    }

    fn instance(schedule: ScheduleDto) -> TemplateInstance {
        TemplateInstance {
            blueprint_id: "it.morning-health-check".into(),
            blueprint_schema_version: 1,
            placeholder_values: BTreeMap::new(),
            route_id: None,
            schedule,
            name_override: None,
        }
    }

    #[test]
    fn instantiates_morning_health_check_with_daily_schedule() {
        let bp = morning();
        let inst = instance(ScheduleDto::Daily {
            time: "08:00".into(),
        });
        let resolved = instantiate(&bp, &inst).unwrap();
        assert_eq!(resolved.trigger_kind, "cron");
        assert_eq!(resolved.cron, "0 8 * * *");
        assert_eq!(resolved.binary_kind, "first_party");
        assert!(resolved.binary_route_id.is_none());
        assert_eq!(resolved.template_id, "it.morning-health-check");
        assert!(resolved.prompt.contains("morning health check"));
        // Slug is the blueprint id's suffix (after the dot);
        // display name is the human-readable blueprint name.
        assert_eq!(resolved.name, "morning-health-check");
        assert_eq!(
            resolved.display_name.as_deref(),
            Some("Morning health check")
        );
    }

    #[test]
    fn slugify_handles_freeform_punctuation() {
        assert_eq!(
            slugify("Disk is full — what's eating space?"),
            "disk-is-full-what-s-eating-space"
        );
        assert_eq!(slugify("  --foo  --bar--  "), "foo-bar");
        assert_eq!(slugify("123abc"), "123abc");
        assert_eq!(slugify("___"), "template");
        assert_eq!(slugify(""), "template");
    }

    #[test]
    fn slug_from_diagnostic_blueprint_id() {
        let r = TemplateRegistry::load_bundled().unwrap();
        let bp = r.get("diag.disk-full").unwrap().clone();
        let mut inst = instance(ScheduleDto::Manual);
        inst.blueprint_id = "diag.disk-full".into();
        let resolved = instantiate(&bp, &inst).unwrap();
        assert_eq!(resolved.name, "disk-full");
        assert!(resolved.display_name.is_some());
    }

    #[test]
    fn schedule_dto_to_cron_table() {
        for (dto, expected) in [
            (
                ScheduleDto::Daily {
                    time: "08:00".into(),
                },
                ("cron", "0 8 * * *"),
            ),
            (
                ScheduleDto::Weekdays {
                    time: "09:30".into(),
                },
                ("cron", "30 9 * * 1-5"),
            ),
            (
                ScheduleDto::Weekly {
                    day: Weekday::Sun,
                    time: "07:00".into(),
                },
                ("cron", "0 7 * * 0"),
            ),
            (
                ScheduleDto::Hourly { every_n_hours: 4 },
                ("cron", "0 */4 * * *"),
            ),
            (
                ScheduleDto::Custom {
                    cron: "*/5 * * * *".into(),
                },
                ("cron", "*/5 * * * *"),
            ),
        ] {
            let r = schedule_to_cron(&dto).unwrap();
            assert_eq!(r.trigger_kind, expected.0, "{dto:?}");
            assert_eq!(r.cron, expected.1, "{dto:?}");
        }

        let manual = schedule_to_cron(&ScheduleDto::Manual).unwrap();
        assert_eq!(manual.trigger_kind, "manual");
        assert_eq!(manual.cron, "");
    }

    #[test]
    fn rejects_invalid_time_format() {
        let err = schedule_to_cron(&ScheduleDto::Daily {
            time: "not-a-time".into(),
        })
        .unwrap_err();
        assert!(err.to_string().contains("HH:MM"));
    }

    #[test]
    fn rejects_out_of_range_hour() {
        let err = schedule_to_cron(&ScheduleDto::Daily {
            time: "25:00".into(),
        })
        .unwrap_err();
        assert!(err.to_string().contains("hour"));
    }

    #[test]
    fn rejects_hourly_zero() {
        let err = schedule_to_cron(&ScheduleDto::Hourly { every_n_hours: 0 }).unwrap_err();
        assert!(err.to_string().contains("range"));
    }

    #[test]
    fn rejects_blueprint_id_mismatch() {
        let bp = morning();
        let mut inst = instance(ScheduleDto::Daily {
            time: "08:00".into(),
        });
        inst.blueprint_id = "wrong.id".into();
        let err = instantiate(&bp, &inst).unwrap_err();
        assert!(err.to_string().contains("does not match"));
    }

    #[test]
    fn rejects_blueprint_schema_version_mismatch() {
        let bp = morning();
        let mut inst = instance(ScheduleDto::Daily {
            time: "08:00".into(),
        });
        inst.blueprint_schema_version = 999;
        let err = instantiate(&bp, &inst).unwrap_err();
        assert!(err.to_string().contains("schema_version"));
    }

    #[test]
    fn route_binding_propagates() {
        let bp = morning();
        let mut inst = instance(ScheduleDto::Daily {
            time: "08:00".into(),
        });
        inst.route_id = Some("00000000-0000-0000-0000-000000000001".into());
        let resolved = instantiate(&bp, &inst).unwrap();
        assert_eq!(resolved.binary_kind, "route");
        assert_eq!(
            resolved.binary_route_id.as_deref(),
            Some("00000000-0000-0000-0000-000000000001")
        );
    }

    #[test]
    fn substitute_replaces_known_tokens_only() {
        let mut values = BTreeMap::new();
        values.insert("download_path".into(), "/tmp/downloads".into());
        let out = substitute("Scan {download_path}; today is {date}; bye.", &values);
        // Known token replaced; unknown `{date}` passes through.
        assert_eq!(out, "Scan /tmp/downloads; today is {date}; bye.");
    }

    #[test]
    fn placeholder_value_kind_check() {
        assert!(PlaceholderValue::Path { value: "/x".into() }.matches_kind(PlaceholderType::Path));
        assert!(
            !PlaceholderValue::Path { value: "/x".into() }.matches_kind(PlaceholderType::Number)
        );
    }

    #[test]
    fn manual_schedule_requires_blueprint_allowance() {
        let bp = morning();
        // Morning health check's allowed_shapes includes "manual",
        // so this should succeed.
        let inst = instance(ScheduleDto::Manual);
        let resolved = instantiate(&bp, &inst).unwrap();
        assert_eq!(resolved.trigger_kind, "manual");
    }
}

//! Per-variable resolution, the value projection, and the three buckets.
//!
//! # Why core keeps raw JSON and the projection is separate
//!
//! An `Option<String>` cannot represent a value the spec does not understand,
//! so it would coerce one into the nearest chooser state and write over the
//! user's data at the next interaction. A `Custom(String)` is not enough
//! either: `env` children are arbitrary JSON, and a `String` cannot hold a
//! number, bool, array, object, or null without the same coercion.
//!
//! So core reads the raw [`serde_json::Value`] and [`project`] is a
//! deliberate *projection* — and the projection is where the disclosure rule
//! lives. Anything the pane cannot faithfully edit renders read-only with its
//! raw value and two explicit actions; anything whose contents might hide a
//! credential renders without them.

use crate::cc_env::settings;
use crate::cc_env::spec::{self, EnvControl, EnvVarSpec};
use serde::Serialize;
use serde_json::{Map, Value as JsonValue};

/// The JSON shape a value actually had. Reported even when the contents are
/// withheld, because "someone put an object here" is itself the finding.
#[derive(Clone, Copy, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EnvValueKind {
    String,
    Number,
    Bool,
    Array,
    Object,
    Null,
}

impl EnvValueKind {
    fn of(v: &JsonValue) -> Self {
        match v {
            JsonValue::String(_) => Self::String,
            JsonValue::Number(_) => Self::Number,
            JsonValue::Bool(_) => Self::Bool,
            JsonValue::Array(_) => Self::Array,
            JsonValue::Object(_) => Self::Object,
            JsonValue::Null => Self::Null,
        }
    }
}

/// What the renderer is allowed to know about one variable's value.
#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EnvValue {
    /// No key. The row reads "No settings.json override" — never "CC default",
    /// which is a claim about the user's shell that we cannot see.
    Absent,
    /// Set, and the variable can carry a credential. **No value, ever** — not
    /// truncated, not previewed.
    SecretSet,
    /// A string the control can round-trip.
    Known { value: String },
    /// A scalar the control cannot round-trip: `"true"` on a toggle, `"12x"`
    /// on a number, an unknown enum member, or an explicit `""`. Rendered
    /// read-only with its raw value plus **Replace** and **Clear**, so the
    /// next interaction cannot silently coerce it.
    Custom { raw: String, kind: EnvValueKind },
    /// An array, object, or null. Same treatment as [`EnvValue::Custom`]
    /// minus the value: an unrecognized nested structure could contain
    /// anything, including a credential.
    CustomOpaque { kind: EnvValueKind },
    /// A key in nobody's list. Shown as set, with the value withheld — an
    /// unknown name may be a credential nobody has documented yet.
    Withheld { kind: EnvValueKind },
}

impl EnvValue {
    pub fn is_set(&self) -> bool {
        !matches!(self, Self::Absent)
    }
}

/// Whether a string round-trips through this control unchanged.
///
/// An explicit empty string never does, for any control: it is a distinct
/// process state from unset, and a text box showing "" is indistinguishable
/// from one showing nothing.
fn round_trips(spec: &EnvVarSpec, s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    match spec.control {
        EnvControl::Text => true,
        // The same check the writer uses. Two different integer tests would
        // let a value the writer accepts come back as non-round-trippable —
        // the row would render read-only for a value it had just written.
        EnvControl::Number => settings::is_integer_syntax(s),
        EnvControl::Enum | EnvControl::Toggle => spec
            .values
            .as_ref()
            .is_some_and(|vals| vals.iter().any(|v| v == s)),
    }
}

/// Project a raw settings value onto what the renderer may see.
pub fn project(spec: &EnvVarSpec, value: Option<&JsonValue>) -> EnvValue {
    let Some(value) = value else {
        return EnvValue::Absent;
    };
    // The declared-secret check comes first and covers every JSON type. A
    // credential inside an array is still a credential.
    if spec.safety.secret {
        return EnvValue::SecretSet;
    }
    // And the check the flag cannot make: a token sitting in a variable
    // nobody classified as credential-bearing. `ANTHROPIC_BASE_URL` is a URL
    // by every rule in the spec, and a user who pasted a key into it has
    // still put a key in a file we are about to serialize over IPC. Withhold
    // on content, not only on classification.
    if let JsonValue::String(s) = value {
        if crate::cc_env::errors::looks_like_a_credential(s) {
            return EnvValue::SecretSet;
        }
    }
    match value {
        JsonValue::String(s) if round_trips(spec, s) => EnvValue::Known { value: s.clone() },
        JsonValue::String(s) => EnvValue::Custom {
            raw: s.clone(),
            kind: EnvValueKind::String,
        },
        JsonValue::Number(n) => EnvValue::Custom {
            raw: n.to_string(),
            kind: EnvValueKind::Number,
        },
        JsonValue::Bool(b) => EnvValue::Custom {
            raw: b.to_string(),
            kind: EnvValueKind::Bool,
        },
        other => EnvValue::CustomOpaque {
            kind: EnvValueKind::of(other),
        },
    }
}

/// Which file, if any, we can see setting this variable.
///
/// Deliberately not "where the value comes from": the user's shell is a
/// source we cannot read, so the third arm says what we know rather than
/// what we would like to say.
#[derive(Clone, Copy, Debug, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedSource {
    /// `~/.claude/settings.json` — the file this pane edits, and the winner
    /// wherever it and `~/.claude.json` disagree.
    SettingsOverride,
    /// Only `~/.claude.json` sets it. Clearing a settings override would let
    /// this value surface, which is why the confirmation has to say so.
    LegacyGlobal,
    /// No file we read sets it. **Not** the same as "CC's default is in
    /// effect" — a shell export would beat both files and is invisible here.
    NoKnownFileOverride,
}

/// One documented variable, resolved.
#[derive(Clone, Debug, Serialize)]
pub struct EnvVarState {
    pub spec: &'static EnvVarSpec,
    /// What this pane edits.
    pub settings_value: EnvValue,
    /// `~/.claude.json`'s value, projected under the same secret rule.
    pub legacy_global: Option<EnvValue>,
    pub resolved_source: ResolvedSource,
}

/// A key in the user's `env` that is in no documented list.
///
/// Not in the stated requirements, and a correctness requirement anyway:
/// without it a hand-set key is invisible in a pane that claims to show env
/// config.
#[derive(Clone, Debug, Serialize)]
pub struct UnrecognizedEntry {
    pub name: String,
    /// Always [`EnvValue::Withheld`]. Values are withheld by default because
    /// an unknown key may be a credential nobody has documented yet.
    pub value: EnvValue,
}

/// The undocumented-names appendix, or an explicit statement that it cannot
/// be shown.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UndocumentedBucket {
    /// The installed Claude Code is exactly the version the snapshot was
    /// taken from.
    Available {
        snapshot_version: String,
        names: Vec<String>,
    },
    /// Any other case. Renders the section shell with "unavailable for this
    /// version" — never stale names. Undocumented names are non-monotonic, so
    /// a nearest-version match would be a confident-sounding lie rather than
    /// an approximation.
    Unavailable {
        snapshot_version: String,
        installed_version: Option<String>,
    },
}

/// Everything the pane needs, in one pass over the files.
#[derive(Clone, Debug, Serialize)]
pub struct EnvOverview {
    pub documented: Vec<EnvVarState>,
    pub undocumented: UndocumentedBucket,
    pub unrecognized: Vec<UnrecognizedEntry>,
    /// Provenance, surfaced in the (i) disclosure. Two facts, not one.
    /// Category keys and labels in grouping order, straight from the
    /// generated artifact.
    pub categories: Vec<spec::EnvCategory>,
    pub docs_fetched_at: String,
    pub docs_sha256: String,
    pub binary_crosscheck_version: String,
    /// Which Claude Code the cross-check was compared against, and where it
    /// lives — a user with several installs can see which one was measured.
    pub installed_version: Option<String>,
    pub installed_path: Option<String>,
    /// The settings file this pane actually edits, resolved.
    ///
    /// Not a constant `~/.claude/settings.json`: `CLAUDE_CONFIG_DIR` moves it,
    /// and telling a user to hand-edit a file that is not the one being
    /// written is worse than saying nothing. `rules/path-display.md` wants
    /// paths readable in full; the pane renders this selectable.
    pub settings_path: String,
    /// Whether `present_in_build` may be trusted at all. On a mismatch no
    /// documented row is hidden or tagged "not in build" on the strength of a
    /// snapshot that does not describe the running build.
    pub crosscheck_is_exact: bool,
}

/// Resolve every documented variable plus the two appendix buckets.
///
/// `env` is the settings map (already read), `legacy` is `~/.claude.json`'s,
/// and `installed` is the running Claude Code's version and path — passed in
/// rather than probed here so this stays a pure function over its inputs.
pub fn resolve_all(
    env: &Map<String, JsonValue>,
    legacy: &Map<String, JsonValue>,
    installed_version: Option<&str>,
    installed_path: Option<&str>,
) -> EnvOverview {
    let s = spec::spec();

    let documented: Vec<EnvVarState> = s
        .vars
        .iter()
        .map(|var| {
            let settings_value = project(var, env.get(&var.name));
            let legacy_value = legacy.get(&var.name).map(|v| project(var, Some(v)));
            let resolved_source = if settings_value.is_set() {
                ResolvedSource::SettingsOverride
            } else if legacy_value.as_ref().is_some_and(EnvValue::is_set) {
                ResolvedSource::LegacyGlobal
            } else {
                ResolvedSource::NoKnownFileOverride
            };
            EnvVarState {
                spec: var,
                settings_value,
                legacy_global: legacy_value,
                resolved_source,
            }
        })
        .collect();

    let exact = s.crosscheck_validity(installed_version).is_exact();
    let undocumented = if exact {
        UndocumentedBucket::Available {
            snapshot_version: s.binary_crosscheck_version.clone(),
            names: s.undocumented_in_build.clone(),
        }
    } else {
        UndocumentedBucket::Unavailable {
            snapshot_version: s.binary_crosscheck_version.clone(),
            installed_version: installed_version.map(str::to_string),
        }
    };

    // Third bucket: set by hand, documented nowhere. The undocumented
    // snapshot counts as "a list it is in" only when that snapshot describes
    // the running build — otherwise the name has no known home and belongs
    // here, where it is at least visible and clearable.
    let known_undocumented: std::collections::HashSet<&str> = if exact {
        s.undocumented_in_build.iter().map(String::as_str).collect()
    } else {
        std::collections::HashSet::new()
    };
    let mut unrecognized: Vec<UnrecognizedEntry> = env
        .iter()
        .filter(|(name, _)| {
            !spec::is_documented(name) && !known_undocumented.contains(name.as_str())
        })
        .map(|(name, value)| UnrecognizedEntry {
            name: name.clone(),
            value: EnvValue::Withheld {
                kind: EnvValueKind::of(value),
            },
        })
        .collect();
    unrecognized.sort_by(|a, b| a.name.cmp(&b.name));

    EnvOverview {
        documented,
        undocumented,
        unrecognized,
        categories: s.categories.clone(),
        docs_fetched_at: s.docs_fetched_at.clone(),
        docs_sha256: s.docs_sha256.clone(),
        binary_crosscheck_version: s.binary_crosscheck_version.clone(),
        installed_version: installed_version.map(str::to_string),
        installed_path: installed_path.map(str::to_string),
        settings_path: settings::user_settings_path().to_string_lossy().to_string(),
        crosscheck_is_exact: exact,
    }
}

/// Read both files and resolve. The I/O sibling of [`resolve_all`].
///
/// Goes through [`spec::try_spec`] rather than the infallible accessor, so a
/// build whose embedded artifact is unusable reports `MalformedSpec` instead
/// of rendering as "this Claude Code documents no environment variables" — a
/// statement that would be false and unactionable at the same time.
pub fn load(
    installed_version: Option<&str>,
    installed_path: Option<&str>,
) -> Result<EnvOverview, crate::cc_env::CcEnvError> {
    spec::try_spec()?;
    let env = settings::read_env_map(&settings::user_settings_path())?;
    let legacy = settings::read_legacy_global_env();
    Ok(resolve_all(
        &env,
        &legacy,
        installed_version,
        installed_path,
    ))
}

/// Which Claude Code the cross-check is being compared against, and where it
/// lives.
///
/// `resolve_active_cli_binary` picks the CLI that would actually run — first
/// match on `PATH`. `cc_doctor`'s own candidate search is the fallback for
/// when nothing is on `PATH`; using it as the *primary* would let the version
/// cross-check describe a build the user never runs, which §6.6 spends its
/// whole argument on not doing.
///
/// Lives in core rather than in the Tauri command because "which binary counts
/// as yours" is a policy decision, and `rules/architecture.md` puts those here.
pub fn resolve_installed_claude() -> (Option<String>, Option<String>) {
    let probe = crate::cc_doctor::probes::probe_version();
    if let Some(active) = crate::updates::detect::resolve_active_cli_binary() {
        // When the probe describes a *different* install, report the path we
        // resolved and no version rather than pairing a path with someone
        // else's number. An unknown version renders the undocumented bucket
        // unavailable, which is the right outcome for "not measured".
        let version = probe.filter(|p| p.binary_path == active).map(|p| p.version);
        return (version, Some(active.to_string_lossy().to_string()));
    }
    match probe {
        Some(p) => (
            Some(p.version),
            Some(p.binary_path.to_string_lossy().to_string()),
        ),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(pairs: &[(&str, JsonValue)]) -> Map<String, JsonValue> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn state_of<'a>(o: &'a EnvOverview, name: &str) -> &'a EnvVarState {
        o.documented.iter().find(|v| v.spec.name == name).unwrap()
    }

    #[test]
    fn a_string_that_round_trips_is_known() {
        let s = spec::lookup("MAX_THINKING_TOKENS").unwrap();
        assert_eq!(
            project(s, Some(&json!("1024"))),
            EnvValue::Known {
                value: "1024".into()
            }
        );
    }

    #[test]
    fn scalars_the_control_cannot_round_trip_become_custom_never_coerced() {
        let toggle = spec::lookup("USE_BUILTIN_RIPGREP").unwrap();
        assert_eq!(
            project(toggle, Some(&json!("true"))),
            EnvValue::Custom {
                raw: "true".into(),
                kind: EnvValueKind::String
            }
        );
        let number = spec::lookup("MAX_THINKING_TOKENS").unwrap();
        assert_eq!(
            project(number, Some(&json!("12x"))),
            EnvValue::Custom {
                raw: "12x".into(),
                kind: EnvValueKind::String
            }
        );
        let enumv = spec::lookup("CLAUDE_CODE_EFFORT_LEVEL").unwrap();
        assert!(matches!(
            project(enumv, Some(&json!("ludicrous"))),
            EnvValue::Custom { .. }
        ));
        // A JSON number is not a JSON string, and CC's schema wants a string.
        assert_eq!(
            project(number, Some(&json!(1024))),
            EnvValue::Custom {
                raw: "1024".into(),
                kind: EnvValueKind::Number
            }
        );
        assert_eq!(
            project(toggle, Some(&json!(true))),
            EnvValue::Custom {
                raw: "true".into(),
                kind: EnvValueKind::Bool
            }
        );
    }

    #[test]
    fn an_explicit_empty_string_is_custom_not_absent() {
        let s = spec::lookup("ANTHROPIC_BASE_URL").unwrap();
        assert_eq!(
            project(s, Some(&json!(""))),
            EnvValue::Custom {
                raw: String::new(),
                kind: EnvValueKind::String
            }
        );
        assert_ne!(project(s, Some(&json!(""))), EnvValue::Absent);
        assert!(project(s, Some(&json!(""))).is_set());
    }

    #[test]
    fn nested_shapes_report_their_kind_and_withhold_their_contents() {
        let s = spec::lookup("ANTHROPIC_BASE_URL").unwrap();
        for (value, kind) in [
            (json!({"a": "secret-ish"}), EnvValueKind::Object),
            (json!(["secret-ish"]), EnvValueKind::Array),
            (JsonValue::Null, EnvValueKind::Null),
        ] {
            let projected = project(s, Some(&value));
            assert_eq!(projected, EnvValue::CustomOpaque { kind });
            let json = serde_json::to_string(&projected).unwrap();
            assert!(!json.contains("secret-ish"), "{json}");
        }
    }

    #[test]
    fn a_secret_never_projects_its_value_whatever_its_json_type() {
        let s = spec::lookup("ANTHROPIC_API_KEY").unwrap();
        for value in [
            json!("sk-ant-oat01-leak"),
            json!(["sk-ant-oat01-leak"]),
            json!({"k": "sk-ant-oat01-leak"}),
            json!(42),
        ] {
            let projected = project(s, Some(&value));
            assert_eq!(projected, EnvValue::SecretSet);
            assert!(!serde_json::to_string(&projected)
                .unwrap()
                .contains("sk-ant"));
        }
        assert_eq!(project(s, None), EnvValue::Absent);
    }

    /// The leak the `secret` flag cannot see: a token pasted into a variable
    /// the spec classifies as an ordinary URL. Withholding is decided on
    /// content as well as classification.
    #[test]
    fn a_token_under_a_non_secret_variable_is_withheld_too() {
        let s = spec::lookup("ANTHROPIC_BASE_URL").unwrap();
        assert!(!s.safety.secret);
        let projected = project(s, Some(&json!("sk-ant-oat01-pasted")));
        assert_eq!(projected, EnvValue::SecretSet);
        assert!(!serde_json::to_string(&projected)
            .unwrap()
            .contains("sk-ant"));

        // And an ordinary value for the same variable still round-trips.
        assert_eq!(
            project(s, Some(&json!("https://api.example.com"))),
            EnvValue::Known {
                value: "https://api.example.com".into()
            }
        );
    }

    /// "Can write" and "can display" must agree. A number beyond `i64` is
    /// syntactically fine and Claude Code reads it with `parseInt`, so the
    /// writer accepts it — and the projection has to as well, or the row
    /// would render read-only for a value it had just written.
    #[test]
    fn a_number_the_writer_accepts_also_round_trips_in_the_projection() {
        let s = spec::lookup("MAX_THINKING_TOKENS").unwrap();
        for big in ["99999999999999999999", "-99999999999999999999", "0", "-1"] {
            assert!(
                crate::cc_env::settings::validate(s, big).is_ok(),
                "{big} should be writable"
            );
            assert_eq!(
                project(s, Some(&json!(big))),
                EnvValue::Known {
                    value: big.to_string()
                },
                "{big} should round-trip"
            );
        }
        // And a non-integer is rejected on both sides.
        assert!(crate::cc_env::settings::validate(s, "12x").is_err());
        assert!(matches!(
            project(s, Some(&json!("12x"))),
            EnvValue::Custom { .. }
        ));
    }

    #[test]
    fn absent_reports_no_known_file_override_not_cc_default() {
        let o = resolve_all(&Map::new(), &Map::new(), None, None);
        let v = state_of(&o, "MAX_THINKING_TOKENS");
        assert_eq!(v.settings_value, EnvValue::Absent);
        assert_eq!(v.resolved_source, ResolvedSource::NoKnownFileOverride);
        assert!(v.legacy_global.is_none());
    }

    #[test]
    fn legacy_global_is_reported_when_only_claude_json_sets_a_variable() {
        let legacy = map(&[("MAX_THINKING_TOKENS", json!("4096"))]);
        let o = resolve_all(&Map::new(), &legacy, None, None);
        let v = state_of(&o, "MAX_THINKING_TOKENS");
        assert_eq!(v.settings_value, EnvValue::Absent);
        assert_eq!(v.resolved_source, ResolvedSource::LegacyGlobal);
        assert_eq!(
            v.legacy_global,
            Some(EnvValue::Known {
                value: "4096".into()
            })
        );
    }

    #[test]
    fn settings_wins_over_the_legacy_global_but_both_are_reported() {
        let env = map(&[("MAX_THINKING_TOKENS", json!("1"))]);
        let legacy = map(&[("MAX_THINKING_TOKENS", json!("2"))]);
        let o = resolve_all(&env, &legacy, None, None);
        let v = state_of(&o, "MAX_THINKING_TOKENS");
        assert_eq!(v.resolved_source, ResolvedSource::SettingsOverride);
        assert_eq!(v.legacy_global, Some(EnvValue::Known { value: "2".into() }));
    }

    #[test]
    fn a_secret_in_the_legacy_global_is_redacted_too() {
        let legacy = map(&[("ANTHROPIC_API_KEY", json!("sk-ant-oat01-leak"))]);
        let o = resolve_all(&Map::new(), &legacy, None, None);
        let v = state_of(&o, "ANTHROPIC_API_KEY");
        assert_eq!(v.legacy_global, Some(EnvValue::SecretSet));
        assert!(!serde_json::to_string(&o).unwrap().contains("sk-ant"));
    }

    #[test]
    fn a_hand_set_undocumented_key_lands_in_the_unrecognized_bucket_with_no_value() {
        let env = map(&[("TOTALLY_MADE_UP", json!("could-be-a-token"))]);
        let o = resolve_all(&env, &Map::new(), None, None);
        assert_eq!(o.unrecognized.len(), 1);
        assert_eq!(o.unrecognized[0].name, "TOTALLY_MADE_UP");
        assert_eq!(
            o.unrecognized[0].value,
            EnvValue::Withheld {
                kind: EnvValueKind::String
            }
        );
        assert!(!serde_json::to_string(&o)
            .unwrap()
            .contains("could-be-a-token"));
    }

    #[test]
    fn a_documented_key_never_lands_in_the_unrecognized_bucket() {
        let env = map(&[("MAX_THINKING_TOKENS", json!("1"))]);
        let o = resolve_all(&env, &Map::new(), None, None);
        assert!(o.unrecognized.is_empty());
    }

    #[test]
    fn the_undocumented_bucket_is_available_only_on_an_exact_version_match() {
        let snapshot = &spec::spec().binary_crosscheck_version;

        let o = resolve_all(&Map::new(), &Map::new(), Some(snapshot), None);
        assert!(o.crosscheck_is_exact);
        match &o.undocumented {
            UndocumentedBucket::Available { names, .. } => assert!(!names.is_empty()),
            other => panic!("expected Available, got {other:?}"),
        }

        for installed in [Some("99.0.0"), None] {
            let o = resolve_all(&Map::new(), &Map::new(), installed, None);
            assert!(!o.crosscheck_is_exact);
            match &o.undocumented {
                UndocumentedBucket::Unavailable {
                    snapshot_version, ..
                } => assert_eq!(snapshot_version, snapshot),
                other => panic!("expected Unavailable, got {other:?}"),
            }
            // Never leak a stale name through the serialized payload.
            let json = serde_json::to_string(&o).unwrap();
            let a_stale_name = &spec::spec().undocumented_in_build[0];
            assert!(!json.contains(a_stale_name.as_str()), "stale name rendered");
        }
    }

    #[test]
    fn on_a_version_mismatch_a_known_undocumented_name_shows_as_unrecognized() {
        let name = spec::spec().undocumented_in_build[0].clone();
        let env = map(&[(name.as_str(), json!("1"))]);

        let exact = spec::spec().binary_crosscheck_version.clone();
        let matched = resolve_all(&env, &Map::new(), Some(&exact), None);
        assert!(
            matched.unrecognized.is_empty(),
            "on an exact match the snapshot already accounts for it"
        );

        let mismatched = resolve_all(&env, &Map::new(), Some("99.0.0"), None);
        assert_eq!(mismatched.unrecognized.len(), 1);
        assert_eq!(mismatched.unrecognized[0].name, name);
    }

    /// The gate that has to pass before any renderer work: build an overview
    /// over a settings file that sets **every** secret-capable variable — both
    /// `SAFE_ENV_VARS` overlaps included — plus a nested object value and a
    /// `~/.claude.json` secret, serialize it, and assert not one secret byte
    /// survives.
    #[test]
    fn no_secret_byte_survives_serialization() {
        const MARKER: &str = "SECRET-CANARY-9f3a";
        let secrets: Vec<&EnvVarSpec> = spec::spec()
            .vars
            .iter()
            .filter(|v| v.safety.secret)
            .collect();
        assert!(secrets.len() >= 12, "expected the full secret set");
        assert!(
            secrets
                .iter()
                .any(|v| v.name == "ANTHROPIC_CUSTOM_HEADERS" && v.safety.pretrust_safe),
            "overlap case 1 must be in the fixture"
        );
        assert!(
            secrets
                .iter()
                .any(|v| v.name == "ANTHROPIC_FOUNDRY_API_KEY" && v.safety.pretrust_safe),
            "overlap case 2 must be in the fixture"
        );

        let mut env = Map::new();
        for (i, v) in secrets.iter().enumerate() {
            // Cycle the JSON type too: a credential inside an array or an
            // object is still a credential.
            let value = match i % 4 {
                0 => json!(format!("sk-ant-{MARKER}")),
                1 => json!([format!("bearer {MARKER}")]),
                2 => json!({ "Authorization": format!("Bearer {MARKER}") }),
                _ => json!(format!("{MARKER}")),
            };
            env.insert(v.name.clone(), value);
        }
        // A nested object on a NON-secret variable: the contents are withheld
        // because an unrecognized structure could hide anything.
        env.insert(
            "ANTHROPIC_BASE_URL".to_string(),
            json!({ "sneaky": MARKER }),
        );
        // And an unrecognized key, whose value is withheld by default.
        env.insert("SOME_FUTURE_CREDENTIAL".to_string(), json!(MARKER));

        let legacy = map(&[("ANTHROPIC_AUTH_TOKEN", json!(format!("legacy-{MARKER}")))]);

        let overview = resolve_all(&env, &legacy, None, None);
        let payload = serde_json::to_string(&overview).unwrap();
        assert!(!payload.contains(MARKER), "a secret crossed the boundary");
        assert!(!payload.contains("sk-ant"), "a secret crossed the boundary");

        // And the rows still say the variables ARE set — withholding a value
        // must not read as "nothing here".
        for v in &secrets {
            assert_eq!(
                state_of(&overview, &v.name).settings_value,
                EnvValue::SecretSet,
                "{} should read as set-but-withheld",
                v.name
            );
        }
    }

    #[test]
    fn provenance_travels_with_the_overview() {
        let o = resolve_all(&Map::new(), &Map::new(), Some("2.0.0"), Some("/tmp/claude"));
        assert_eq!(o.docs_fetched_at, spec::spec().docs_fetched_at);
        assert_eq!(o.installed_path.as_deref(), Some("/tmp/claude"));
        assert_eq!(o.installed_version.as_deref(), Some("2.0.0"));
    }
}

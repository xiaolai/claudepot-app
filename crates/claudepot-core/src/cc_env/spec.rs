//! The embedded Claude Code env-var spec.
//!
//! `crates/claudepot-core/data/cc-env-spec.json` is produced by
//! `scripts/build-cc-env-spec.py` from committed evidence and embedded here
//! with `include_str!`. `cargo xtask verify-docs` re-runs that script with
//! `--check`, which regenerates the artifact from the same evidence and
//! compares byte-for-byte — a checksum next to its own artifact would only
//! prove the two were edited together.
//!
//! # Provenance is two facts, never one
//!
//! [`EnvSpec::docs_fetched_at`] / [`EnvSpec::docs_sha256`] describe the
//! official docs page. [`EnvSpec::binary_crosscheck_version`] describes one
//! Claude Code binary that happened to be installed when the evidence was
//! rebuilt. They have different lifetimes — the live docs already list
//! variables absent from any given build — so nothing here may be labelled
//! "documented for 2.1.220".
//!
//! [`EnvSpec::undocumented_in_build`] and every [`EnvVarSpec::present_in_build`]
//! flag are facts about **that one binary only**, and are valid solely on an
//! exact version match. Undocumented names are non-monotonic: Claude Code can
//! rename or delete one in any release, so a nearest-version match would be a
//! confident-sounding lie rather than an approximation. See
//! [`CrosscheckValidity`].

use crate::cc_env::errors::CcEnvError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

const SPEC_JSON: &str = include_str!("../../data/cc-env-spec.json");

/// What control the pane should render for a variable.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EnvControl {
    /// Two- or three-state switch. See [`EnvVarSpec::off`] for which.
    Toggle,
    /// Closed value set — a select.
    Enum,
    /// Integer input. The spec carries no min/max, and inventing bounds
    /// would reject valid values, so validation is syntax only.
    Number,
    /// Free text. [`EnvVarSpec::format`] is a display hint, never validation.
    Text,
}

/// A specific, evidenced way a value can hurt you.
///
/// The first three are Claude Code's own taxonomy, quoted from the comment
/// above `SAFE_ENV_VARS` in `utils/managedEnvConstants.ts`. The next two are
/// Claudepot's, for classes CC does not enumerate. [`Hazard::Unknown`] is the
/// honest label for "not on CC's pre-trust allowlist, specific risk not
/// established" — absence from that list says something is risky without
/// saying what, and naming a specific risk anyway would be the same sin as
/// guessing a control type.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Hazard {
    /// Redirect traffic to an attacker-controlled server.
    Redirect,
    /// Trust an attacker-controlled server (TLS verification / CA bundle).
    TrustCert,
    /// Value lands in a spawned command line.
    ExecuteCode,
    /// Switch to an attacker-controlled project / account.
    SwitchProject,
    /// Silently stops security updates from landing.
    DisableUpdates,
    /// Risk unestablished. Renders as a conservative generic note, never an
    /// invented label.
    Unknown,
}

/// Why a variable is not editable from this pane.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    /// Writing it splits Claude Code's own bootstrap: CC resolves its config
    /// directory from `process.env` *before* settings load and memoizes it,
    /// then applies `settings.env` over it — so some paths resolve against
    /// the old directory and later ones against the new. Claudepot's own
    /// target file does not move either way (`paths.rs` reads the same
    /// variable from its own process env). Set it in your shell.
    BootstrapSplitBrain,
    /// Injected per run by the host or the subprocess launcher, never
    /// user-set. Rendered read-only rather than hidden, so a hand-set key is
    /// never invisible in a pane that claims to show env config.
    HostInjected,
    /// Claude Code reads it **only** from the process environment `claude`
    /// was started with, and explicitly not from a settings `env` block —
    /// which is the only block this pane writes.
    ///
    /// So the variable is perfectly real and perfectly settable; it just
    /// cannot be set *here*. Writing it would land the key in
    /// `settings.json`, report success, and change nothing — the same silent
    /// no-op as writing a `cleanupPeriodDays` value CC rejects. The pane
    /// shows it read-only and says where it does work.
    EnvOnlyNotSettings,
}

/// Independent safety attributes — deliberately **not** exclusive tiers.
///
/// Claude Code's own sets overlap, and they answer different questions.
/// `SAFE_ENV_VARS` means "safe to apply from an untrusted source before the
/// trust dialog" — a trust-boundary judgement. What this pane needs is "safe
/// to display" — a disclosure judgement. `ANTHROPIC_CUSTOM_HEADERS` is the
/// proof: CC lists it as pre-trust-safe, and its documented `Name: Value`
/// format happily holds `Authorization: Bearer …`. Collapsing the two axes
/// into one tier leaks it.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct Safety {
    /// The value may carry a credential → never serialize it over IPC.
    pub secret: bool,
    /// Not editable here at all.
    pub blocked_reason: Option<Blocked>,
    /// Membership in CC's `SAFE_ENV_VARS`.
    pub pretrust_safe: bool,
    /// Membership in CC's `PROVIDER_MANAGED_ENV_VARS`, or the
    /// `VERTEX_REGION_CLAUDE_` prefix. A host-managed launch may override the
    /// value regardless of what is set here.
    pub provider_managed: bool,
    /// Never empty for a variable outside `SAFE_ENV_VARS`; see [`Hazard`].
    pub hazards: Vec<Hazard>,
}

/// One documented environment variable.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct EnvVarSpec {
    pub name: String,
    pub category: String,
    /// The official prose, verbatim.
    pub doc: String,
    /// Found by scanning ONE Claude Code binary. Valid only on an exact
    /// version match — see [`CrosscheckValidity`].
    pub present_in_build: bool,
    pub safety: Safety,
    pub control: EnvControl,
    /// Closed value set for `Enum`, or the accepted literals for `Toggle`.
    #[serde(default)]
    pub values: Option<Vec<String>>,
    /// The documented default, empty when the prose states none. Shown as a
    /// *placeholder*, never written — writing it would pin today's number
    /// into settings and override whatever CC changes it to later.
    pub default: String,
    /// `ms`, `s`, `tokens`, `chars`, `%`, or empty.
    pub unit: String,
    /// Value that turns a toggle on, in the vocabulary the variable's own
    /// documentation uses — `1` for most, `true` for the eleven whose prose
    /// says so. Offering `1` where the doc says `true` would be guessing.
    pub on: Option<String>,
    /// The documented off value (`"0"` or `"false"`) for a three-state
    /// toggle, or `"unset"` for a two-state one where off means removing the
    /// key.
    pub off: Option<String>,
    /// Which rule proved the value is a number. Empty for non-numbers.
    /// Recorded so a control type is never an unexplained assertion.
    #[serde(default)]
    pub numeric_evidence: String,
    /// Display hint for `Text` (`url`, `path`, `model-id`, `secret`, …).
    /// **Never validation**: the MODEL-in-name rule labels
    /// `ANTHROPIC_DEFAULT_OPUS_MODEL_DESCRIPTION`, which is prose.
    pub format: String,
}

impl EnvVarSpec {
    /// A three-state toggle (Unset · off · on) rather than a two-state one.
    ///
    /// Keyed on "the off literal is a real value" rather than on `== "0"`:
    /// eleven variables use the `true`/`false` vocabulary, and comparing
    /// against `0` classified every one of them as two-state.
    pub fn is_tristate(&self) -> bool {
        self.control == EnvControl::Toggle && self.off.as_deref().is_some_and(|off| off != "unset")
    }

    /// Editable from this pane at all.
    pub fn is_editable(&self) -> bool {
        self.safety.blocked_reason.is_none()
    }
}

/// One category, in the order the pane groups by.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct EnvCategory {
    pub key: String,
    pub label: String,
}

/// The embedded artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnvSpec {
    pub schema_version: u32,
    pub docs_url: String,
    pub docs_fetched_at: String,
    pub docs_sha256: String,
    /// The one Claude Code binary the cross-check was run against.
    pub binary_crosscheck_version: String,
    /// When Claude Code's `managedEnvConstants.ts` was last read for the
    /// safety lists. A date, not a version: the source checkout and the
    /// installed binary can be different releases, and borrowing the
    /// binary's number for the source would be exactly the quiet lie the
    /// two provenance fields exist to prevent.
    pub cc_source_read_at: String,
    /// Which KIND of source the safety lists were read from —
    /// `pinned_mirror` today, `installed_binary` if a future extraction
    /// reads them from the running build. Emitted by the generator from
    /// the path it actually opened; see [`SafetyProvenance`] for why
    /// this is not a constant on this side.
    pub cc_source_kind: String,
    /// The version that source is stuck at, or `unknown`.
    pub cc_source_version: String,
    /// Category keys and labels, in the generator's order. Shipped rather
    /// than mirrored in the renderer, so adding or reordering a category is
    /// one edit instead of two that can disagree.
    pub categories: Vec<EnvCategory>,
    pub documented_count: usize,
    /// Names found in that binary and documented nowhere. Non-monotonic —
    /// render only on an exact version match.
    pub undocumented_in_build: Vec<String>,
    pub documented_not_in_build: Vec<String>,
    pub vars: Vec<EnvVarSpec>,
}

/// Whether the binary cross-check may be used at all for the running build.
///
/// Deliberately binary. A nearest-`≤` match is rejected outright: undocumented
/// names can be renamed or deleted in any release, so "nearest" is unsound
/// rather than approximate.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CrosscheckValidity {
    /// Installed version equals the snapshot's. `undocumented_in_build` and
    /// `present_in_build` are usable.
    Exact,
    /// Anything else, including an unresolvable version. Neither list may be
    /// used: no name is rendered, and no documented row is hidden or tagged
    /// "not in build" on the strength of a snapshot that does not describe
    /// the running build.
    Mismatch {
        snapshot_version: String,
        installed_version: Option<String>,
    },
}

impl CrosscheckValidity {
    pub fn is_exact(&self) -> bool {
        matches!(self, Self::Exact)
    }
}

/// Where the *safety* classification came from, and why that is a
/// weaker claim than the binary cross-check beside it.
///
/// `binary_crosscheck_version` is compared against the running build and
/// self-disables on a mismatch ([`CrosscheckValidity`]). The safety
/// lists — `SAFE_ENV_VARS`, `PROVIDER_MANAGED_ENV_VARS`, and therefore
/// every row's `pre_trust_safe` / `provider_managed` flag — have no such
/// gate: `scripts/build-cc-env-spec.py` reads them from
/// `~/github/claude_code_src/src/utils/managedEnvConstants.ts`, a
/// third-party source mirror **pinned at 2.1.88 and abandoned upstream
/// on 2026-04-15**. `.claude/rules/cc-upstream-watch.md` forbids that
/// mirror as a verification source; the generator predates the rule.
///
/// This type exists so the weakness is *legible* rather than silent. It
/// cannot be resolved by comparing versions — the mirror has no version
/// to compare, it is simply old — so the honest surface is the date it
/// was read and the fact that it is a mirror.
///
/// Why it matters rather than being pedantry: the flags are not
/// cosmetic. `ANTHROPIC_CUSTOM_HEADERS` is pre-trust-safe *and* able to
/// carry `Authorization: Bearer …`; "safe to apply from an untrusted
/// source" and "safe to display" are different axes, and a row carrying
/// a stale answer to the first can leak on the second.
///
/// The lists are minified out of the shipped binary, so closing this
/// needs a new extraction strategy, not a re-run of the generator.
///
/// **Every field is read from the generated artifact, never hardcoded
/// here.** A constant on this side would keep announcing "pinned mirror,
/// 2.1.88" as fact after the generator moved to a different source —
/// which is the same status-surface-asserts-an-unverified-claim failure
/// this disclosure exists to prevent, reintroduced by the fix for it.
/// The generator derives both from the path it actually opened and the
/// tarball it found beside it.
#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub struct SafetyProvenance {
    /// `cc_source_read_at` — the mtime of the file the lists were read
    /// from, not the date of the release they describe.
    pub read_at: String,
    /// True while the lists come from the pinned source mirror. Derived
    /// from the artifact's `cc_source_kind`, so a future extraction from
    /// the installed binary flips it without anyone editing Rust.
    pub from_pinned_mirror: bool,
    /// The version that source is stuck at, or `unknown` when the
    /// generator could not determine it. Rendered as-is — an honest
    /// blank beats a stale constant.
    pub mirror_version: String,
}

impl EnvSpec {
    /// Provenance of the safety flags. See [`SafetyProvenance`] — this
    /// is deliberately not a validity *gate*: unlike the binary
    /// cross-check there is nothing to compare against, and hiding the
    /// flags would leave the pane unable to warn about anything at all.
    /// The caller's job is to say where they came from, not to suppress
    /// them.
    pub fn safety_provenance(&self) -> SafetyProvenance {
        SafetyProvenance {
            read_at: self.cc_source_read_at.clone(),
            from_pinned_mirror: self.cc_source_kind == "pinned_mirror",
            mirror_version: self.cc_source_version.clone(),
        }
    }

    /// Compare the snapshot's binary version with the installed one.
    pub fn crosscheck_validity(&self, installed_version: Option<&str>) -> CrosscheckValidity {
        match installed_version {
            Some(v) if v == self.binary_crosscheck_version => CrosscheckValidity::Exact,
            other => CrosscheckValidity::Mismatch {
                snapshot_version: self.binary_crosscheck_version.clone(),
                installed_version: other.map(str::to_string),
            },
        }
    }
}

static SPEC: OnceLock<Result<EnvSpec, String>> = OnceLock::new();
static INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();

/// The embedded spec, or why it could not be used.
///
/// `rules/rust-conventions.md` forbids `unwrap`/`expect` in core, and this is
/// exactly why: a malformed artifact would take the whole app down at the
/// first pane render rather than leaving every other surface working while
/// this one reports a broken build.
///
/// Both failure modes are build defects the gates already catch —
/// `--check` regenerates byte-for-byte and `embedded_spec_parses_and_is_indexed`
/// runs on every `cargo test` — so this path should never execute. It exists
/// so that if it ever does, it degrades instead of crashing.
pub fn try_spec() -> Result<&'static EnvSpec, CcEnvError> {
    match SPEC.get_or_init(load_spec) {
        Ok(s) => Ok(s),
        Err(why) => Err(CcEnvError::MalformedSpec(why.clone())),
    }
}

fn load_spec() -> Result<EnvSpec, String> {
    let parsed: EnvSpec =
        serde_json::from_str(SPEC_JSON).map_err(|e| format!("embedded cc-env-spec.json: {e}"))?;
    // Names index a HashMap, so a duplicate would silently shadow its twin:
    // one row would render while `lookup` — and therefore every secret and
    // editability decision — resolved to the other.
    let mut seen = std::collections::HashSet::with_capacity(parsed.vars.len());
    for v in &parsed.vars {
        if !seen.insert(v.name.as_str()) {
            return Err(format!("embedded cc-env-spec.json lists {} twice", v.name));
        }
    }
    if parsed.documented_count != parsed.vars.len() {
        return Err(format!(
            "embedded cc-env-spec.json says {} documented but carries {}",
            parsed.documented_count,
            parsed.vars.len()
        ));
    }
    Ok(parsed)
}

/// Infallible accessor for the callers that cannot do anything useful with a
/// broken build (tests, and the invariants below). Every runtime path that
/// crosses to the user goes through [`try_spec`].
fn spec_or_empty() -> &'static EnvSpec {
    static EMPTY: OnceLock<EnvSpec> = OnceLock::new();
    match SPEC.get_or_init(load_spec) {
        Ok(s) => s,
        Err(_) => EMPTY.get_or_init(|| EnvSpec {
            schema_version: 0,
            docs_url: String::new(),
            docs_fetched_at: String::new(),
            docs_sha256: String::new(),
            binary_crosscheck_version: String::new(),
            cc_source_read_at: String::new(),
            cc_source_kind: String::new(),
            cc_source_version: String::new(),
            categories: Vec::new(),
            documented_count: 0,
            undocumented_in_build: Vec::new(),
            documented_not_in_build: Vec::new(),
            vars: Vec::new(),
        }),
    }
}

/// The embedded spec. Empty when the artifact is unusable, so a broken build
/// renders an empty pane rather than panicking. Prefer [`try_spec`] where the
/// caller can report the failure.
pub fn spec() -> &'static EnvSpec {
    spec_or_empty()
}

fn index() -> &'static HashMap<&'static str, usize> {
    INDEX.get_or_init(|| {
        spec()
            .vars
            .iter()
            .enumerate()
            .map(|(i, v)| (v.name.as_str(), i))
            .collect()
    })
}

/// Exact-name lookup. Environment variable names are case-sensitive on every
/// platform Claude Code writes settings for, and a case-insensitive match here
/// would let `anthropic_api_key` inherit `ANTHROPIC_API_KEY`'s secret flag
/// while writing a different key.
pub fn lookup(name: &str) -> Option<&'static EnvVarSpec> {
    index().get(name).map(|i| &spec().vars[*i])
}

/// Whether a name is documented at all.
pub fn is_documented(name: &str) -> bool {
    index().contains_key(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const VECTORS: &str = include_str!("../../testdata/cc-env-vectors.json");

    #[test]
    fn embedded_spec_parses_and_is_indexed() {
        let s = spec();
        assert_eq!(s.schema_version, 1);
        assert_eq!(s.documented_count, s.vars.len());
        assert!(s.vars.len() > 300, "got {} vars", s.vars.len());
        assert_eq!(
            lookup("ANTHROPIC_API_KEY").unwrap().name,
            "ANTHROPIC_API_KEY"
        );
        assert!(lookup("NOT_A_REAL_VARIABLE").is_none());
    }

    /// The same hand-authored vectors `scripts/build-cc-env-spec.py` runs.
    /// Both the Python producer and the Rust consumer assert against them, so
    /// the two cannot drift apart silently.
    #[test]
    fn golden_vectors_match_the_embedded_spec() {
        let doc: Value = serde_json::from_str(VECTORS).unwrap();
        let vectors = doc["vectors"].as_array().unwrap();
        assert!(vectors.len() >= 35, "vector set shrank: {}", vectors.len());

        let raw: Value = serde_json::from_str(SPEC_JSON).unwrap();
        let by_name: HashMap<&str, &Value> = raw["vars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (v["name"].as_str().unwrap(), v))
            .collect();

        for vec in vectors {
            let name = vec["name"].as_str().unwrap();
            let actual = by_name
                .get(name)
                .unwrap_or_else(|| panic!("{name}: no such documented variable"));
            for (key, want) in vec.as_object().unwrap() {
                if key == "name" || key == "why" {
                    continue;
                }
                if key == "safety" {
                    for (skey, swant) in want.as_object().unwrap() {
                        assert_eq!(
                            &actual["safety"][skey], swant,
                            "{name}.safety.{skey} drifted"
                        );
                    }
                    continue;
                }
                assert_eq!(&actual[key], want, "{name}.{key} drifted");
            }
        }
    }

    #[test]
    fn every_number_carries_its_evidence_and_no_toggle_does() {
        for v in &spec().vars {
            match v.control {
                EnvControl::Number => assert!(
                    !v.numeric_evidence.is_empty(),
                    "{}: number without evidence",
                    v.name
                ),
                _ => assert!(
                    v.numeric_evidence.is_empty(),
                    "{}: non-number carries numeric evidence",
                    v.name
                ),
            }
        }
    }

    #[test]
    fn absence_from_cc_safe_list_always_yields_a_hazard() {
        for v in &spec().vars {
            if !v.safety.pretrust_safe {
                assert!(
                    !v.safety.hazards.is_empty(),
                    "{}: outside SAFE_ENV_VARS with no hazard at all",
                    v.name
                );
            }
        }
    }

    #[test]
    fn the_two_known_overlaps_are_both_axes_at_once() {
        for name in ["ANTHROPIC_CUSTOM_HEADERS", "ANTHROPIC_FOUNDRY_API_KEY"] {
            let v = lookup(name).unwrap();
            assert!(v.safety.secret, "{name} must be secret");
            assert!(v.safety.pretrust_safe, "{name} must be pre-trust-safe");
        }
    }

    #[test]
    fn toggle_shapes_are_well_formed() {
        for v in &spec().vars {
            if v.control != EnvControl::Toggle {
                assert!(v.on.is_none() && v.off.is_none(), "{}", v.name);
                continue;
            }
            let on = v.on.as_deref().unwrap();
            let off = v.off.as_deref().unwrap();
            // Exactly one vocabulary per variable, never a mix — the
            // generator asserts the same thing over the prose.
            let expected_off = match on {
                "1" => "0",
                "true" => "false",
                other => panic!("{}: unknown on-literal {other}", v.name),
            };
            assert!(
                off == expected_off || off == "unset",
                "{}: on={on} off={off}",
                v.name
            );
            let vals = v.values.as_ref().unwrap();
            if off == "unset" {
                assert_eq!(vals, &[on.to_string()], "{}", v.name);
                assert!(!v.is_tristate(), "{}", v.name);
            } else {
                assert_eq!(vals, &[off.to_string(), on.to_string()], "{}", v.name);
                assert!(v.is_tristate(), "{}", v.name);
            }
        }
    }

    #[test]
    fn blocked_variables_are_not_editable() {
        let cfg = lookup("CLAUDE_CONFIG_DIR").unwrap();
        assert_eq!(
            cfg.safety.blocked_reason,
            Some(Blocked::BootstrapSplitBrain)
        );
        assert!(!cfg.is_editable());
        let sid = lookup("CLAUDE_CODE_SESSION_ID").unwrap();
        assert_eq!(sid.safety.blocked_reason, Some(Blocked::HostInjected));
        assert!(!sid.is_editable());

        // #88. CC 2.1.234 added this one, and its docs say CC "reads it
        // only from the environment you start `claude` from, never from a
        // settings file `env` block" — which is the only block this pane
        // writes. Editable, it would save the key and change nothing.
        let pdn = lookup("CLAUDE_CODE_PROJECT_DIR_NAME").unwrap();
        assert_eq!(
            pdn.safety.blocked_reason,
            Some(Blocked::EnvOnlyNotSettings),
            "a variable CC ignores from settings must not render as a field"
        );
        assert!(!pdn.is_editable());

        // The other var 2.1.234 added is an ordinary editable number.
        let goal = lookup("CLAUDE_CODE_GOAL_CHECKIN_MINUTES").unwrap();
        assert_eq!(goal.safety.blocked_reason, None);
        assert!(goal.is_editable());
        assert_eq!(goal.default, "30", "documented default");

        // And the credential 2.1.241's docs added is both host-injected
        // and secret — the name-shaped-like-a-credential auditor in
        // `build-cc-env-spec.py` refuses to build until it is adjudicated.
        let tok = lookup("CLAUDE_CODE_MESSAGING_TOKEN").unwrap();
        assert!(tok.safety.secret, "it is the peer-messaging peerToken");
        assert_eq!(tok.safety.blocked_reason, Some(Blocked::HostInjected));
        assert!(lookup("MAX_THINKING_TOKENS").unwrap().is_editable());
    }

    #[test]
    fn crosscheck_is_exact_match_only() {
        let s = spec();
        assert!(s
            .crosscheck_validity(Some(&s.binary_crosscheck_version))
            .is_exact());
        // A newer build must NOT fall back to the nearest snapshot.
        assert!(!s.crosscheck_validity(Some("99.0.0")).is_exact());
        assert!(!s.crosscheck_validity(None).is_exact());
    }

    #[test]
    fn provenance_carries_both_facts_separately() {
        let s = spec();
        assert!(!s.docs_fetched_at.is_empty());
        assert_eq!(s.docs_sha256.len(), 64);
        assert!(!s.binary_crosscheck_version.is_empty());
    }

    /// The provenance the disclosure renders must come from the shipped
    /// artifact, not from a literal in this file. The first version of
    /// `SafetyProvenance` hardcoded `from_pinned_mirror: true` and the
    /// version string, which would have gone on asserting "pinned
    /// mirror, 2.1.88" as fact after the generator changed source — the
    /// failure the disclosure exists to prevent.
    #[test]
    fn safety_provenance_is_read_from_the_artifact() {
        let s = spec();
        let p = s.safety_provenance();
        assert_eq!(p.read_at, s.cc_source_read_at);
        assert_eq!(p.mirror_version, s.cc_source_version);
        assert_eq!(p.from_pinned_mirror, s.cc_source_kind == "pinned_mirror");
        // and the artifact actually carries them
        assert!(
            !s.cc_source_kind.is_empty(),
            "generator must emit cc_source_kind"
        );
        assert!(
            !s.cc_source_version.is_empty(),
            "generator must emit cc_source_version (`unknown` is allowed, empty is not)"
        );
    }

    /// A different source kind must flip the flag with no Rust edit —
    /// the whole point of moving it into the artifact.
    #[test]
    fn a_non_mirror_source_kind_clears_the_pinned_mirror_flag() {
        let mut s = spec().clone();
        s.cc_source_kind = "installed_binary".to_string();
        assert!(!s.safety_provenance().from_pinned_mirror);
    }
}

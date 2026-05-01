//! Template registry — bundled blueprints loaded into memory.
//!
//! Per `dev-docs/templates-implementation-plan.md` §5, blueprints
//! are embedded into the binary via `include_str!` at build time.
//! At runtime, the registry exposes them by id and validates the
//! whole bundle (no duplicates, every blueprint parses, every
//! sample report path resolves).
//!
//! New blueprints are added by:
//!
//! 1. Authoring `blueprints/<id>.toml` with a fresh id (validated)
//! 2. Authoring `blueprints/samples/<sample-name>.md`
//! 3. Adding both to the [`BUNDLED`] table below
//!
//! The build-time test `bundled_blueprints_load_cleanly` parses
//! every entry and verifies sample reports resolve. Drift between
//! the table and the filesystem fails CI.

use std::collections::BTreeMap;

use super::blueprint::Blueprint;
use super::error::TemplateError;

/// One bundled blueprint entry: the id (matches the blueprint's
/// `id` field), the raw TOML source, and an optional sample-report
/// markdown source. Sample is optional at runtime — the install
/// dialog renders "no sample yet" when missing — but every shipped
/// blueprint must have one (enforced by tests).
struct BundledEntry {
    id: &'static str,
    toml: &'static str,
    sample_md: Option<&'static str>,
}

/// All blueprints shipped in the binary. Order doesn't matter
/// (the registry sorts by id). New entries go at the bottom and
/// are paired with a sample.
const BUNDLED: &[BundledEntry] = &[
    BundledEntry {
        id: "it.morning-health-check",
        toml: include_str!("blueprints/it.morning-health-check.toml"),
        sample_md: Some(include_str!("blueprints/samples/morning-health-check.md")),
    },
];

/// Registry of all bundled blueprints. Built once at startup;
/// cheap to clone (the blueprints are themselves Clone).
#[derive(Debug, Clone, Default)]
pub struct TemplateRegistry {
    by_id: BTreeMap<String, Blueprint>,
    samples: BTreeMap<String, &'static str>,
}

impl TemplateRegistry {
    /// Parse and validate every bundled blueprint. Returns an
    /// error if any entry is malformed, has a duplicate id, or
    /// declares a sample that isn't bundled.
    pub fn load_bundled() -> Result<Self, TemplateError> {
        let mut by_id = BTreeMap::new();
        let mut samples = BTreeMap::new();

        for entry in BUNDLED {
            let bp = Blueprint::from_toml(entry.toml)?;

            // The bundle table's `id` and the blueprint's own id
            // must match — otherwise `include_str!` paths can drift
            // from the blueprint contents undetected.
            if bp.id().0 != entry.id {
                return Err(TemplateError::malformed(
                    entry.id,
                    format!(
                        "bundle table id {:?} does not match blueprint id {:?}",
                        entry.id,
                        bp.id().0
                    ),
                ));
            }

            // Sample-report claim must match what's bundled.
            match (&bp.sample_report, entry.sample_md) {
                (Some(_), Some(md)) => {
                    samples.insert(bp.id().0.clone(), md);
                }
                (Some(path), None) => {
                    return Err(TemplateError::MissingSample {
                        id: bp.id().0.clone(),
                        path: path.clone(),
                    });
                }
                (None, Some(_)) | (None, None) => {
                    // Blueprint doesn't claim a sample; nothing to
                    // load. Bundling a sample without a claim is
                    // harmless but currently silent. If that
                    // becomes a problem, fail loudly here.
                }
            }

            if by_id.insert(bp.id().0.clone(), bp).is_some() {
                return Err(TemplateError::DuplicateId {
                    id: entry.id.to_string(),
                });
            }
        }

        Ok(Self { by_id, samples })
    }

    pub fn get(&self, id: &str) -> Option<&Blueprint> {
        self.by_id.get(id)
    }

    pub fn list(&self) -> impl Iterator<Item = &Blueprint> {
        self.by_id.values()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// The bundled markdown for a blueprint's sample report, if
    /// the blueprint declared one and the bundle includes it.
    pub fn sample_report(&self, id: &str) -> Option<&'static str> {
        self.samples.get(id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::super::blueprint::{Category, Tier};
    use super::*;

    #[test]
    fn bundled_blueprints_load_cleanly() {
        let registry = TemplateRegistry::load_bundled()
            .expect("every bundled blueprint must parse and validate");
        assert!(
            !registry.is_empty(),
            "registry should ship at least one blueprint"
        );
    }

    #[test]
    fn first_blueprint_is_morning_health_check() {
        let registry = TemplateRegistry::load_bundled().unwrap();
        let bp = registry
            .get("it.morning-health-check")
            .expect("morning health check should ship in the bundle");
        assert_eq!(bp.category, Category::ItHealth);
        assert_eq!(bp.tier, Tier::Ambient);
    }

    #[test]
    fn sample_report_is_loaded_for_morning_check() {
        let registry = TemplateRegistry::load_bundled().unwrap();
        let sample = registry
            .sample_report("it.morning-health-check")
            .expect("morning health check ships a sample report");
        assert!(
            sample.contains("Morning health check"),
            "sample report should be the markdown body"
        );
    }

    #[test]
    fn unknown_id_returns_none() {
        let registry = TemplateRegistry::load_bundled().unwrap();
        assert!(registry.get("never.exists").is_none());
        assert!(registry.sample_report("never.exists").is_none());
    }

    #[test]
    fn list_is_sorted_by_id() {
        let registry = TemplateRegistry::load_bundled().unwrap();
        let ids: Vec<_> = registry.list().map(|bp| bp.id().0.clone()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }
}

//! Conflict resolution — project-level skip / merge / replace.
//!
//! See `dev-docs/project-migrate-spec.md` §6.
//!
//! The conflict matrix per project:
//!
//! | Mode | Absent | Present, no overlap | Present, overlap |
//! |---|---|---|---|
//! | skip (default) | apply | refuse | refuse |
//! | merge | apply | union | uniform prefer-* policy |
//! | replace | apply | archive + apply | archive + apply |
//!
//! `replace` requires `--yes` on the CLI (handled at the adapter
//! layer, not here).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ConflictMode {
    #[default]
    Skip,
    Merge,
    Replace,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergePreference {
    /// Imported wins on session collisions.
    Imported,
    /// Target wins on session collisions.
    Target,
}

/// Per-project conflict shape, computed from a comparison between the
/// bundle's project manifest and the target's on-disk state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProjectConflict {
    /// Target has no slug for this project; safe to apply.
    None,
    /// Target slug exists, no overlapping sessionIds.
    PresentNoOverlap {
        target_slug: String,
        target_session_count: usize,
    },
    /// Target slug exists AND has overlapping sessionIds.
    PresentOverlap {
        target_slug: String,
        overlapping_ids: Vec<String>,
    },
}

/// Outcome of applying the conflict matrix to a single project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Apply,
    /// Apply, but archive target's matching slug to claudepot trash
    /// before extracting.
    ArchiveThenApply,
    /// Apply per merge: union sessions, dedupe, prefer per the
    /// merge-preference flag.
    Merge {
        prefer: MergePreference,
    },
    /// Refuse — surface to the user with the carried reason.
    Refuse(String),
}

/// Apply the conflict matrix. Returns the chosen resolution.
pub fn resolve(
    conflict: &ProjectConflict,
    mode: ConflictMode,
    prefer: Option<MergePreference>,
) -> Resolution {
    match (conflict, mode) {
        (ProjectConflict::None, _) => Resolution::Apply,
        (ProjectConflict::PresentNoOverlap { target_slug, .. }, ConflictMode::Skip) => {
            Resolution::Refuse(format!(
                "target already has project slug {target_slug}; \
                 use --mode=merge or --mode=replace"
            ))
        }
        (ProjectConflict::PresentNoOverlap { .. }, ConflictMode::Merge) => Resolution::Merge {
            prefer: prefer.unwrap_or(MergePreference::Imported),
        },
        (ProjectConflict::PresentNoOverlap { .. }, ConflictMode::Replace) => {
            Resolution::ArchiveThenApply
        }
        (
            ProjectConflict::PresentOverlap {
                target_slug,
                overlapping_ids,
            },
            ConflictMode::Skip,
        ) => Resolution::Refuse(format!(
            "target slug {target_slug} has {} overlapping sessionId(s); \
             use --mode=merge --prefer-imported|--prefer-target or --mode=replace",
            overlapping_ids.len()
        )),
        (
            ProjectConflict::PresentOverlap {
                overlapping_ids, ..
            },
            ConflictMode::Merge,
        ) => match prefer {
            None => Resolution::Refuse(format!(
                "{} overlapping sessionId(s); --mode=merge requires \
                     --prefer-imported or --prefer-target",
                overlapping_ids.len()
            )),
            Some(p) => Resolution::Merge { prefer: p },
        },
        (ProjectConflict::PresentOverlap { .. }, ConflictMode::Replace) => {
            Resolution::ArchiveThenApply
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_conflict_always_applies() {
        let r = resolve(&ProjectConflict::None, ConflictMode::Skip, None);
        assert_eq!(r, Resolution::Apply);
    }

    #[test]
    fn skip_mode_refuses_present() {
        let c = ProjectConflict::PresentNoOverlap {
            target_slug: "-x".to_string(),
            target_session_count: 0,
        };
        let r = resolve(&c, ConflictMode::Skip, None);
        match r {
            Resolution::Refuse(_) => {}
            _ => panic!("expected Refuse"),
        }
    }

    #[test]
    fn merge_overlap_requires_preference() {
        let c = ProjectConflict::PresentOverlap {
            target_slug: "-x".to_string(),
            overlapping_ids: vec!["abc".to_string()],
        };
        let r = resolve(&c, ConflictMode::Merge, None);
        match r {
            Resolution::Refuse(_) => {}
            _ => panic!("expected Refuse without preference"),
        }
        let r = resolve(&c, ConflictMode::Merge, Some(MergePreference::Imported));
        assert_eq!(
            r,
            Resolution::Merge {
                prefer: MergePreference::Imported
            }
        );
    }

    #[test]
    fn merge_no_overlap_defaults_to_imported() {
        let c = ProjectConflict::PresentNoOverlap {
            target_slug: "-x".to_string(),
            target_session_count: 5,
        };
        let r = resolve(&c, ConflictMode::Merge, None);
        assert_eq!(
            r,
            Resolution::Merge {
                prefer: MergePreference::Imported
            }
        );
    }

    #[test]
    fn replace_archives_then_applies() {
        let c = ProjectConflict::PresentNoOverlap {
            target_slug: "-x".to_string(),
            target_session_count: 0,
        };
        assert_eq!(
            resolve(&c, ConflictMode::Replace, None),
            Resolution::ArchiveThenApply
        );
    }
}

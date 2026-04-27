//! §11.2 FS goldens — filesystem-shape edge cases.
//!
//! These exercise the conflict-detection + apply paths against
//! synthesized on-disk state. Runs on every host OS; case-insensitive
//! collisions are simulated logically (we don't depend on HFS+/APFS
//! at test time).

use crate::migrate::conflicts::{
    resolve, ConflictMode, MergePreference, ProjectConflict, Resolution,
};

// Row 11 — case-insensitive collision on target (HFS+ has
// `-Users-Alice-X`, import brings `-Users-alice-x`). Refuse with
// explicit error; suggest `--mode=replace`.
#[test]
fn row11_case_insensitive_collision_refused_with_replace_hint() {
    // Logical case: target has `-Users-Alice-X` (capital A/X) and the
    // bundle would land `-Users-alice-x`. On HFS+ those collide; on
    // ext4 they don't. We simulate the detection by treating the
    // existing slug as a `PresentNoOverlap` conflict.
    let c = ProjectConflict::PresentNoOverlap {
        target_slug: "-Users-Alice-X".to_string(),
        target_session_count: 0,
    };
    match resolve(&c, ConflictMode::Skip, None) {
        Resolution::Refuse(reason) => {
            assert!(
                reason.contains("--mode=replace") || reason.contains("--mode=merge"),
                "expected hint about --mode flag; got: {reason}"
            );
        }
        other => panic!("expected Refuse, got {other:?}"),
    }
}

// Row 12 — two projects in one bundle whose slugs collide on target
// after rewrite. Refuse with both source paths listed.
#[test]
fn row12_two_projects_collide_on_target() {
    // Both source projects, after path rewrite, end at the same target
    // slug. Detected as `PresentNoOverlap` on the second project once
    // the first lands.
    let c = ProjectConflict::PresentNoOverlap {
        target_slug: "-shared-target".to_string(),
        target_session_count: 0,
    };
    let r = resolve(&c, ConflictMode::Skip, None);
    assert!(matches!(r, Resolution::Refuse(_)));
}

// Row 13 — target has same project at a different cwd (rename
// happened on target without claudepot). Refuse with both cwds.
#[test]
fn row13_target_renamed_outside_claudepot_refuses() {
    // Same conflict shape as row 12 — the rename-outside-claudepot
    // case manifests as the target slug existing under a cwd different
    // from what the bundle expects. Resolution policy is identical:
    // skip-mode refuses.
    let c = ProjectConflict::PresentNoOverlap {
        target_slug: "-Users-different-x".to_string(),
        target_session_count: 5,
    };
    let r = resolve(&c, ConflictMode::Skip, None);
    assert!(matches!(r, Resolution::Refuse(_)));
}

// Row 14 — encrypted home (eCryptfs / Windows EFS) where path-length
// budget is shorter than source. Pre-flight refuses with affected
// files.
//
// We can't simulate the actual filesystem-level NAME_MAX cap, but we
// can pin the contract: long-path slugs that exceed our 200-byte
// sanitized cap get a djb2 suffix; CC's `findProjectDir` prefix-scan
// handles them. The pre-flight failure described in the spec is FS
// level, not slug level — locked here as a spec reminder.
#[test]
fn row14_long_path_emits_dual_hashable_slug() {
    let long = "/Users/joker/".to_string() + &"a".repeat(200);
    let slug = crate::migrate::plan::target_slug(&long);
    assert!(slug.len() > 200);
    // Suffix shape: `-` + base36 djb2 hash.
    let after_200 = &slug[200..];
    assert!(after_200.starts_with('-'));
    let suffix = &after_200[1..];
    assert!(!suffix.is_empty());
    assert!(suffix.chars().all(|c| c.is_ascii_alphanumeric()));
}

// Row 15 — disk-full mid-apply: staging on same volume; pre-flight
// `statvfs` × 1.5; never start apply.
//
// The disk-full pre-flight check itself is FS-level. We pin the
// invariant by asserting the staging directory contract: the staging
// path always lives under `claudepot_data_dir()`, and per spec §8.1
// the apply rename target is under `claude_config_dir()`. When these
// are on the same volume (the default) the rename is atomic; the
// disk-full pre-flight is a separate function (deferred). Locked
// here as a contract reminder.
#[test]
fn row15_staging_dir_lives_under_claudepot_data_dir() {
    let id = "abc";
    let staging = crate::migrate::apply::staging_dir(id);
    let data_dir = crate::paths::claudepot_data_dir();
    assert!(
        staging.starts_with(&data_dir),
        "staging must live under claudepot_data_dir for atomic rename: {} not under {}",
        staging.display(),
        data_dir.display()
    );
}

// Bonus: merge preference round-trip — locks the prefer-imported /
// prefer-target wire shape.
#[test]
fn merge_overlap_with_prefer_target() {
    let c = ProjectConflict::PresentOverlap {
        target_slug: "-Users-x".to_string(),
        overlapping_ids: vec!["abc".to_string()],
    };
    let r = resolve(&c, ConflictMode::Merge, Some(MergePreference::Target));
    match r {
        Resolution::Merge { prefer } => assert_eq!(prefer, MergePreference::Target),
        other => panic!("expected Merge, got {other:?}"),
    }
}

//! Integration / E2E suite for the templates feature.
//!
//! Designed to be deployed to and run on a real macOS host
//! (mac-mini-home — see `dev-docs/templates-e2e-plan.md`). Each
//! test owns its `tempfile::TempDir` and never touches the
//! caller's `~/.claudepot/`. The suite exercises:
//!
//! - Bundled registry boot + every blueprint's instantiate cycle.
//! - schedule_to_cron coverage for all six shape variants.
//! - Apply pipeline against real disk: validator + executor on a
//!   tempdir scope, plus traversal / symlink-escape rejection.
//! - Routing rules round-trip on disk + first-match-wins evaluator.
//! - Caregiver consent file mode 0600 verified via stat.
//!
//! Why integration: unit tests run from the source tree. These
//! tests confirm the binary that actually ships works on a
//! freshly-deployed machine — APFS atomic writes, real /tmp
//! symlink resolution (macOS), and arch-specific serde behavior.
//!
//! Run with:
//!   cargo test -p claudepot-core --test templates_e2e -- --nocapture
//!
//! All scenarios run on every invocation; there is no `#[ignore]`
//! gate because no test reaches outside its own tempdir.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use claudepot_core::automations::types::HostPlatform;
use claudepot_core::routes::Route;
use claudepot_core::templates::apply::{
    apply_selected, validate_item, ItemOutcome, Operation, PendingChanges, PendingGroup,
    PendingItem,
};
use claudepot_core::templates::caregiver::consent::{ConsentRecord, ConsentStore, SmtpProvider};
use claudepot_core::templates::routing::{
    at_path as routing_at_path, evaluate, Match, RoutingRule, RoutingRules, RoutingStore,
    Suggestion, UseRoute,
};
use claudepot_core::templates::{
    instantiate, schedule_to_cron, ScheduleDto, TemplateInstance, TemplateRegistry, Weekday,
};

// ===================================================================
// Section 1 — Registry boots cleanly with every bundled blueprint.
// ===================================================================

#[test]
fn registry_loads_every_bundled_blueprint() {
    let registry = TemplateRegistry::load_bundled().expect("registry must boot cleanly");
    let count = registry.len();
    assert!(
        count >= 22,
        "expected at least 22 bundled blueprints, got {count}"
    );
    // Every blueprint listed must have a non-empty id and name.
    for bp in registry.list() {
        assert!(!bp.id().0.is_empty(), "blueprint id is empty");
        assert!(!bp.name.is_empty(), "blueprint {} has no name", bp.id().0);
        assert!(
            !bp.tagline.is_empty(),
            "blueprint {} has no tagline",
            bp.id().0
        );
    }
}

#[test]
fn registry_list_is_sorted_by_id() {
    let registry = TemplateRegistry::load_bundled().unwrap();
    let ids: Vec<String> = registry.list().map(|bp| bp.id().0.clone()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "registry ordering must be stable + sorted");
}

#[test]
fn registry_includes_known_canonical_ids() {
    // Pin the existence of one blueprint per category so the
    // bundle table can't accidentally drop a category in a
    // refactor.
    let registry = TemplateRegistry::load_bundled().unwrap();
    let must_have = [
        "it.morning-health-check", // it-health
        "diag.disk-full",          // diagnostics
        "audit.cache-cleanup",     // audit
        "house.downloads-tidy",    // housekeeping
        "network.lan-census",      // network
        "caregiver.heartbeat",     // caregiver
    ];
    for id in must_have {
        assert!(
            registry.get(id).is_some(),
            "missing canonical blueprint {id}"
        );
    }
}

#[test]
fn every_bundled_blueprint_currently_supports_only_macos() {
    // Pin the cross-platform audit decision: every shipped
    // blueprint hardcodes macOS shell tools (`defaults`, `pmset`,
    // `system_profiler`, `~/Library/...`). Until per-platform
    // variants are authored, the registry filters them out on
    // Linux/Windows. This test fails the moment a blueprint
    // declares broader support without a Linux/Windows
    // implementation — forcing the author to think about it.
    let registry = TemplateRegistry::load_bundled().unwrap();
    for bp in registry.list() {
        assert_eq!(
            bp.supported_platforms,
            vec![HostPlatform::Macos],
            "blueprint {} must declare only macOS support until a non-macOS variant ships",
            bp.id().0
        );
    }
}

#[test]
fn registry_list_for_filters_by_host_platform() {
    let registry = TemplateRegistry::load_bundled().unwrap();
    let mac_count = registry.list_for(HostPlatform::Macos).count();
    let linux_count = registry.list_for(HostPlatform::Linux).count();
    let windows_count = registry.list_for(HostPlatform::Windows).count();
    assert!(
        mac_count >= 22,
        "macOS must see every shipped blueprint, got {mac_count}"
    );
    assert_eq!(
        linux_count, 0,
        "Linux must currently see zero blueprints (none declare Linux support yet)"
    );
    assert_eq!(
        windows_count, 0,
        "Windows must currently see zero blueprints (none declare Windows support yet)"
    );
}

#[test]
fn every_bundled_blueprint_has_a_sample_or_explicit_none() {
    let registry = TemplateRegistry::load_bundled().unwrap();
    for bp in registry.list() {
        if bp.sample_report.is_some() {
            let body = registry.sample_report(&bp.id().0).unwrap_or_else(|| {
                panic!(
                    "blueprint {} declares a sample but registry has none",
                    bp.id().0
                )
            });
            assert!(
                !body.trim().is_empty(),
                "blueprint {}: sample body is empty",
                bp.id().0
            );
        }
    }
}

// ===================================================================
// Section 2 — Instantiate each blueprint with its default schedule.
// ===================================================================

#[test]
fn every_bundled_blueprint_instantiates_with_default_schedule() {
    let registry = TemplateRegistry::load_bundled().unwrap();
    for bp in registry.list() {
        // Pick a schedule the blueprint actually allows. Prefer
        // manual when available (it sidesteps cron syntax for
        // this round-trip); otherwise fall back to the first
        // allowed shape with a defaulted time.
        let shape = bp
            .schedule
            .allowed_shapes
            .iter()
            .copied()
            .next()
            .expect("every blueprint must allow at least one shape");
        let schedule = synth_schedule(shape);

        let instance = TemplateInstance {
            blueprint_id: bp.id().0.clone(),
            blueprint_schema_version: bp.schema_version,
            placeholder_values: BTreeMap::new(),
            route_id: None,
            schedule,
            name_override: None,
        };
        let resolved = instantiate(bp, &instance)
            .unwrap_or_else(|e| panic!("instantiate failed for {}: {e:?}", bp.id().0));
        assert_eq!(resolved.template_id, bp.id().0);
        assert!(!resolved.name.is_empty());
        assert!(!resolved.prompt.is_empty());
    }
}

fn synth_schedule(shape: claudepot_core::templates::ScheduleShape) -> ScheduleDto {
    use claudepot_core::templates::ScheduleShape::*;
    match shape {
        Daily => ScheduleDto::Daily {
            time: "08:00".into(),
        },
        Weekdays => ScheduleDto::Weekdays {
            time: "08:00".into(),
        },
        Weekly => ScheduleDto::Weekly {
            day: Weekday::Mon,
            time: "08:00".into(),
        },
        Hourly => ScheduleDto::Hourly { every_n_hours: 4 },
        Manual => ScheduleDto::Manual,
        Custom => ScheduleDto::Custom {
            cron: "0 8 * * *".into(),
        },
    }
}

// ===================================================================
// Section 3 — schedule_to_cron coverage.
// ===================================================================

#[test]
fn schedule_to_cron_daily() {
    let r = schedule_to_cron(&ScheduleDto::Daily {
        time: "09:30".into(),
    })
    .unwrap();
    assert_eq!(r.trigger_kind, "cron");
    assert_eq!(r.cron, "30 9 * * *");
}

#[test]
fn schedule_to_cron_weekly_uses_cron_field_for_weekday() {
    let r = schedule_to_cron(&ScheduleDto::Weekly {
        day: Weekday::Sun,
        time: "09:30".into(),
    })
    .unwrap();
    assert_eq!(r.trigger_kind, "cron");
    assert!(
        r.cron.ends_with("0") || r.cron.ends_with("7"),
        "Sun cron field is 0 or 7, got {:?}",
        r.cron
    );
}

#[test]
fn schedule_to_cron_weekdays_emits_1_5_field() {
    let r = schedule_to_cron(&ScheduleDto::Weekdays {
        time: "07:00".into(),
    })
    .unwrap();
    assert_eq!(r.cron, "0 7 * * 1-5");
}

#[test]
fn schedule_to_cron_hourly_within_range() {
    let r = schedule_to_cron(&ScheduleDto::Hourly { every_n_hours: 6 }).unwrap();
    assert_eq!(r.cron, "0 */6 * * *");
}

#[test]
fn schedule_to_cron_manual_emits_manual_kind_no_cron() {
    let r = schedule_to_cron(&ScheduleDto::Manual).unwrap();
    assert_eq!(r.trigger_kind, "manual");
    assert!(r.cron.is_empty());
}

#[test]
fn schedule_to_cron_custom_round_trips() {
    let r = schedule_to_cron(&ScheduleDto::Custom {
        cron: "*/15 * * * *".into(),
    })
    .unwrap();
    assert_eq!(r.trigger_kind, "cron");
    assert_eq!(r.cron, "*/15 * * * *");
}

#[test]
fn schedule_to_cron_rejects_bad_inputs() {
    assert!(schedule_to_cron(&ScheduleDto::Daily {
        time: "25:00".into()
    })
    .is_err());
    assert!(schedule_to_cron(&ScheduleDto::Hourly { every_n_hours: 0 }).is_err());
    assert!(schedule_to_cron(&ScheduleDto::Hourly { every_n_hours: 99 }).is_err());
    assert!(schedule_to_cron(&ScheduleDto::Custom { cron: "".into() }).is_err());
}

// ===================================================================
// Section 4 — Apply pipeline on real disk (tempdir-scoped).
// ===================================================================

fn apply_config_for(scope_root: &Path, ops: &[&str]) -> claudepot_core::templates::ApplyConfig {
    use claudepot_core::templates::{ApplyConfig, ApplyOperation, ApplyScope};
    let allowed_operations = ops
        .iter()
        .map(|s| match *s {
            "move" => ApplyOperation::Move,
            "rename" => ApplyOperation::Rename,
            "mkdir" => ApplyOperation::Mkdir,
            "write" => ApplyOperation::Write,
            "delete" => ApplyOperation::Delete,
            other => panic!("unknown op {other}"),
        })
        .collect();
    ApplyConfig {
        scope: ApplyScope {
            allowed_paths: vec![format!("{}/**", scope_root.display())],
            deny_outside: true,
        },
        allowed_operations,
        pending_changes_path: "{output_dir}/.pending-changes.json".into(),
        schema_version: 1,
        item_id_strategy: claudepot_core::templates::ItemIdStrategy::ContentHash,
    }
}

#[tokio::test]
async fn apply_executor_moves_files_inside_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let from = tmp.path().join("a.txt");
    let to_dir = tmp.path().join("Documents");
    let to = to_dir.join("a.txt");
    fs::write(&from, b"hi").unwrap();

    let apply = apply_config_for(tmp.path(), &["move", "mkdir"]);
    let pending = PendingChanges {
        schema_version: 1,
        automation_id: "test-auto".into(),
        run_id: "test-run".into(),
        generated_at: "2026-05-02T00:00:00Z".into(),
        summary: "1 move".into(),
        groups: vec![PendingGroup {
            id: "g1".into(),
            title: "g1".into(),
            items: vec![PendingItem {
                id: "i1".into(),
                description: "move a.txt → Documents/a.txt".into(),
                operation: Operation::Move {
                    from: from.clone(),
                    to: to.clone(),
                },
            }],
        }],
    };

    let receipt = apply_selected(&pending, &apply, &["i1".to_string()]).await;
    assert_eq!(receipt.outcomes.len(), 1);
    assert!(matches!(receipt.outcomes[0].outcome, ItemOutcome::Applied));
    assert!(!from.exists(), "source must be gone after move");
    assert!(to.exists(), "destination must exist after move");
}

#[test]
fn validator_rejects_op_kind_not_in_allow_list() {
    let tmp = tempfile::tempdir().unwrap();
    let inside = tmp.path().join("a.txt");
    fs::write(&inside, b"x").unwrap();

    // allow only `move`; submit a `delete`.
    let apply = apply_config_for(tmp.path(), &["move"]);
    let err = validate_item(
        &Operation::Delete {
            path: inside.clone(),
            must_be_empty: false,
        },
        &apply,
    )
    .expect_err("delete must be rejected when allow-list is move-only");
    assert!(matches!(
        err,
        claudepot_core::templates::apply::validator::ValidationError::OperationNotAllowed(_),
    ));
}

#[test]
fn validator_rejects_paths_outside_scope() {
    let scope = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("foreign.txt");
    fs::write(&outside_file, b"x").unwrap();

    let apply = apply_config_for(scope.path(), &["move"]);
    let err = validate_item(
        &Operation::Move {
            from: outside_file.clone(),
            to: scope.path().join("trespass.txt"),
        },
        &apply,
    )
    .expect_err("path outside scope must be rejected");
    // Either OutsideScope or SymlinkEscape is acceptable — on macOS
    // `/var/folders/...` is a symlink to `/private/var/folders/...`
    // and depending on which path participates in canonicalization,
    // the validator may classify the rejection either way. The
    // important property is that it IS rejected, with the right
    // error family.
    assert!(
        matches!(
            err,
            claudepot_core::templates::apply::validator::ValidationError::OutsideScope(_)
                | claudepot_core::templates::apply::validator::ValidationError::SymlinkEscape(_)
        ),
        "expected OutsideScope or SymlinkEscape, got {err:?}"
    );
}

#[test]
fn validator_rejects_traversal_via_dotdot() {
    let scope = tempfile::tempdir().unwrap();
    let inside_file = scope.path().join("real.txt");
    fs::write(&inside_file, b"x").unwrap();

    let apply = apply_config_for(scope.path(), &["move"]);
    // /<scope>/Documents/../../../escape — traverses out.
    let escape = scope
        .path()
        .join("Documents")
        .join("..")
        .join("..")
        .join("..")
        .join("escape.txt");

    let err = validate_item(
        &Operation::Move {
            from: inside_file,
            to: escape,
        },
        &apply,
    )
    .expect_err("dotdot traversal must be rejected after normalization");
    assert!(matches!(
        err,
        claudepot_core::templates::apply::validator::ValidationError::OutsideScope(_),
    ));
}

#[cfg(unix)]
#[test]
fn validator_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let scope = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();

    // /scope/link → /outside (escapes scope after resolving).
    let link = scope.path().join("link");
    symlink(outside.path(), &link).unwrap();
    let target_via_link = link.join("target.txt");

    let apply = apply_config_for(scope.path(), &["write"]);
    let err = validate_item(
        &Operation::Write {
            path: target_via_link,
            content_b64: "aGk=".into(), // "hi"
        },
        &apply,
    )
    .expect_err("symlink that resolves outside scope must be rejected");
    assert!(
        matches!(
            err,
            claudepot_core::templates::apply::validator::ValidationError::SymlinkEscape(_)
                | claudepot_core::templates::apply::validator::ValidationError::OutsideScope(_)
        ),
        "expected SymlinkEscape or OutsideScope, got {err:?}"
    );
}

// ===================================================================
// Section 5 — Routing rules round-trip + first-match-wins evaluator.
// ===================================================================

#[test]
fn routing_store_round_trips_through_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("routing-rules.json");

    let mut store = RoutingStore::at(path.clone());
    let rules = RoutingRules {
        schema_version: 1,
        rules: vec![
            RoutingRule {
                id: "r1".into(),
                match_: Match {
                    blueprint_privacy: Some("local".into()),
                    ..Match::default()
                },
                use_route: UseRoute::FirstLocalCapable,
            },
            RoutingRule {
                id: "r2".into(),
                match_: Match::default(),
                use_route: UseRoute::PrimaryAnthropic,
            },
        ],
    };
    store.replace(rules.clone());
    store.save().unwrap();

    let reloaded = routing_at_path(&path).expect("reload from disk must succeed");
    assert_eq!(
        reloaded.rules(),
        &rules,
        "round-trip must preserve every field"
    );
}

#[test]
fn routing_evaluator_first_match_wins() {
    let rules = RoutingRules {
        schema_version: 1,
        rules: vec![
            RoutingRule {
                id: "first".into(),
                match_: Match {
                    blueprint_category: Some("caregiver".into()),
                    ..Match::default()
                },
                use_route: UseRoute::PrimaryAnthropic,
            },
            RoutingRule {
                id: "second".into(),
                match_: Match::default(),
                use_route: UseRoute::CheapestCapable,
            },
        ],
    };

    let routes: Vec<&Route> = vec![];
    let s = evaluate(
        &rules,
        "local",
        "caregiver",
        "trivial",
        "caregiver.heartbeat",
        &routes,
        &|_| true,
        &|_| true,
    );
    // The first rule fires — PrimaryAnthropic = DefaultClaude.
    assert_eq!(s, Suggestion::DefaultClaude);
}

#[test]
fn routing_evaluator_falls_through_when_no_rule_matches() {
    let rules = RoutingRules {
        schema_version: 1,
        rules: vec![RoutingRule {
            id: "narrow".into(),
            match_: Match {
                blueprint_id_pattern: Some("network.*".into()),
                ..Match::default()
            },
            use_route: UseRoute::PrimaryAnthropic,
        }],
    };
    let routes: Vec<&Route> = vec![];
    let s = evaluate(
        &rules,
        "any",
        "it-health",
        "trivial",
        "it.morning-health-check",
        &routes,
        &|_| true,
        &|_| true,
    );
    assert_eq!(s, Suggestion::DefaultClaude);
}

// ===================================================================
// Section 6 — Caregiver consent file is mode 0600 on Unix.
// ===================================================================

#[cfg(unix)]
#[test]
fn caregiver_consent_file_is_mode_0600() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let store = ConsentStore::at(tmp.path().to_path_buf());

    let record = ConsentRecord {
        id: ConsentRecord::new_id(),
        automation_id: "auto-1".into(),
        blueprint_id: "caregiver.weekly-report".into(),
        blueprint_version: 1,
        dependent_label: "Dad's MacBook".into(),
        dependent_typed_name: "Dad".into(),
        caregiver_email: "kid@example.com".into(),
        smtp_provider: SmtpProvider::Generic,
        report_scope_shown: vec!["health".into(), "anomalies".into()],
        consented_at: "2026-05-02T00:00:00Z".into(),
        revoked_at: None,
        revoke_reason: None,
        schedule_changes: vec![],
    };
    let saved = store.create(record).unwrap();
    let path = tmp.path().join(format!("{}.json", saved.id));

    let meta = std::fs::metadata(&path).expect("consent file must exist");
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "consent record permissions must be 0600 (got {mode:o})"
    );
}

#[test]
fn caregiver_consent_record_is_active_until_revoked() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ConsentStore::at(tmp.path().to_path_buf());

    let r = store
        .create(ConsentRecord {
            id: ConsentRecord::new_id(),
            automation_id: "auto-1".into(),
            blueprint_id: "caregiver.heartbeat".into(),
            blueprint_version: 1,
            dependent_label: "x".into(),
            dependent_typed_name: "x".into(),
            caregiver_email: "x@example.com".into(),
            smtp_provider: SmtpProvider::Generic,
            report_scope_shown: vec![],
            consented_at: "2026-05-02T00:00:00Z".into(),
            revoked_at: None,
            revoke_reason: None,
            schedule_changes: vec![],
        })
        .unwrap();
    assert!(r.is_active());

    let revoked = store
        .revoke(
            &r.id,
            claudepot_core::templates::caregiver::consent::RevokeReason::UserRequest,
        )
        .unwrap();
    assert!(!revoked.is_active(), "revoked record must not be active");
    assert!(revoked.revoked_at.is_some());
}

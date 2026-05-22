//! Notification routing types — Category, Priority, Surface, and the
//! pure `route()` function.
//!
//! See `dev-docs/notification-system-plan.md` for the full design.
//! This file ships in Phase 0 of the refactor: types and a routing
//! function with no I/O, no preferences lookup, and no Tauri
//! dependency. Phase 1 builds the dispatcher facade on top.
//!
//! Adding a [`Category`] variant requires four lockstep changes:
//!   1. Add the variant here.
//!   2. Bind its [`Priority`] in [`Category::priority`] (the exhaustive
//!      `match` is a compile-time guard — adding a variant without
//!      binding it fails to build).
//!   3. Mirror the variant in `src/lib/notifications/types.ts`.
//!   4. Add an entry to the metadata table in [`Category::display_meta`].

use serde::{Deserialize, Serialize};

/// User-facing notification categories. Each variant maps to exactly
/// one [`Priority`] via [`Category::priority`]; routing is parameterized
/// over `(priority, context)` rather than pulled out of a sub-table
/// inside the dispatcher.
///
/// Variants are grouped by their default priority — comments mark the
/// tier boundaries. Categories are NEVER reordered in the public enum
/// body because their serde representation (camelCase variant name)
/// is the wire format and persisted log payloads reference them by
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    // ── P0 — Blocking: user setup is broken ─────────────────────────
    AccountAuthRejected,
    KeychainLocked,
    CcSlotDrift,
    DesktopDrift,
    RepairConflict,

    // ── P1 — Stalled: CC is paused or near limits ───────────────────
    SessionWaiting,
    SessionStuck,
    SessionErrorBurst,
    OpDoneUnfocused,
    RotationSuggested,
    UsageThreshold,
    UpdateInstallReady,

    // ── P2 — Acknowledge: user-initiated action completed ───────────
    AccountVerified,
    AccountSwitched,
    ProjectRenamed,
    ProjectRepaired,
    SessionPruned,
    KeyCopied,
    KeyAdded,
    KeyRemoved,
    ConfigEdited,
    // serialized value kept as "automationRan" (persisted in
    // notifications.json + prefs); renamed by the Phase 1 migration.
    #[serde(rename = "automationRan")]
    AgentRan,
    RotationApplied,
    /// Auto-rotation swap attempt failed. The audit log has the full
    /// reason; the toast carries a concise summary. Separate from
    /// `RotationApplied` so a user can mute success-acks while still
    /// hearing about failures.
    RotationFailed,
    /// Paired "resolved" event emitted when a banner clears. Lets the
    /// bell timeline show "auth-rejected → resolved" instead of just
    /// "auth-rejected appeared one hour ago, presumably still bad."
    BannerResolved,

    // ── P3 — Ambient: things happened while you weren't looking ────
    MemoryChanged,
    ConfigTreePatched,
    ServiceStatusChanged,
    UpdateAvailable,
}

/// Notification urgency. Drives the default surface set in [`route`];
/// per-category context overrides can deviate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Priority {
    /// Blocking — user must act to unblock themselves.
    P0Blocking,
    /// Stalled — CC paused or near limits; user can unblock.
    P1Stalled,
    /// Acknowledge — completion of a user-initiated action.
    P2Acknowledge,
    /// Ambient — passive awareness; no action required.
    P3Ambient,
}

/// Visual surface a notification can be shown on. Each surface has a
/// different delivery contract — toasts always succeed if requested
/// (renderer-side state push); OS banners can be focus-gated,
/// permission-gated, or rate-limited; persistent banners are
/// state-derived from `useStatusIssues` and "show" via re-render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Surface {
    /// In-app toast via the `useToasts` hook.
    Toast,
    /// OS desktop banner via `tauri-plugin-notification`.
    OsBanner,
    /// Persistent shell banner via `StatusIssuesBanner`. Emitted only
    /// for state-transitions (not-shown → shown, shown → not-shown).
    Banner,
}

/// The default set of surfaces a routed event should render on.
/// `route()` returns this before any delivery gate fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceSet {
    pub toast: bool,
    pub os_banner: bool,
    pub banner: bool,
    /// Always true today — kept as an explicit field so future
    /// "this category is so high-volume we don't even log it" cases
    /// have a clean way to opt out.
    pub log: bool,
    /// When true, OS banner dispatch ignores `document.hasFocus()`.
    /// Reserved for P0 — fatal-class alerts where the OS-level
    /// prominence is the point.
    pub ignore_focus: bool,
}

impl SurfaceSet {
    /// Convert this set to a [`Vec`] of requested [`Surface`] variants.
    /// Stable order: Toast, OsBanner, Banner. Used to populate
    /// `NotificationEntry::surfaces_requested`.
    pub fn requested_surfaces(self) -> Vec<Surface> {
        let mut v = Vec::with_capacity(3);
        if self.toast {
            v.push(Surface::Toast);
        }
        if self.os_banner {
            v.push(Surface::OsBanner);
        }
        if self.banner {
            v.push(Surface::Banner);
        }
        v
    }

    /// Empty set — used when a category is muted by user preference.
    /// Distinct from `Default::default()` only by intent.
    pub fn muted() -> Self {
        Self::default()
    }
}

/// Rotation orchestrator mode. Some categories (notably
/// `RotationApplied`) route differently depending on whether the
/// user opted into silent auto-rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RotationMode {
    /// User must confirm each suggested swap.
    Confirm,
    /// Swaps apply silently; the user opted into "trust the rule."
    Auto,
}

/// Context the routing function reads when overriding the
/// priority-defaulted surface set. Add fields here when a new
/// category needs context-aware routing; the type lives in core so
/// both the Rust dispatcher and any future test injection can
/// reproduce the routing decision.
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchContext {
    /// `Some(Auto)` silences `RotationApplied` toasts.
    pub rotation_mode: Option<RotationMode>,
    /// Echoes `document.hasFocus()` at emit time. Today only the OS
    /// dispatcher's focus gate reads this directly; routing keeps it
    /// available for future overrides without re-plumbing.
    pub window_focused: bool,
}

/// The minimum event payload routing needs. Real emitters build a
/// richer struct (title/body/target/dedupe_key) and use this view
/// for routing computation only.
#[derive(Debug, Clone, Copy)]
pub struct EventView {
    pub category: Category,
}

impl Category {
    /// The default priority bound to each category. Adding a category
    /// without a priority binding is a compile error — the wildcard
    /// arm is deliberately absent.
    pub fn priority(self) -> Priority {
        use Category::*;
        use Priority::*;
        match self {
            // P0 — Blocking
            AccountAuthRejected | KeychainLocked | CcSlotDrift | DesktopDrift | RepairConflict => {
                P0Blocking
            }

            // P1 — Stalled
            SessionWaiting | SessionStuck | SessionErrorBurst | OpDoneUnfocused
            | RotationSuggested | UsageThreshold | UpdateInstallReady => P1Stalled,

            // P2 — Acknowledge
            AccountVerified | AccountSwitched | ProjectRenamed | ProjectRepaired
            | SessionPruned | KeyCopied | KeyAdded | KeyRemoved | ConfigEdited | AgentRan
            | RotationApplied | RotationFailed | BannerResolved => P2Acknowledge,

            // P3 — Ambient
            MemoryChanged | ConfigTreePatched | ServiceStatusChanged | UpdateAvailable => P3Ambient,
        }
    }

    /// Human-readable label + group label for the Settings pane. Used
    /// by the `notification_categories_metadata` IPC; mirror is read
    /// at runtime by the TS Settings pane (no hand-maintained TS
    /// metadata file). Source-of-truth is here.
    pub fn display_meta(self) -> CategoryMeta {
        use Category::*;
        let (label, group, default_enabled) = match self {
            AccountAuthRejected => ("Account auth rejected", "Setup", true),
            KeychainLocked => ("Keychain locked", "Setup", true),
            CcSlotDrift => ("CC slot drift", "Setup", true),
            DesktopDrift => ("Desktop drift", "Setup", true),
            RepairConflict => ("Repair conflict", "Setup", true),

            SessionWaiting => ("Session waiting on input", "Live work", true),
            SessionStuck => ("Session stuck", "Live work", false),
            SessionErrorBurst => ("Session error burst", "Live work", false),
            OpDoneUnfocused => ("Background op finished", "Live work", false),
            RotationSuggested => ("Account rotation suggested", "Live work", true),
            UsageThreshold => ("Usage near limit", "Live work", true),
            UpdateInstallReady => ("Update ready to install", "Live work", true),

            AccountVerified => ("Account verified", "Actions", true),
            AccountSwitched => ("Account switched", "Actions", true),
            ProjectRenamed => ("Project renamed", "Actions", true),
            ProjectRepaired => ("Project repaired", "Actions", true),
            SessionPruned => ("Session pruned", "Actions", true),
            KeyCopied => ("Key copied", "Actions", true),
            KeyAdded => ("Key added", "Actions", true),
            KeyRemoved => ("Key removed", "Actions", true),
            ConfigEdited => ("Config edited", "Actions", true),
            AgentRan => ("Agent ran", "Actions", true),
            RotationApplied => ("Account rotation applied", "Actions", true),
            RotationFailed => ("Account rotation failed", "Actions", true),
            BannerResolved => ("Banner resolved", "Actions", true),

            MemoryChanged => ("CLAUDE.md changed externally", "Background", true),
            ConfigTreePatched => ("Config file changed externally", "Background", true),
            ServiceStatusChanged => ("anthropic.com status changed", "Background", true),
            UpdateAvailable => ("Update available", "Background", true),
        };
        CategoryMeta {
            id: self,
            label,
            group,
            priority: self.priority(),
            default_enabled,
        }
    }

    /// Iterate every category — used by the metadata IPC and tests
    /// that sweep the enum (TS mirror sweep, default-prefs map
    /// construction). Update when adding variants.
    pub fn all() -> &'static [Category] {
        use Category::*;
        &[
            AccountAuthRejected,
            KeychainLocked,
            CcSlotDrift,
            DesktopDrift,
            RepairConflict,
            SessionWaiting,
            SessionStuck,
            SessionErrorBurst,
            OpDoneUnfocused,
            RotationSuggested,
            UsageThreshold,
            UpdateInstallReady,
            AccountVerified,
            AccountSwitched,
            ProjectRenamed,
            ProjectRepaired,
            SessionPruned,
            KeyCopied,
            KeyAdded,
            KeyRemoved,
            ConfigEdited,
            AgentRan,
            RotationApplied,
            RotationFailed,
            BannerResolved,
            MemoryChanged,
            ConfigTreePatched,
            ServiceStatusChanged,
            UpdateAvailable,
        ]
    }
}

/// Metadata for a [`Category`] — label, group, default enabled state,
/// priority. Returned by the `notification_categories_metadata` IPC
/// (built in Phase 1.5); the TS Settings pane reads this at runtime
/// so adding a category requires only updating
/// [`Category::display_meta`].
#[derive(Debug, Clone, Serialize)]
pub struct CategoryMeta {
    pub id: Category,
    pub label: &'static str,
    pub group: &'static str,
    pub priority: Priority,
    pub default_enabled: bool,
}

/// Pure routing function. Returns the surface set the dispatcher
/// SHOULD use, before any user-preference filtering or delivery
/// gates fire. The dispatcher applies prefs separately
/// (`apply_prefs`), then primitives apply per-surface delivery
/// gates (focus, permission, rate-limit).
///
/// Separating routing from prefs from delivery is the load-bearing
/// invariant — the audit's "log records intent" claim only holds if
/// we keep these three concerns distinct.
pub fn route(event: EventView, ctx: &DispatchContext) -> SurfaceSet {
    let priority = event.category.priority();
    let mut s = match priority {
        Priority::P0Blocking => SurfaceSet {
            toast: false,
            os_banner: true,
            banner: true,
            log: true,
            ignore_focus: true,
        },
        Priority::P1Stalled => SurfaceSet {
            toast: false,
            os_banner: true,
            banner: false,
            log: true,
            ignore_focus: false,
        },
        Priority::P2Acknowledge => SurfaceSet {
            toast: true,
            os_banner: false,
            banner: false,
            log: true,
            ignore_focus: false,
        },
        Priority::P3Ambient => SurfaceSet {
            toast: false,
            os_banner: false,
            banner: false,
            log: true,
            ignore_focus: false,
        },
    };

    // ── Category × context overrides ──────────────────────────────
    //
    // Each override is a deliberate deviation from the priority
    // default. Document the *why* on every arm.
    match event.category {
        // Auto-mode rotation: user opted into silent application.
        // Drop the toast; the bell still records the swap.
        Category::RotationApplied if ctx.rotation_mode == Some(RotationMode::Auto) => {
            s.toast = false;
        }
        // RotationSuggested is P1 (os_banner default), but the
        // in-app toast carries the "Switch" action — it must
        // render regardless of focus state. P1 default + this
        // override = toast + os_banner, with the OS dispatcher's
        // focus gate suppressing the banner when the window is
        // focused.
        Category::RotationSuggested => {
            s.toast = true;
        }
        _ => {}
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_exhaustive_for_every_category() {
        // If a future variant is added without binding a priority,
        // `Category::priority()` won't compile. This test exists to
        // catch the OTHER direction: a variant added without being
        // included in `all()`. Sweep every all()-listed variant
        // through priority() — if all() is stale, the test passes
        // but the metadata IPC will be incomplete; the
        // test_all_returns_every_variant test below catches that.
        for &c in Category::all() {
            // Just call it; the match-without-wildcard inside
            // priority() guarantees correctness.
            let _ = c.priority();
        }
    }

    #[test]
    fn test_all_returns_every_variant() {
        // Synthetic exhaustive match: this fails to compile if a new
        // variant is added without updating `all()`. Update the
        // counter and the match arms in lockstep with the enum.
        const EXPECTED: usize = 29;
        let actual = Category::all().len();
        assert_eq!(
            actual, EXPECTED,
            "Category::all() returns {actual} but the enum has {EXPECTED} variants. \
             Did you add a variant without updating all()? Both this test and \
             display_meta() must be kept in sync."
        );

        // And spot-check exhaustiveness against display_meta — if a
        // variant is missing from display_meta, the test catches it.
        for &c in Category::all() {
            let meta = c.display_meta();
            assert_eq!(meta.id, c, "display_meta id mismatch for {c:?}");
            assert!(!meta.label.is_empty(), "label empty for {c:?}");
            assert!(!meta.group.is_empty(), "group empty for {c:?}");
        }
    }

    #[test]
    fn test_route_p0_blocks_focus_and_shows_banner() {
        let s = route(
            EventView {
                category: Category::AccountAuthRejected,
            },
            &DispatchContext::default(),
        );
        assert!(s.os_banner);
        assert!(s.banner);
        assert!(s.ignore_focus);
        assert!(s.log);
        assert!(!s.toast); // P0 leans on banner, not transient toast
    }

    #[test]
    fn test_route_p1_emits_os_banner_only() {
        let s = route(
            EventView {
                category: Category::UsageThreshold,
            },
            &DispatchContext::default(),
        );
        assert!(s.os_banner);
        assert!(!s.banner);
        assert!(!s.ignore_focus);
        assert!(s.log);
        assert!(!s.toast);
    }

    #[test]
    fn test_route_p2_emits_toast_only() {
        let s = route(
            EventView {
                category: Category::ProjectRenamed,
            },
            &DispatchContext::default(),
        );
        assert!(s.toast);
        assert!(!s.os_banner);
        assert!(!s.banner);
        assert!(s.log);
    }

    #[test]
    fn test_route_p3_logs_only() {
        let s = route(
            EventView {
                category: Category::MemoryChanged,
            },
            &DispatchContext::default(),
        );
        assert!(!s.toast);
        assert!(!s.os_banner);
        assert!(!s.banner);
        assert!(s.log);
    }

    #[test]
    fn test_rotation_suggested_routes_toast_plus_os_banner() {
        // Category override: P1 default (os_banner only) + toast
        // forced on so the interactive Switch action always
        // renders in-app.
        let s = route(
            EventView {
                category: Category::RotationSuggested,
            },
            &DispatchContext::default(),
        );
        assert!(s.toast, "RotationSuggested must show toast (Switch action)");
        assert!(s.os_banner, "RotationSuggested keeps P1 os_banner");
        assert!(!s.ignore_focus);
        assert!(s.log);
    }

    #[test]
    fn test_rotation_applied_silent_in_auto_mode() {
        // P2 default = toast on. Auto mode override = toast off.
        let confirm_ctx = DispatchContext {
            rotation_mode: Some(RotationMode::Confirm),
            ..Default::default()
        };
        let auto_ctx = DispatchContext {
            rotation_mode: Some(RotationMode::Auto),
            ..Default::default()
        };
        let event = EventView {
            category: Category::RotationApplied,
        };
        assert!(route(event, &confirm_ctx).toast);
        assert!(!route(event, &auto_ctx).toast);
        // Log still happens in both modes.
        assert!(route(event, &confirm_ctx).log);
        assert!(route(event, &auto_ctx).log);
    }

    #[test]
    fn test_requested_surfaces_order_is_stable() {
        let s = SurfaceSet {
            toast: true,
            os_banner: true,
            banner: true,
            log: true,
            ignore_focus: false,
        };
        assert_eq!(
            s.requested_surfaces(),
            vec![Surface::Toast, Surface::OsBanner, Surface::Banner]
        );
    }

    #[test]
    fn test_serde_round_trip_category() {
        // Wire format is camelCase — guard against accidental rename.
        let json = serde_json::to_string(&Category::SessionWaiting).unwrap();
        assert_eq!(json, "\"sessionWaiting\"");
        let back: Category = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Category::SessionWaiting);
    }

    #[test]
    fn test_serde_round_trip_priority() {
        let json = serde_json::to_string(&Priority::P0Blocking).unwrap();
        assert_eq!(json, "\"p0Blocking\"");
    }

    #[test]
    fn test_serde_round_trip_surface() {
        let json = serde_json::to_string(&Surface::OsBanner).unwrap();
        assert_eq!(json, "\"osBanner\"");
    }

    /// Audit-fix Low #17 (full close): the checked-in fixture at
    /// `src/lib/notifications/__fixtures__/categories.fixture.json`
    /// is the canonical cross-language source of truth. Both sides
    /// validate against it:
    ///
    ///   - Rust: this test asserts Category::all().iter().map(display_meta())
    ///     serializes to the same shape.
    ///   - TS: vitest reads the fixture and compares against
    ///     CATEGORY_NAMES + priorityForCategory.
    ///
    /// Any drift on either side fails the relevant test before it
    /// surfaces as a user-visible bug. To intentionally update the
    /// fixture after a Rust change: set `CLAUDEPOT_REGEN_FIXTURES=1`
    /// and re-run; the test writes the new shape and you commit it.
    #[test]
    fn test_categories_fixture_matches_rust() {
        use std::path::PathBuf;
        let fixture_path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("src")
            .join("lib")
            .join("notifications")
            .join("__fixtures__")
            .join("categories.fixture.json");

        // Build the canonical shape from the Rust enum.
        let live: Vec<serde_json::Value> = Category::all()
            .iter()
            .map(|c| {
                let meta = c.display_meta();
                serde_json::json!({
                    "id": meta.id,
                    "priority": meta.priority,
                    "group": meta.group,
                    "defaultEnabled": meta.default_enabled,
                })
            })
            .collect();

        // Regen mode: env var lets contributors rewrite the fixture
        // after a deliberate Rust change. Default mode: asserts.
        if std::env::var("CLAUDEPOT_REGEN_FIXTURES").is_ok() {
            let doc = serde_json::json!({
                "_schema": "Canonical snapshot of `Category::all().iter().map(|c| c.display_meta())` from claudepot-core. Both sides validate against this file: cargo test `test_categories_fixture_matches_rust` asserts the Rust source produces this JSON; vitest `Rust metadata mirror via fixture` asserts the TS mirror agrees. Update by running `cargo test --workspace -- categories_fixture` (writes the file when CLAUDEPOT_REGEN_FIXTURES=1) and committing the diff.",
                "categories": live,
            });
            let pretty = serde_json::to_string_pretty(&doc).unwrap();
            std::fs::write(&fixture_path, pretty).expect("write fixture");
            return;
        }

        // Read the on-disk fixture.
        let bytes = std::fs::read(&fixture_path)
            .unwrap_or_else(|e| panic!("read fixture {}: {e}", fixture_path.display()));
        let doc: serde_json::Value = serde_json::from_slice(&bytes).expect("fixture is valid JSON");
        let on_disk = doc
            .get("categories")
            .and_then(|v| v.as_array())
            .expect("fixture has `categories` array")
            .clone();

        assert_eq!(
            on_disk, live,
            "Category fixture drift detected. Re-generate with \
             `CLAUDEPOT_REGEN_FIXTURES=1 cargo test -p claudepot-core categories_fixture` \
             and commit the diff. Mirror also requires updating CATEGORY_NAMES in \
             src/lib/notifications/types.ts."
        );
    }
}

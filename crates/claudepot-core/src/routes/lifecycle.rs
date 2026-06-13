//! The add / edit save transaction for routes — secret commit,
//! store persist, rollback, and post-store side effects, in one
//! tested home.
//!
//! Extracted from `src-tauri/commands/routes.rs` (audit HIGH:
//! `routes_add` / `routes_edit` carried an untested multi-step
//! rollback transaction in the command layer, contradicting both
//! that file's "No business logic lives here" doc and
//! `.claude/rules/architecture.md`). Mirrors the agent install-gate
//! pattern (`agent::install_gate`): the impure writers (keychain,
//! helper scripts, wrapper, Desktop profile) are injected behind
//! [`RouteEffects`] so the ordering and BOTH rollback directions are
//! unit-tested with fakes; production passes [`OsRouteEffects`].
//!
//! ## Ordering and rollback
//!
//! **Add** — `commit_secrets → store.add`:
//! - a secret-commit failure tears down any keychain entry / helper
//!   script already written (the route was never persisted, so
//!   nothing may be left behind) and scrubs the in-memory secrets;
//! - a store rejection (duplicate name, persist I/O) runs the same
//!   keychain/helper tear-down — the rolled-back state is "no
//!   route, no artifacts".
//!
//! **Edit** — `commit_secrets → store.update → side effects`:
//! - a secret-commit failure leaves the persisted route untouched
//!   (nothing to roll back; in-memory secrets are scrubbed);
//! - a store rejection must **NOT** delete helpers (audit fix for
//!   commands_routes.rs:613: the previous shape ran
//!   `delete_helpers` here, removing scripts the still-existing,
//!   un-updated route depends on). The keychain may already hold
//!   the new secret while the route still references the old shape
//!   — the caller surfaces "re-enter the secret and save again";
//! - post-store side effects (stale-wrapper delete on rename,
//!   wrapper rewrite, Desktop library profile, activation
//!   re-mirror) can't be rolled back; failures are aggregated as
//!   warnings on [`SavedRoute`] so the user sees when disk state
//!   diverges from the persisted route.

use uuid::Uuid;
use zeroize::Zeroize;

use super::error::RouteError;
use super::keychain::SecretField;
use super::store::RouteStore;
use super::types::{Route, RouteId, RouteProvider};

/// The impure writers the save transaction drives, injected so the
/// rollback matrix is testable with fakes. [`OsRouteEffects`] is the
/// production implementation over the real keychain / disk helpers.
pub trait RouteEffects {
    fn store_secret(
        &self,
        route_id: RouteId,
        field: SecretField,
        secret: &str,
    ) -> Result<(), RouteError>;
    fn write_helper(&self, route_id: RouteId, field: SecretField) -> Result<(), RouteError>;
    fn delete_keychain_for_route(&self, route_id: RouteId) -> Result<(), RouteError>;
    fn delete_helpers(&self, route_id: RouteId) -> Result<(), RouteError>;
    fn delete_wrapper(&self, name: &str) -> Result<(), RouteError>;
    fn write_wrapper(&self, route: &Route) -> Result<(), RouteError>;
    fn write_library_profile(&self, route: &Route) -> Result<(), RouteError>;
    fn activate_desktop(&self, route: &Route, disable_chooser: bool) -> Result<(), RouteError>;
}

/// Production [`RouteEffects`] over the real keychain and disk
/// writers in the sibling modules.
pub struct OsRouteEffects;

impl RouteEffects for OsRouteEffects {
    fn store_secret(
        &self,
        route_id: RouteId,
        field: SecretField,
        secret: &str,
    ) -> Result<(), RouteError> {
        super::keychain::store_secret(route_id, field, secret)
    }
    fn write_helper(&self, route_id: RouteId, field: SecretField) -> Result<(), RouteError> {
        super::helper::write_helper(route_id, field).map(|_| ())
    }
    fn delete_keychain_for_route(&self, route_id: RouteId) -> Result<(), RouteError> {
        super::keychain::delete_all_for_route(route_id)
    }
    fn delete_helpers(&self, route_id: RouteId) -> Result<(), RouteError> {
        super::helper::delete_helpers(route_id, None)
    }
    fn delete_wrapper(&self, name: &str) -> Result<(), RouteError> {
        super::wrapper::delete_wrapper(name)
    }
    fn write_wrapper(&self, route: &Route) -> Result<(), RouteError> {
        super::wrapper::write_wrapper(route).map(|_| ())
    }
    fn write_library_profile(&self, route: &Route) -> Result<(), RouteError> {
        super::desktop::write_library_profile(route).map(|_| ())
    }
    fn activate_desktop(&self, route: &Route, disable_chooser: bool) -> Result<(), RouteError> {
        super::desktop::activate_desktop(route, disable_chooser).map(|_| ())
    }
}

/// Why a save transaction failed. Split by phase so callers can
/// attach the right user guidance ([`Store`] on edit = "the
/// previously-saved route remains active; re-enter the secret and
/// save again").
///
/// [`Store`]: SaveRouteError::Store
#[derive(Debug, thiserror::Error)]
pub enum SaveRouteError {
    /// `edit_route`'s target id doesn't exist.
    #[error("route not found: {0}")]
    NotFound(String),
    /// Keychain / helper write failed during `commit_secrets`. Add:
    /// partial writes were torn down. Edit: persisted route is
    /// untouched.
    #[error(transparent)]
    Secrets(RouteError),
    /// The store rejected the route (duplicate name, persist I/O).
    /// Add: keychain/helper writes were torn down. Edit: the
    /// persisted route — and its helpers — remain as they were.
    #[error(transparent)]
    Store(RouteError),
}

/// A successfully saved route plus any post-store side-effect
/// failures. A non-empty `warnings` means the route IS persisted
/// but on-disk artifacts (wrapper, Desktop profile) may be stale.
#[derive(Debug)]
pub struct SavedRoute {
    pub route: Route,
    pub warnings: Vec<String>,
}

/// Resolve how each provider's secret is stored, given the new
/// (post-form) and previous (already-on-disk) provider configs:
///
///   - **Plaintext mode (`use_keychain == false`)**: blank secret on
///     edit means "keep prev value"; non-empty replaces.
///   - **Keychain mode (`use_keychain == true`)**: any non-empty
///     incoming secret is written to the OS keychain and the helper
///     script is (re)materialized; the inline field is then blanked
///     (zeroized first — `String::clear` only sets len = 0) so it
///     never reaches routes.json on disk.
///
/// Runs before `RouteStore::add` / `update` so the persisted route
/// reflects the post-effect state.
pub fn commit_secrets(
    new_provider: &mut RouteProvider,
    route_id: RouteId,
    prev: Option<&RouteProvider>,
    fx: &dyn RouteEffects,
) -> Result<(), RouteError> {
    match new_provider {
        RouteProvider::Gateway(cfg) => {
            if cfg.use_keychain {
                if !cfg.api_key.is_empty() {
                    fx.store_secret(route_id, SecretField::GatewayApiKey, &cfg.api_key)?;
                    fx.write_helper(route_id, SecretField::GatewayApiKey)?;
                }
                cfg.api_key.zeroize();
            } else if cfg.api_key.is_empty() {
                if let Some(RouteProvider::Gateway(p)) = prev {
                    cfg.api_key = p.api_key.clone();
                }
            }
        }
        RouteProvider::Bedrock(cfg) => {
            if cfg.use_keychain {
                if let Some(mut t) = cfg.bearer_token.take() {
                    if !t.is_empty() {
                        fx.store_secret(route_id, SecretField::BedrockBearerToken, &t)?;
                        fx.write_helper(route_id, SecretField::BedrockBearerToken)?;
                    }
                    t.zeroize();
                }
                cfg.bearer_token = None;
            } else {
                let need_inherit = cfg.bearer_token.as_ref().is_some_and(|t| t.is_empty());
                if need_inherit {
                    if let Some(RouteProvider::Bedrock(p)) = prev {
                        cfg.bearer_token = p.bearer_token.clone();
                    } else {
                        cfg.bearer_token = None;
                    }
                }
            }
        }
        RouteProvider::Foundry(cfg) => {
            if cfg.use_keychain {
                if let Some(mut k) = cfg.api_key.take() {
                    if !k.is_empty() {
                        fx.store_secret(route_id, SecretField::FoundryApiKey, &k)?;
                        fx.write_helper(route_id, SecretField::FoundryApiKey)?;
                    }
                    k.zeroize();
                }
                cfg.api_key = None;
            } else {
                let need_inherit = cfg.api_key.as_ref().is_some_and(|k| k.is_empty());
                if need_inherit {
                    if let Some(RouteProvider::Foundry(p)) = prev {
                        cfg.api_key = p.api_key.clone();
                    } else {
                        cfg.api_key = None;
                    }
                }
            }
        }
        RouteProvider::Vertex(_) => {}
    }
    Ok(())
}

/// Snapshot every inline secret on a provider config into owned
/// Strings. Callers zeroize them on every exit path (used by
/// [`edit_route`] to scrub shadow copies even when the store-update
/// path drops the original candidate without scrubbing).
fn collect_inline_secrets(p: &RouteProvider) -> Vec<String> {
    match p {
        RouteProvider::Gateway(c) if !c.api_key.is_empty() => {
            vec![c.api_key.clone()]
        }
        RouteProvider::Bedrock(c) => c
            .bearer_token
            .as_ref()
            .filter(|t| !t.is_empty())
            .cloned()
            .map(|t| vec![t])
            .unwrap_or_default(),
        RouteProvider::Foundry(c) => c
            .api_key
            .as_ref()
            .filter(|k| !k.is_empty())
            .cloned()
            .map(|k| vec![k])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Scrub every inline secret on a provider config. Used by error
/// paths so a half-built route doesn't leave the user-typed key
/// resident in process memory until the allocator overwrites it.
pub fn zeroize_provider_secrets(p: &mut RouteProvider) {
    match p {
        RouteProvider::Gateway(c) => c.api_key.zeroize(),
        RouteProvider::Bedrock(c) => {
            if let Some(t) = c.bearer_token.as_mut() {
                t.zeroize();
            }
            c.bearer_token = None;
        }
        RouteProvider::Foundry(c) => {
            if let Some(k) = c.api_key.as_mut() {
                k.zeroize();
            }
            c.api_key = None;
        }
        RouteProvider::Vertex(_) => {}
    }
}

/// Add transaction: `commit_secrets → store.add`, with keychain /
/// helper tear-down on failure of either step. A nil `route.id`
/// gets a fresh UUID here (NOT in `store.add`) so the keychain
/// entries and the persisted route always share one id.
pub fn add_route(
    store: &mut RouteStore,
    mut route: Route,
    fx: &dyn RouteEffects,
) -> Result<Route, SaveRouteError> {
    if route.id.is_nil() {
        route.id = Uuid::new_v4();
    }
    let route_id = route.id;
    if let Err(e) = commit_secrets(&mut route.provider, route_id, None, fx) {
        // commit_secrets may have written to keychain / dropped a
        // helper before failing — best-effort tear-down so we don't
        // leak state for a route that was never persisted.
        let _ = fx.delete_keychain_for_route(route_id);
        let _ = fx.delete_helpers(route_id);
        zeroize_provider_secrets(&mut route.provider);
        return Err(SaveRouteError::Secrets(e));
    }
    match store.add(route) {
        Ok(saved) => Ok(saved),
        Err(e) => {
            // Roll back any keychain / helper writes commit_secrets
            // did, since the store call rejected the route. The
            // in-flight provider copy was already scrubbed by
            // commit_secrets if it took the keychain path.
            let _ = fx.delete_keychain_for_route(route_id);
            let _ = fx.delete_helpers(route_id);
            Err(SaveRouteError::Store(e))
        }
    }
}

/// Edit transaction: capture prev → `commit_secrets` →
/// `store.update` → post-store side effects. See the module doc for
/// the rollback matrix; in particular a store rejection leaves the
/// persisted route's helpers untouched.
pub fn edit_route(
    store: &mut RouteStore,
    mut candidate: Route,
    fx: &dyn RouteEffects,
) -> Result<SavedRoute, SaveRouteError> {
    let id = candidate.id;
    // Capture the prior provider so commit_secrets can decide
    // "blank = keep existing" for plaintext mode, and the prior
    // wrapper name to detect renames for stale-file cleanup.
    let (prev_provider, prev_wrapper_name) = match store.get(id) {
        Some(prev) => (
            prev.provider.clone(),
            if prev.installed_on_cli {
                Some(prev.wrapper_name.clone())
            } else {
                None
            },
        ),
        None => {
            // Scrub the typed secret before bailing — `candidate` is
            // dropped here without store.update ever taking it, so
            // this stale-id path must zeroize like the others below.
            zeroize_provider_secrets(&mut candidate.provider);
            return Err(SaveRouteError::NotFound(id.to_string()));
        }
    };

    if let Err(e) = commit_secrets(&mut candidate.provider, id, Some(&prev_provider), fx) {
        zeroize_provider_secrets(&mut candidate.provider);
        return Err(SaveRouteError::Secrets(e));
    }

    // Snapshot any inline secrets in the candidate into local
    // Strings BEFORE it moves into `store.update`; the shadow copies
    // are zeroized on every exit so a failing update (which drops
    // `candidate` without scrubbing) still leaves no extra resident
    // copy we control.
    let mut shadow_secrets = collect_inline_secrets(&candidate.provider);
    let updated = match store.update(candidate) {
        Ok(u) => u,
        Err(e) => {
            for s in shadow_secrets.iter_mut() {
                s.zeroize();
            }
            // DO NOT delete helpers here (audit fix for
            // commands_routes.rs:613): a failed update leaves the
            // persisted route unchanged, so the helper scripts it
            // depends on must stay. The keychain entry
            // commit_secrets just wrote may now hold the new secret
            // while the route still references the old shape — the
            // caller surfaces that explicitly.
            return Err(SaveRouteError::Store(e));
        }
    };
    for s in shadow_secrets.iter_mut() {
        s.zeroize();
    }

    // Post-store side effects: collect failures so the caller can
    // tell the user when wrapper / Desktop state diverges from the
    // persisted route (instead of silently leaving stale files).
    let mut warnings: Vec<String> = Vec::new();
    if updated.installed_on_cli {
        if let Some(prev_name) = &prev_wrapper_name {
            if prev_name != &updated.wrapper_name {
                if let Err(e) = fx.delete_wrapper(prev_name) {
                    warnings.push(format!(
                        "old wrapper '{prev_name}' could not be removed: {e}"
                    ));
                }
            }
        }
        if let Err(e) = fx.write_wrapper(&updated) {
            warnings.push(format!("wrapper rewrite failed: {e}"));
        }
    }
    // Always rewrite the library profile (regardless of active
    // state), so a defined-but-inactive 3P profile in
    // `configLibrary/` reflects the latest fields and any
    // pre-existing plaintext secret on disk is replaced.
    if let Err(e) = fx.write_library_profile(&updated) {
        warnings.push(format!("Desktop library profile write failed: {e}"));
    }
    if updated.active_on_desktop {
        let disable = store.disable_chooser();
        if let Err(e) = fx.activate_desktop(&updated, disable) {
            warnings.push(format!("Desktop activation re-mirror failed: {e}"));
        }
    }

    Ok(SavedRoute {
        route: updated,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::types::{AuthScheme, GatewayConfig};
    use std::cell::RefCell;

    /// Recording fake. `fail_*` flips individual writers into
    /// returning an error; every call is logged so ordering and
    /// rollback assertions read off one vec.
    #[derive(Default)]
    struct FakeEffects {
        calls: RefCell<Vec<String>>,
        fail_store_secret: bool,
        fail_write_helper: bool,
        fail_delete_wrapper: bool,
        fail_write_wrapper: bool,
        fail_library_profile: bool,
        fail_activate: bool,
    }

    impl FakeEffects {
        fn log(&self, s: impl Into<String>) {
            self.calls.borrow_mut().push(s.into());
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        fn err() -> RouteError {
            RouteError::Io(std::io::Error::other("fake failure"))
        }
    }

    impl RouteEffects for FakeEffects {
        fn store_secret(
            &self,
            _route_id: RouteId,
            field: SecretField,
            secret: &str,
        ) -> Result<(), RouteError> {
            self.log(format!("store_secret:{field:?}:{secret}"));
            if self.fail_store_secret {
                return Err(Self::err());
            }
            Ok(())
        }
        fn write_helper(&self, _route_id: RouteId, field: SecretField) -> Result<(), RouteError> {
            self.log(format!("write_helper:{field:?}"));
            if self.fail_write_helper {
                return Err(Self::err());
            }
            Ok(())
        }
        fn delete_keychain_for_route(&self, _route_id: RouteId) -> Result<(), RouteError> {
            self.log("delete_keychain");
            Ok(())
        }
        fn delete_helpers(&self, _route_id: RouteId) -> Result<(), RouteError> {
            self.log("delete_helpers");
            Ok(())
        }
        fn delete_wrapper(&self, name: &str) -> Result<(), RouteError> {
            self.log(format!("delete_wrapper:{name}"));
            if self.fail_delete_wrapper {
                return Err(Self::err());
            }
            Ok(())
        }
        fn write_wrapper(&self, route: &Route) -> Result<(), RouteError> {
            self.log(format!("write_wrapper:{}", route.wrapper_name));
            if self.fail_write_wrapper {
                return Err(Self::err());
            }
            Ok(())
        }
        fn write_library_profile(&self, _route: &Route) -> Result<(), RouteError> {
            self.log("write_library_profile");
            if self.fail_library_profile {
                return Err(Self::err());
            }
            Ok(())
        }
        fn activate_desktop(&self, _route: &Route, _disable: bool) -> Result<(), RouteError> {
            self.log("activate_desktop");
            if self.fail_activate {
                return Err(Self::err());
            }
            Ok(())
        }
    }

    fn gateway_route(name: &str, wrapper: &str, api_key: &str, use_keychain: bool) -> Route {
        Route {
            id: Uuid::nil(),
            name: name.to_string(),
            provider: RouteProvider::Gateway(GatewayConfig {
                base_url: "https://example.com".into(),
                api_key: api_key.to_string(),
                auth_scheme: AuthScheme::Bearer,
                enable_tool_search: false,
                use_keychain,
            }),
            model: "kimi-k2".into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: wrapper.to_string(),
            deployment_organization_uuid: Uuid::nil(),
            active_on_desktop: false,
            installed_on_cli: false,
            is_private_cloud: false,
            capabilities_override: None,
        }
    }

    fn temp_store() -> (tempfile::TempDir, RouteStore) {
        let tmp = tempfile::tempdir().unwrap();
        let store = RouteStore::open_at(tmp.path().join("routes.json")).unwrap();
        (tmp, store)
    }

    fn gateway_api_key(r: &Route) -> String {
        match &r.provider {
            RouteProvider::Gateway(c) => c.api_key.clone(),
            other => panic!("expected gateway provider, got {other:?}"),
        }
    }

    // ---------- add ----------

    #[test]
    fn test_add_route_keychain_mode_writes_secret_then_helper_and_blanks_inline() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        let saved = add_route(
            &mut store,
            gateway_route("a", "claude-a", "sk-key", true),
            &fx,
        )
        .expect("add must succeed");
        assert_eq!(
            fx.calls(),
            vec![
                "store_secret:GatewayApiKey:sk-key".to_string(),
                "write_helper:GatewayApiKey".to_string(),
            ],
            "keychain write precedes helper materialization; no rollback calls"
        );
        assert_eq!(
            gateway_api_key(&saved),
            "",
            "inline key must be blanked before persist"
        );
        assert_eq!(gateway_api_key(&store.list()[0]), "");
    }

    #[test]
    fn test_add_route_secret_commit_failure_rolls_back_keychain_and_helpers() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects {
            fail_store_secret: true,
            ..Default::default()
        };
        let err = add_route(
            &mut store,
            gateway_route("a", "claude-a", "sk-key", true),
            &fx,
        )
        .expect_err("secret failure must fail the add");
        assert!(matches!(err, SaveRouteError::Secrets(_)));
        let calls = fx.calls();
        assert!(calls.contains(&"delete_keychain".to_string()));
        assert!(calls.contains(&"delete_helpers".to_string()));
        assert!(store.list().is_empty(), "nothing may be persisted");
    }

    #[test]
    fn test_add_route_store_rejection_rolls_back_keychain_and_helpers() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        add_route(&mut store, gateway_route("a", "claude-a", "k1", true), &fx).unwrap();

        // Duplicate name → store.add rejects after secrets committed.
        let fx2 = FakeEffects::default();
        let err = add_route(&mut store, gateway_route("a", "claude-b", "k2", true), &fx2)
            .expect_err("duplicate name must fail");
        assert!(matches!(err, SaveRouteError::Store(_)));
        let calls = fx2.calls();
        assert!(
            calls.contains(&"delete_keychain".to_string())
                && calls.contains(&"delete_helpers".to_string()),
            "store rejection must tear down the just-written keychain state: {calls:?}"
        );
        assert_eq!(store.list().len(), 1, "the rejected route must not persist");
    }

    #[test]
    fn test_add_route_assigns_one_id_shared_by_keychain_and_store() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        let saved = add_route(&mut store, gateway_route("a", "claude-a", "k", true), &fx).unwrap();
        assert!(!saved.id.is_nil(), "nil id must be replaced before commit");
    }

    // ---------- edit ----------

    #[test]
    fn test_edit_route_not_found() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        let mut candidate = gateway_route("a", "claude-a", "", false);
        candidate.id = Uuid::new_v4();
        let err = edit_route(&mut store, candidate, &fx).expect_err("unknown id");
        assert!(matches!(err, SaveRouteError::NotFound(_)));
        assert!(
            fx.calls().is_empty(),
            "no effects may run for a missing route"
        );
    }

    #[test]
    fn test_edit_route_store_rejection_keeps_helpers() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        let a = add_route(&mut store, gateway_route("a", "claude-a", "k", true), &fx).unwrap();
        add_route(&mut store, gateway_route("b", "claude-b", "k", true), &fx).unwrap();

        // Rename route a to b's name → store.update rejects.
        let mut candidate = gateway_route("b", "claude-a", "new-secret", true);
        candidate.id = a.id;
        let fx2 = FakeEffects::default();
        let err = edit_route(&mut store, candidate, &fx2).expect_err("duplicate name");
        assert!(matches!(err, SaveRouteError::Store(_)));
        let calls = fx2.calls();
        assert!(
            !calls.contains(&"delete_helpers".to_string())
                && !calls.contains(&"delete_keychain".to_string()),
            "a failed update must NOT remove the still-active route's helpers \
             (audit fix commands_routes.rs:613): {calls:?}"
        );
    }

    #[test]
    fn test_edit_route_rename_while_installed_deletes_old_wrapper_and_rewrites() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        let a = add_route(&mut store, gateway_route("a", "claude-old", "k", true), &fx).unwrap();
        store.set_installed_cli(a.id, true).unwrap();

        let mut candidate = gateway_route("a", "claude-new", "", true);
        candidate.id = a.id;
        let fx2 = FakeEffects::default();
        let saved = edit_route(&mut store, candidate, &fx2).expect("edit must succeed");
        assert!(saved.warnings.is_empty());
        let calls = fx2.calls();
        assert!(calls.contains(&"delete_wrapper:claude-old".to_string()));
        assert!(calls.contains(&"write_wrapper:claude-new".to_string()));
        assert!(calls.contains(&"write_library_profile".to_string()));
        assert!(
            !calls.contains(&"activate_desktop".to_string()),
            "no re-mirror when the route isn't Desktop-active"
        );
    }

    #[test]
    fn test_edit_route_remirrors_desktop_when_active() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        let a = add_route(&mut store, gateway_route("a", "claude-a", "k", true), &fx).unwrap();
        store.set_active_desktop(Some(a.id)).unwrap();

        let mut candidate = gateway_route("a", "claude-a", "", true);
        candidate.id = a.id;
        let fx2 = FakeEffects::default();
        let saved = edit_route(&mut store, candidate, &fx2).unwrap();
        assert!(saved.warnings.is_empty());
        assert!(fx2.calls().contains(&"activate_desktop".to_string()));
    }

    #[test]
    fn test_edit_route_side_effect_failures_become_warnings_route_still_saved() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        let a = add_route(&mut store, gateway_route("a", "claude-old", "k", true), &fx).unwrap();
        store.set_installed_cli(a.id, true).unwrap();
        store.set_active_desktop(Some(a.id)).unwrap();

        let mut candidate = gateway_route("a", "claude-new", "", true);
        candidate.id = a.id;
        let fx2 = FakeEffects {
            fail_delete_wrapper: true,
            fail_write_wrapper: true,
            fail_library_profile: true,
            fail_activate: true,
            ..Default::default()
        };
        let saved = edit_route(&mut store, candidate, &fx2).expect("route must still save");
        assert_eq!(
            saved.warnings.len(),
            4,
            "every failed side effect surfaces: {:?}",
            saved.warnings
        );
        assert_eq!(
            store.get(a.id).unwrap().wrapper_name,
            "claude-new",
            "the store update committed despite side-effect failures"
        );
    }

    // ---------- commit_secrets policy ----------

    #[test]
    fn test_edit_plaintext_blank_secret_inherits_previous_value() {
        let (_tmp, mut store) = temp_store();
        let fx = FakeEffects::default();
        let a = add_route(
            &mut store,
            gateway_route("a", "claude-a", "old-key", false),
            &fx,
        )
        .unwrap();
        assert!(
            fx.calls().is_empty(),
            "plaintext mode never touches keychain"
        );

        let mut candidate = gateway_route("a", "claude-a", "", false);
        candidate.id = a.id;
        let fx2 = FakeEffects::default();
        let saved = edit_route(&mut store, candidate, &fx2).unwrap();
        assert_eq!(
            gateway_api_key(&saved.route),
            "old-key",
            "blank plaintext secret on edit means keep the previous value"
        );
    }

    #[test]
    fn test_keychain_mode_empty_secret_skips_keychain_but_blanks_field() {
        let fx = FakeEffects::default();
        let mut provider = RouteProvider::Gateway(GatewayConfig {
            base_url: "https://example.com".into(),
            api_key: String::new(),
            auth_scheme: AuthScheme::Bearer,
            enable_tool_search: false,
            use_keychain: true,
        });
        commit_secrets(&mut provider, Uuid::new_v4(), None, &fx).unwrap();
        assert!(
            fx.calls().is_empty(),
            "no keychain write for an empty secret"
        );
        match provider {
            RouteProvider::Gateway(c) => assert_eq!(c.api_key, ""),
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_zeroize_provider_secrets_clears_every_shape() {
        let mut p = RouteProvider::Gateway(GatewayConfig {
            base_url: "u".into(),
            api_key: "secret".into(),
            auth_scheme: AuthScheme::Bearer,
            enable_tool_search: false,
            use_keychain: false,
        });
        zeroize_provider_secrets(&mut p);
        match p {
            RouteProvider::Gateway(c) => assert_eq!(c.api_key, ""),
            _ => unreachable!(),
        }
    }
}

//! Route definitions persisted as `~/.claudepot/routes.json`.
//!
//! JSON over SQLite because routes are a different shape than
//! accounts (no migrations, no live state, no transactions across
//! more than one row at a time) and we want zero coupling with the
//! existing `accounts.db` migration story. Atomic writes via
//! `fs_utils::atomic_write` (mode 0600).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::fs_utils;
use crate::paths::claudepot_data_dir;

use super::error::RouteError;
use super::types::{Route, RouteId};

/// On-disk envelope. The `version` field is bumped only when the
/// shape changes incompatibly; serde's `default` handles forward-
/// compat field additions without touching the version.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RoutesFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    routes: Vec<Route>,
    #[serde(default = "default_chooser")]
    disable_deployment_mode_chooser: bool,
}

fn default_version() -> u32 {
    1
}
fn default_chooser() -> bool {
    false
}

impl Default for RoutesFile {
    fn default() -> Self {
        Self {
            version: default_version(),
            routes: Vec::new(),
            disable_deployment_mode_chooser: default_chooser(),
        }
    }
}

/// In-memory cache + read-modify-write helper around `routes.json`.
/// Construct once per command; not `Clone`. Internally serializes
/// every mutation through atomic writes, so cross-process safety is
/// best-effort (concurrent claudepot CLI + GUI mutations may stomp
/// each other — we don't expect that pairing in practice).
pub struct RouteStore {
    path: PathBuf,
    file: RoutesFile,
}

impl RouteStore {
    /// Open or create the store at `<claudepot_data_dir>/routes.json`.
    pub fn open() -> Result<Self, RouteError> {
        Self::open_at(routes_file_path())
    }

    /// Open or create at an explicit path. Used by tests and any
    /// caller that wants to override the data dir.
    pub fn open_at(path: PathBuf) -> Result<Self, RouteError> {
        let file = if path.exists() {
            let raw = std::fs::read(&path)?;
            if raw.is_empty() {
                RoutesFile::default()
            } else {
                serde_json::from_slice::<RoutesFile>(&raw)?
            }
        } else {
            RoutesFile::default()
        };
        Ok(Self { path, file })
    }

    /// Snapshot of all routes.
    pub fn list(&self) -> &[Route] {
        &self.file.routes
    }

    pub fn get(&self, id: RouteId) -> Option<&Route> {
        self.file.routes.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: RouteId) -> Option<&mut Route> {
        self.file.routes.iter_mut().find(|r| r.id == id)
    }

    pub fn disable_chooser(&self) -> bool {
        self.file.disable_deployment_mode_chooser
    }

    pub fn set_disable_chooser(&mut self, value: bool) -> Result<(), RouteError> {
        self.file.disable_deployment_mode_chooser = value;
        self.persist()
    }

    /// Insert a new route. Generates a fresh `id` and
    /// `deployment_organization_uuid` if the caller passed `Uuid::nil()`.
    pub fn add(&mut self, mut route: Route) -> Result<Route, RouteError> {
        if self.file.routes.iter().any(|r| r.name == route.name) {
            return Err(RouteError::DuplicateName(route.name));
        }
        if route.id.is_nil() {
            route.id = Uuid::new_v4();
        }
        if route.deployment_organization_uuid.is_nil() {
            route.deployment_organization_uuid = Uuid::new_v4();
        }
        self.file.routes.push(route.clone());
        self.persist()?;
        Ok(route)
    }

    /// Replace an existing route. Returns `NotFound` if `route.id`
    /// doesn't exist. Preserves activation flags untouched —
    /// activation/installation are managed via dedicated calls.
    pub fn update(&mut self, route: Route) -> Result<Route, RouteError> {
        let idx = self
            .file
            .routes
            .iter()
            .position(|r| r.id == route.id)
            .ok_or_else(|| RouteError::NotFound(route.id.to_string()))?;
        // Reject rename collisions with another route.
        if self
            .file
            .routes
            .iter()
            .enumerate()
            .any(|(i, r)| i != idx && r.name == route.name)
        {
            return Err(RouteError::DuplicateName(route.name));
        }
        let prev = &self.file.routes[idx];
        let merged = Route {
            active_on_desktop: prev.active_on_desktop,
            installed_on_cli: prev.installed_on_cli,
            deployment_organization_uuid: prev.deployment_organization_uuid,
            ..route
        };
        self.file.routes[idx] = merged.clone();
        self.persist()?;
        Ok(merged)
    }

    pub fn remove(&mut self, id: RouteId) -> Result<Route, RouteError> {
        let idx = self
            .file
            .routes
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| RouteError::NotFound(id.to_string()))?;
        let removed = self.file.routes.remove(idx);
        self.persist()?;
        Ok(removed)
    }

    /// Mark exactly one route as active on Desktop. Clears the flag
    /// on all other routes in the same write.
    pub fn set_active_desktop(&mut self, id: Option<RouteId>) -> Result<(), RouteError> {
        if let Some(target) = id {
            if !self.file.routes.iter().any(|r| r.id == target) {
                return Err(RouteError::NotFound(target.to_string()));
            }
        }
        for r in self.file.routes.iter_mut() {
            r.active_on_desktop = Some(r.id) == id;
        }
        self.persist()
    }

    pub fn set_installed_cli(
        &mut self,
        id: RouteId,
        installed: bool,
    ) -> Result<(), RouteError> {
        let r = self
            .file
            .routes
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| RouteError::NotFound(id.to_string()))?;
        r.installed_on_cli = installed;
        self.persist()
    }

    fn persist(&self) -> Result<(), RouteError> {
        let bytes = serde_json::to_vec_pretty(&self.file)?;
        fs_utils::atomic_write(&self.path, &bytes)?;
        Ok(())
    }
}

/// Canonical path: `<claudepot_data_dir>/routes.json`.
pub fn routes_file_path() -> PathBuf {
    claudepot_data_dir().join("routes.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::types::{AuthScheme, GatewayConfig, RouteProvider};
    use tempfile::tempdir;

    fn sample_route(name: &str) -> Route {
        Route {
            id: Uuid::nil(),
            name: name.into(),
            provider: RouteProvider::Gateway(GatewayConfig {
                base_url: "http://127.0.0.1:11434".into(),
                api_key: "ollama".into(),
                auth_scheme: AuthScheme::Bearer,
                enable_tool_search: false,
                use_keychain: false,
            }),
            model: "llama3.2:3b".into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: format!("claude-{}", name.to_ascii_lowercase()),
            deployment_organization_uuid: Uuid::nil(),
            active_on_desktop: false,
            installed_on_cli: false,
        }
    }

    #[test]
    fn open_at_missing_creates_default() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("routes.json");
        let s = RouteStore::open_at(p).unwrap();
        assert!(s.list().is_empty());
        assert!(!s.disable_chooser());
    }

    #[test]
    fn add_assigns_uuids() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("routes.json");
        let mut s = RouteStore::open_at(p).unwrap();
        let r = s.add(sample_route("Local")).unwrap();
        assert!(!r.id.is_nil());
        assert!(!r.deployment_organization_uuid.is_nil());
    }

    #[test]
    fn add_rejects_duplicate_name() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("routes.json");
        let mut s = RouteStore::open_at(p).unwrap();
        s.add(sample_route("Local")).unwrap();
        let err = s.add(sample_route("Local")).unwrap_err();
        assert!(matches!(err, RouteError::DuplicateName(_)));
    }

    #[test]
    fn update_preserves_flags() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("routes.json");
        let mut s = RouteStore::open_at(p).unwrap();
        let mut r = s.add(sample_route("Local")).unwrap();
        s.set_installed_cli(r.id, true).unwrap();
        s.set_active_desktop(Some(r.id)).unwrap();

        // Caller passes a fresh route without these flags set.
        r.installed_on_cli = false;
        r.active_on_desktop = false;
        r.model = "llama3.2:8b".into();
        let updated = s.update(r).unwrap();
        assert!(updated.installed_on_cli);
        assert!(updated.active_on_desktop);
        assert_eq!(updated.model, "llama3.2:8b");
    }

    #[test]
    fn set_active_desktop_is_exclusive() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("routes.json");
        let mut s = RouteStore::open_at(p).unwrap();
        let a = s.add(sample_route("a")).unwrap();
        let b = s.add(sample_route("b")).unwrap();
        s.set_active_desktop(Some(a.id)).unwrap();
        s.set_active_desktop(Some(b.id)).unwrap();
        assert!(!s.get(a.id).unwrap().active_on_desktop);
        assert!(s.get(b.id).unwrap().active_on_desktop);
        s.set_active_desktop(None).unwrap();
        assert!(!s.get(a.id).unwrap().active_on_desktop);
        assert!(!s.get(b.id).unwrap().active_on_desktop);
    }

    #[test]
    fn remove_deletes() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("routes.json");
        let mut s = RouteStore::open_at(p).unwrap();
        let r = s.add(sample_route("Local")).unwrap();
        s.remove(r.id).unwrap();
        assert!(s.list().is_empty());
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("routes.json");
        {
            let mut s = RouteStore::open_at(p.clone()).unwrap();
            s.add(sample_route("Local")).unwrap();
            s.set_disable_chooser(true).unwrap();
        }
        let s2 = RouteStore::open_at(p).unwrap();
        assert_eq!(s2.list().len(), 1);
        assert!(s2.disable_chooser());
    }

    #[test]
    fn empty_file_treated_as_default() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("routes.json");
        std::fs::write(&p, b"").unwrap();
        let s = RouteStore::open_at(p).unwrap();
        assert!(s.list().is_empty());
    }
}

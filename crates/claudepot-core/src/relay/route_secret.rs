//! Where a published route's secret comes from when a relay serves it
//! (plan §5.5).
//!
//! A gateway route keeps its key either in `routes.json` (plaintext) or in
//! the OS keychain (`use_keychain`). Both binaries read `routes.json`. For
//! the keychain, `routes::keychain` writes through `/usr/bin/security` on
//! macOS (surface 2 in `rules/architecture.md`), so a key the GUI stored is
//! readable by a headless `claudepot relay serve` and by the wrapper's
//! helper script alike, and every edit or rotation lands in the one place
//! all three read.
//!
//! The plan's §5.5 assumed a migration from per-binary `keyring` items.
//! There was nothing to migrate: with no `keyring` backend feature enabled
//! the crate used its in-memory mock store, so no such item ever reached
//! the keychain (see `routes::keychain`). Linux and Windows keep that
//! mock until a backend is chosen, so a keychain route there is
//! `Missing` here — reported, never served with an empty key.
//!
//! **A locked or unreadable keychain is an error, never a silent skip and
//! never an empty key.** The relay renders every [`RouteSecretError`] as
//! its fixed `credential_unavailable` refusal.
//!
//! Only gateway routes are relay-capable — the relay forwards Anthropic
//! bytes — so the one field this module reads is the gateway API key.
//!
//! **A serving relay reads the key as it is now, for the target it was
//! approved for.** [`RouteKey`] re-reads `routes.json` on every request,
//! so a plaintext key rotated in place reaches the next request exactly
//! as a keychain rotation does. Everything else that decides the target —
//! origin, models, where the key is kept — stays the approved snapshot's:
//! a route that was re-targeted, moved or removed since the relay started
//! is [`RouteSecretError::Changed`], never its new key sent to the old
//! origin.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::secret::Secret;
use crate::routes::{Route, RouteId, RouteProvider, RouteStore, SecretField};

/// The one relay-capable secret field.
const FIELD: SecretField = SecretField::GatewayApiKey;

/// Why a published route has no usable secret. The reason is for the
/// operator's log and the CLI, and never carries the secret.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouteSecretError {
    #[error("route {0} is not a gateway route; only gateway routes can be published")]
    NotRelayCapable(RouteId),
    #[error("the keychain is locked — unlock it and retry")]
    Locked,
    #[error("route {0} keeps its key in the keychain, and no key was found there")]
    Missing(RouteId),
    #[error("keychain: {0}")]
    Backend(String),
    /// The route in `routes.json` no longer matches the target the relay
    /// serves: removed, pointed elsewhere, or its key moved.
    #[error("route {0} was removed, re-targeted or had its key moved since the relay started; restart the relay")]
    Changed(RouteId),
    /// `routes.json` could not be read. The parse error is not kept: the
    /// file holds plaintext keys.
    #[error("routes.json could not be read")]
    RoutesFile,
}

/// Reads one route's keychain cell. A seam so the serving contract is
/// testable without a keychain.
pub trait KeychainReader {
    fn read(&self, route: RouteId) -> Result<Option<Secret>, RouteSecretError>;
}

/// The production reader: the cell `routes::keychain` writes.
pub struct OsKeychain;

impl KeychainReader for OsKeychain {
    #[cfg(target_os = "macos")]
    fn read(&self, route: RouteId) -> Result<Option<Secret>, RouteSecretError> {
        use crate::security_cli::SecurityCliError as E;
        let account = crate::routes::keychain_account(route, FIELD);
        // Called directly rather than through `routes::read_keychain_secret`
        // so a locked keychain stays distinguishable from a backend error.
        crate::security_cli::read(crate::routes::KEYCHAIN_SERVICE, &account).map_err(|e| match e {
            E::Locked => RouteSecretError::Locked,
            other => RouteSecretError::Backend(other.to_string()),
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn read(&self, route: RouteId) -> Result<Option<Secret>, RouteSecretError> {
        crate::routes::read_keychain_secret(route, FIELD)
            .map(|v| v.map(Secret::new))
            .map_err(|e| RouteSecretError::Backend(e.to_string()))
    }
}

/// Check, before a route is published, that a server could read its key
/// now. Refuse the publish on `Err`: publishing a route whose key is
/// unreadable would turn every request into `credential_unavailable`.
pub async fn check_for_publish(route: &Route) -> Result<(), RouteSecretError> {
    read_for_serving(route).await.map(|_| ())
}

/// The key a server presents upstream for a published route. `None` for
/// a plaintext route with an empty key (local servers that accept any
/// token); `Err` whenever a keychain route's key cannot be read.
pub async fn read_for_serving(route: &Route) -> Result<Option<Secret>, RouteSecretError> {
    let route = route.clone();
    tokio::task::spawn_blocking(move || read_with(&route, &OsKeychain))
        .await
        .map_err(|e| RouteSecretError::Backend(format!("keychain task: {e}")))?
}

/// [`read_for_serving`] over an injected reader.
pub fn read_with<K: KeychainReader + ?Sized>(
    route: &Route,
    keychain: &K,
) -> Result<Option<Secret>, RouteSecretError> {
    let RouteProvider::Gateway(cfg) = &route.provider else {
        return Err(RouteSecretError::NotRelayCapable(route.id));
    };
    if !cfg.use_keychain {
        return Ok((!cfg.api_key.is_empty()).then(|| Secret::new(cfg.api_key.clone())));
    }
    match keychain.read(route.id)? {
        Some(s) if !s.is_empty() => Ok(Some(s)),
        _ => Err(RouteSecretError::Missing(route.id)),
    }
}

/// The key `approved` presents now. With `routes_file`, the route is
/// re-read from it by id and must still describe the approved target
/// (the key-free [`super::passthrough::route_target_spec`]); its key is
/// then read wherever that route keeps it. Without `routes_file`, the key
/// is read from `approved` itself.
pub fn read_current<K: KeychainReader + ?Sized>(
    approved: &Route,
    routes_file: Option<&Path>,
    keychain: &K,
) -> Result<Option<Secret>, RouteSecretError> {
    let Some(path) = routes_file else {
        return read_with(approved, keychain);
    };
    let approved_spec = super::passthrough::route_target_spec(approved)?.digest();
    let store = RouteStore::open_at(path.to_owned()).map_err(|_| RouteSecretError::RoutesFile)?;
    let now = store
        .get(approved.id)
        .ok_or(RouteSecretError::Changed(approved.id))?;
    match super::passthrough::route_target_spec(now) {
        Ok(spec) if spec.digest() == approved_spec => read_with(now, keychain),
        _ => Err(RouteSecretError::Changed(approved.id)),
    }
}

/// One published route's key source, shared by the pass-through and the
/// translated handler so both follow the same rotation contract.
#[derive(Clone)]
pub struct RouteKey {
    approved: Route,
    keychain: Arc<dyn KeychainReader + Send + Sync>,
    routes_file: Option<PathBuf>,
}

impl RouteKey {
    /// Reads from `approved` itself: a plaintext key is the snapshot's.
    pub fn new(approved: Route, keychain: Arc<dyn KeychainReader + Send + Sync>) -> Self {
        Self {
            approved,
            keychain,
            routes_file: None,
        }
    }

    /// Re-read the route from `path` on every read (see [`read_current`]).
    pub fn with_routes_file(mut self, path: PathBuf) -> Self {
        self.routes_file = Some(path);
        self
    }

    /// The production source: the OS keychain and `routes_file`, the
    /// `routes.json` of the data dir the relay serves from. The caller
    /// names it; this never guesses a default data dir.
    pub fn production(approved: Route, routes_file: PathBuf) -> Self {
        Self::new(approved, Arc::new(OsKeychain)).with_routes_file(routes_file)
    }

    pub fn route_id(&self) -> RouteId {
        self.approved.id
    }

    /// The key now. Blocking I/O (`routes.json`, `security`) runs off the
    /// async runtime.
    pub async fn read(&self) -> Result<Option<Secret>, RouteSecretError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            read_current(
                &this.approved,
                this.routes_file.as_deref(),
                this.keychain.as_ref(),
            )
        })
        .await
        .map_err(|e| RouteSecretError::Backend(format!("keychain task: {e}")))?
    }
}

#[cfg(test)]
#[path = "tests_route_secret.rs"]
mod tests;

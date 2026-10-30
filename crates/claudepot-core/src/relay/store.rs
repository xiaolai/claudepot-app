//! The relay server's two JSON stores: `relay-config.json` (what is
//! published, under which generation) and `relay-devices.json` (who may
//! call, and what each accepted).
//!
//! Both load through [`json_store::load_or_recover`] and **report** a
//! reset, for the reason `remote::store` gives: a silently emptied
//! device file erases the record of what was revoked, and a silently
//! emptied config restarts every generation at 1 — so a grant accepted
//! against an old target would match a new target that reuses the
//! number. A reset therefore also rotates the config's `epoch`, which
//! every grant carries; a grant from an earlier epoch never matches.

use std::collections::HashSet;
use std::path::Path;

use super::devices::DeviceRecord;
use super::target::UpstreamId;
use crate::json_store::{self, CorruptionRecovery, Loaded, SaveError};

pub const CONFIG_FILENAME: &str = "relay-config.json";
pub const DEVICES_FILENAME: &str = "relay-devices.json";
pub const SCHEMA_VERSION: u32 = 1;
/// The spike's port; a client given only a host tries this.
pub const DEFAULT_PORT: u16 = 8787;

const CONFIG_STORE: &str = "relay_config_store";
const DEVICES_STORE: &str = "relay_devices_store";

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("relay store is schema version {found}, expected {expected}")]
    UnknownSchema { expected: u32, found: u32 },
    #[error("the relay config has no server id or epoch")]
    MissingIdentity,
    #[error("upstream {0} is published twice")]
    DuplicateUpstream(UpstreamId),
    #[error("upstream {0} has generation 0")]
    ZeroGeneration(UpstreamId),
    #[error("upstream {0} names both an account and a route")]
    TwoSources(UpstreamId),
    #[error("upstream {0} translates, but names no route")]
    TranslateWithoutRoute(UpstreamId),
    #[error("two relay devices share an id or a token hash")]
    DuplicateDevice,
    #[error("a relay device has an empty token hash")]
    EmptyTokenHash,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("relay store {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("relay store {path} would be invalid: {error}")]
    Invalid { path: String, error: String },
    /// Read while a relay serves, so it was left in place: the serving
    /// process holds the live copy and its next write replaces the file.
    #[error(
        "relay store {path} is unreadable ({error}); a relay is serving, so the \
         file was left in place — stop the relay to reset it"
    )]
    Unreadable { path: String, error: String },
}

pub(crate) fn save_error<E: std::fmt::Display>(path: &Path, e: SaveError<E>) -> StoreError {
    let path = path.display().to_string();
    match e {
        SaveError::Io(source) => StoreError::Io { path, source },
        SaveError::Validation(e) => StoreError::Invalid {
            path,
            error: e.to_string(),
        },
        SaveError::Serde(e) => StoreError::Invalid {
            path,
            error: e.to_string(),
        },
    }
}

/// One upstream this server publishes, or once published.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Publication {
    pub id: UpstreamId,
    /// [`super::identity::TargetSpec::digest`] of the current target.
    pub digest: String,
    /// Bumped whenever `digest` changes. Never reset: an unpublished
    /// upstream keeps its row (`published: false`), so republishing a
    /// different target under the same id cannot reuse a generation an
    /// old grant names.
    pub generation: u64,
    pub published: bool,
    /// The local account a subscription upstream renews through.
    #[serde(default)]
    pub account_id: Option<uuid::Uuid>,
    /// The route (`routes.json`) a pass-through upstream serves. At most
    /// one of `account_id` and `route_id` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_id: Option<uuid::Uuid>,
    /// Serve the route through this protocol translation instead of
    /// passing it through. Only meaningful with `route_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translate: Option<super::translate::Translation>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConfigFile {
    pub schema_version: u32,
    /// Stable per installation; health reports it so a client can tell
    /// two servers apart.
    pub server_id: String,
    /// Rotated when the file is reset; grants carry it.
    pub epoch: String,
    /// Bind address; `None` means loopback.
    #[serde(default)]
    pub bind: Option<std::net::IpAddr>,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Extra names (tailnet DNS name, LAN name) the leaf must carry.
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub publications: Vec<Publication>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            server_id: uuid::Uuid::new_v4().to_string(),
            epoch: uuid::Uuid::new_v4().to_string(),
            bind: None,
            port: DEFAULT_PORT,
            names: Vec::new(),
            publications: Vec::new(),
        }
    }
}

impl json_store::Validate for ConfigFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnknownSchema {
                expected: SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if self.server_id.is_empty() || self.epoch.is_empty() {
            return Err(ValidationError::MissingIdentity);
        }
        let mut seen = HashSet::new();
        for p in &self.publications {
            if !seen.insert(&p.id) {
                return Err(ValidationError::DuplicateUpstream(p.id.clone()));
            }
            if p.generation == 0 {
                return Err(ValidationError::ZeroGeneration(p.id.clone()));
            }
            if p.account_id.is_some() && p.route_id.is_some() {
                return Err(ValidationError::TwoSources(p.id.clone()));
            }
            if p.translate.is_some() && p.route_id.is_none() {
                return Err(ValidationError::TranslateWithoutRoute(p.id.clone()));
            }
        }
        Ok(())
    }
}

impl ConfigFile {
    /// Publish `id` with the target `digest`. A new id starts at
    /// generation 1; a changed digest bumps the generation; the same
    /// digest keeps it. Returns the current generation.
    pub fn publish(
        &mut self,
        id: &UpstreamId,
        digest: &str,
        account_id: Option<uuid::Uuid>,
    ) -> u64 {
        self.publish_source(id, digest, account_id, None, None)
    }

    /// [`Self::publish`] for a route upstream, passed through or
    /// translated. Replaces any account the id served; the digest (which
    /// covers the translation) decides whether the generation moves.
    pub fn publish_route(
        &mut self,
        id: &UpstreamId,
        digest: &str,
        route_id: uuid::Uuid,
        translate: Option<super::translate::Translation>,
    ) -> u64 {
        self.publish_source(id, digest, None, Some(route_id), translate)
    }

    fn publish_source(
        &mut self,
        id: &UpstreamId,
        digest: &str,
        account_id: Option<uuid::Uuid>,
        route_id: Option<uuid::Uuid>,
        translate: Option<super::translate::Translation>,
    ) -> u64 {
        if let Some(p) = self.publications.iter_mut().find(|p| &p.id == id) {
            if p.digest != digest {
                p.digest = digest.to_owned();
                p.generation += 1;
            }
            p.published = true;
            p.account_id = account_id;
            p.route_id = route_id;
            p.translate = translate;
            return p.generation;
        }
        self.publications.push(Publication {
            id: id.clone(),
            digest: digest.to_owned(),
            generation: 1,
            published: true,
            account_id,
            route_id,
            translate,
        });
        1
    }

    /// Stop publishing `id`. The row stays so its generation is never
    /// reused.
    pub fn unpublish(&mut self, id: &UpstreamId) -> bool {
        match self.publications.iter_mut().find(|p| &p.id == id) {
            Some(p) if p.published => {
                p.published = false;
                true
            }
            _ => false,
        }
    }

    pub fn published(&self) -> impl Iterator<Item = &Publication> {
        self.publications.iter().filter(|p| p.published)
    }

    pub fn publication(&self, id: &UpstreamId) -> Option<&Publication> {
        self.published().find(|p| &p.id == id)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DevicesFile {
    pub schema_version: u32,
    #[serde(default)]
    pub devices: Vec<DeviceRecord>,
}

impl Default for DevicesFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            devices: Vec::new(),
        }
    }
}

impl json_store::Validate for DevicesFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnknownSchema {
                expected: SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        let mut ids = HashSet::new();
        let mut hashes = HashSet::new();
        for d in &self.devices {
            if d.token_hash.is_empty() {
                return Err(ValidationError::EmptyTokenHash);
            }
            if !ids.insert(d.id.as_str()) || !hashes.insert(d.token_hash.as_str()) {
                return Err(ValidationError::DuplicateDevice);
            }
        }
        Ok(())
    }
}

/// Load `relay-config.json`. A reset is logged at `error` and returned,
/// and the default it starts from has a fresh epoch.
pub fn load_config(path: &Path) -> Result<Loaded<ConfigFile>, StoreError> {
    let loaded =
        json_store::load_or_recover::<ConfigFile>(path, CONFIG_STORE).map_err(|source| {
            StoreError::Io {
                path: path.display().to_string(),
                source,
            }
        })?;
    report(path, CONFIG_STORE, &loaded.recovery, "relay configuration was unreadable and has been moved aside; every paired device must accept its upstreams again");
    Ok(loaded)
}

pub fn save_config(path: &Path, file: &ConfigFile) -> Result<(), StoreError> {
    json_store::save(path, file).map_err(|e| save_error(path, e))
}

/// Load `relay-devices.json`, reporting a reset.
pub fn load_devices(path: &Path) -> Result<Loaded<DevicesFile>, StoreError> {
    let loaded =
        json_store::load_or_recover::<DevicesFile>(path, DEVICES_STORE).map_err(|source| {
            StoreError::Io {
                path: path.display().to_string(),
                source,
            }
        })?;
    report(path, DEVICES_STORE, &loaded.recovery, "relay device records were unreadable and have been moved aside; every device must pair again, and any revocation recorded there is gone");
    Ok(loaded)
}

pub fn save_devices(path: &Path, file: &DevicesFile) -> Result<(), StoreError> {
    json_store::save(path, file).map_err(|e| save_error(path, e))
}

/// Read `relay-config.json` **without** recovering: a corrupt file is
/// [`StoreError::Unreadable`] and stays where it is. For a reader that
/// does not own the store — a status read while a relay serves, whose
/// live copy the file belongs to.
pub fn peek_config(path: &Path) -> Result<ConfigFile, StoreError> {
    peek(path)
}

/// [`peek_config`] for `relay-devices.json`.
pub fn peek_devices(path: &Path) -> Result<DevicesFile, StoreError> {
    peek(path)
}

/// The non-mutating half of [`json_store::load_or_recover`]'s contract:
/// missing is the default, and anything that would be moved aside there
/// is an error here.
fn peek<T>(path: &Path) -> Result<T, StoreError>
where
    T: serde::de::DeserializeOwned + Default + json_store::Validate,
{
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(source) => {
            return Err(StoreError::Io {
                path: path.display().to_string(),
                source,
            })
        }
    };
    let unreadable = |error: String| StoreError::Unreadable {
        path: path.display().to_string(),
        error,
    };
    let value: T = serde_json::from_slice(&bytes).map_err(|e| unreadable(e.to_string()))?;
    value.validate().map_err(|e| unreadable(e.to_string()))?;
    Ok(value)
}

fn report(path: &Path, store: &'static str, recovery: &Option<CorruptionRecovery>, what: &str) {
    if recovery.is_some() {
        tracing::error!(store, path = %path.display(), "{what}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> UpstreamId {
        UpstreamId::parse(s).unwrap()
    }

    #[test]
    fn a_changed_target_bumps_the_generation_and_the_same_one_does_not() {
        let mut c = ConfigFile::default();
        assert_eq!(c.publish(&id("sub"), "d1", None), 1);
        assert_eq!(c.publish(&id("sub"), "d1", None), 1);
        assert_eq!(c.publish(&id("sub"), "d2", None), 2);
    }

    #[test]
    fn a_route_publication_replaces_the_account_and_both_together_are_invalid() {
        let mut c = ConfigFile::default();
        let (acct, route) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        c.publish(&id("ds"), "d1", Some(acct));
        assert_eq!(c.publish_route(&id("ds"), "d2", route, None), 2);
        let p = c.publication(&id("ds")).unwrap();
        assert_eq!((p.account_id, p.route_id), (None, Some(route)));
        assert!(json_store::Validate::validate(&c).is_ok());

        c.publications[0].account_id = Some(acct);
        assert!(matches!(
            json_store::Validate::validate(&c),
            Err(ValidationError::TwoSources(_))
        ));
    }

    #[test]
    fn a_translation_needs_a_route_and_switching_it_off_clears_it() {
        use crate::relay::translate::Translation;
        let mut c = ConfigFile::default();
        let route = uuid::Uuid::new_v4();
        c.publish_route(&id("gm"), "d1", route, Some(Translation::Gemini));
        assert_eq!(
            c.publication(&id("gm")).unwrap().translate,
            Some(Translation::Gemini)
        );
        assert!(json_store::Validate::validate(&c).is_ok());
        c.publish_route(&id("gm"), "d2", route, None);
        assert_eq!(c.publication(&id("gm")).unwrap().translate, None);

        c.publications[0].translate = Some(Translation::Codex);
        c.publications[0].route_id = None;
        assert!(matches!(
            json_store::Validate::validate(&c),
            Err(ValidationError::TranslateWithoutRoute(_))
        ));
    }

    #[test]
    fn republishing_after_unpublish_never_reuses_a_generation() {
        let mut c = ConfigFile::default();
        c.publish(&id("ds"), "d1", None);
        assert!(c.unpublish(&id("ds")));
        assert!(c.publication(&id("ds")).is_none());
        assert_eq!(c.publish(&id("ds"), "d2", None), 2);
    }

    #[test]
    fn config_round_trips_and_a_corrupt_file_is_reported_with_a_new_epoch() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(CONFIG_FILENAME);
        let mut c = ConfigFile::default();
        c.publish(&id("sub"), "d1", None);
        save_config(&p, &c).unwrap();
        let back = load_config(&p).unwrap();
        assert!(back.recovery.is_none());
        assert_eq!(back.value, c);

        std::fs::write(&p, b"{not json").unwrap();
        let reset = load_config(&p).unwrap();
        assert!(reset.recovery.is_some(), "a reset must be reported");
        assert_ne!(reset.value.epoch, c.epoch);
        assert!(reset.value.publications.is_empty());
    }

    #[test]
    fn a_corrupt_device_file_is_reported() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(DEVICES_FILENAME);
        std::fs::write(&p, br#"{"schema_version":9,"devices":[]}"#).unwrap();
        let loaded = load_devices(&p).unwrap();
        assert!(loaded.recovery.is_some());
        assert!(loaded.value.devices.is_empty());
    }

    #[test]
    fn a_peek_reads_a_good_file_and_leaves_a_corrupt_one_in_place() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(CONFIG_FILENAME);
        assert_eq!(peek_config(&p).unwrap().publications, vec![]);
        let mut c = ConfigFile::default();
        c.publish(&id("sub"), "d1", None);
        save_config(&p, &c).unwrap();
        assert_eq!(peek_config(&p).unwrap(), c);

        let devices = d.path().join(DEVICES_FILENAME);
        for (file, bytes) in [
            (&p, &b"{not json"[..]),
            (&devices, &br#"{"schema_version":9,"devices":[]}"#[..]),
        ] {
            std::fs::write(file, bytes).unwrap();
            let err = if file == &p {
                peek_config(file).map(|_| ()).unwrap_err()
            } else {
                peek_devices(file).map(|_| ()).unwrap_err()
            };
            assert!(matches!(err, StoreError::Unreadable { .. }), "{err}");
            assert_eq!(std::fs::read(file).unwrap(), bytes);
            assert!(json_store::corrupt_siblings(file).is_empty());
        }
    }

    #[test]
    fn a_missing_file_is_empty_without_a_report() {
        let d = tempfile::tempdir().unwrap();
        assert!(load_devices(&d.path().join(DEVICES_FILENAME))
            .unwrap()
            .recovery
            .is_none());
        assert!(load_config(&d.path().join(CONFIG_FILENAME))
            .unwrap()
            .recovery
            .is_none());
    }
}

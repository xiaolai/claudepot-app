//! The client's store: `relay-servers.json`, the relays this machine has
//! paired with, and what it approved on each.
//!
//! The device token is **not** here. It lives in the OS credential store
//! ([`super::token_store`]), keyed by the server id; this file holds only
//! what a token is useless without — the pinned CA, the approved target
//! fingerprints, and the client salt every fingerprint is computed under.
//!
//! A reset is **reported** (`load_or_recover`): it loses the client salt,
//! so every fingerprint this client approved stops matching, and it
//! orphans the tokens in the credential store. Silently starting empty
//! would look like "never paired" while those tokens still work.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::identity::parse_salt;
use super::store::{save_error, StoreError};
use super::target::UpstreamId;
use crate::json_store::{self, Loaded};

pub const SERVERS_FILENAME: &str = "relay-servers.json";
pub const SCHEMA_VERSION: u32 = 1;
const SERVERS_STORE: &str = "relay_servers_store";

pub fn servers_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(SERVERS_FILENAME)
}

#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("relay servers store is schema version {found}, expected {expected}")]
    UnknownSchema { expected: u32, found: u32 },
    #[error("the relay client salt is missing or malformed")]
    BadSalt,
    #[error("two paired relays share the id or the name {0}")]
    Duplicate(String),
    #[error("relay {0} has an unusable name")]
    BadName(String),
    #[error("relay {0} has an unusable base URL")]
    BadUrl(String),
    #[error("relay {0} is not on loopback and has no pinned CA")]
    Unpinned(String),
    #[error("relay {0} approves upstream {1} twice")]
    DuplicateUpstream(String, UpstreamId),
}

/// One upstream this client approved on one server: the logical target
/// is `(id, fingerprint)` (plan §6). A server whose live fingerprint for
/// `id` differs is not a candidate until `relay accept` records it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApprovedUpstream {
    pub id: UpstreamId,
    /// The target fingerprint under this client's salt.
    pub fingerprint: String,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServerRecord {
    /// The server's own id, as `GET /relay/ca` and health report it.
    pub server_id: String,
    /// The user's name for it; unique on this client. Defaults to the
    /// host it was paired by.
    pub name: String,
    /// `https://host:port`, or `http://` for a validated loopback host.
    pub base_url: String,
    /// The pinned CA. `None` only for a loopback `http://` server.
    pub ca_pem: Option<String>,
    pub ca_fingerprint: Option<String>,
    pub device_id: String,
    pub default_upstream: Option<UpstreamId>,
    pub upstreams: Vec<ApprovedUpstream>,
    pub paired_at: DateTime<Utc>,
}

impl ServerRecord {
    pub fn approved(&self, id: &UpstreamId) -> Option<&ApprovedUpstream> {
        self.upstreams.iter().find(|u| &u.id == id)
    }

    /// Record `id` as approved at `fingerprint` / `generation`.
    pub fn approve(&mut self, id: UpstreamId, fingerprint: String, generation: u64) {
        match self.upstreams.iter_mut().find(|u| u.id == id) {
            Some(u) => {
                u.fingerprint = fingerprint;
                u.generation = generation;
            }
            None => self.upstreams.push(ApprovedUpstream {
                id,
                fingerprint,
                generation,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServersFile {
    pub schema_version: u32,
    /// Hex, 32 bytes. Sent with every pairing and probe; every
    /// fingerprint this client stores is computed under it.
    pub client_salt: String,
    /// In the user's pinned order: ties in scoring go to the earlier one.
    pub servers: Vec<ServerRecord>,
}

impl Default for ServersFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            client_salt: super::client::new_client_salt(),
            servers: Vec::new(),
        }
    }
}

impl ServersFile {
    /// By server id, then by name.
    pub fn find(&self, key: &str) -> Option<&ServerRecord> {
        self.servers
            .iter()
            .find(|s| s.server_id == key)
            .or_else(|| self.servers.iter().find(|s| s.name == key))
    }

    pub fn find_mut(&mut self, key: &str) -> Option<&mut ServerRecord> {
        let i = self
            .servers
            .iter()
            .position(|s| s.server_id == key)
            .or_else(|| self.servers.iter().position(|s| s.name == key))?;
        self.servers.get_mut(i)
    }

    /// Insert or replace by server id. A re-pair keeps the record's place
    /// in the pinned order.
    pub fn upsert(&mut self, record: ServerRecord) {
        match self
            .servers
            .iter_mut()
            .find(|s| s.server_id == record.server_id)
        {
            Some(slot) => *slot = record,
            None => self.servers.push(record),
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<ServerRecord> {
        let i = self
            .servers
            .iter()
            .position(|s| s.server_id == key)
            .or_else(|| self.servers.iter().position(|s| s.name == key))?;
        Some(self.servers.remove(i))
    }
}

/// Is `url` a plain-HTTP URL whose host is loopback? The only case that
/// may go unpinned.
pub fn is_loopback_http(url: &reqwest::Url) -> bool {
    url.scheme() == "http"
        && match url.host_str() {
            Some("localhost") => true,
            Some(h) => h
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()),
            None => false,
        }
}

fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

impl json_store::Validate for ServersFile {
    type Error = ValidationError;

    fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError::UnknownSchema {
                expected: SCHEMA_VERSION,
                found: self.schema_version,
            });
        }
        if parse_salt(&self.client_salt).is_none() {
            return Err(ValidationError::BadSalt);
        }
        let mut seen = std::collections::HashSet::new();
        for s in &self.servers {
            if !seen.insert(s.server_id.as_str()) {
                return Err(ValidationError::Duplicate(s.server_id.clone()));
            }
            if !seen.insert(s.name.as_str()) && s.name != s.server_id {
                return Err(ValidationError::Duplicate(s.name.clone()));
            }
            if !name_ok(&s.name) || s.server_id.is_empty() {
                return Err(ValidationError::BadName(s.name.clone()));
            }
            let url = reqwest::Url::parse(&s.base_url)
                .map_err(|_| ValidationError::BadUrl(s.name.clone()))?;
            if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
                return Err(ValidationError::BadUrl(s.name.clone()));
            }
            if url.scheme() == "http" && !is_loopback_http(&url) {
                return Err(ValidationError::Unpinned(s.name.clone()));
            }
            if url.scheme() == "https" && s.ca_pem.is_none() {
                return Err(ValidationError::Unpinned(s.name.clone()));
            }
            let mut ups = std::collections::HashSet::new();
            for u in &s.upstreams {
                if !ups.insert(&u.id) {
                    return Err(ValidationError::DuplicateUpstream(
                        s.name.clone(),
                        u.id.clone(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Load `relay-servers.json`, reporting a reset.
pub fn load(path: &Path) -> Result<Loaded<ServersFile>, StoreError> {
    let loaded =
        json_store::load_or_recover::<ServersFile>(path, SERVERS_STORE).map_err(|source| {
            StoreError::Io {
                path: path.display().to_string(),
                source,
            }
        })?;
    if loaded.recovery.is_some() {
        tracing::error!(
            store = SERVERS_STORE,
            path = %path.display(),
            "paired relay records were unreadable and have been moved aside; pair each relay again (device tokens already in the credential store are orphaned — revoke them on the server)"
        );
    }
    Ok(loaded)
}

pub fn save(path: &Path, file: &ServersFile) -> Result<(), StoreError> {
    json_store::save(path, file).map_err(|e| save_error(path, e))
}

/// Check `file` the way [`save`] would, without writing it — so a caller
/// can refuse a change before it touches anything outside this file.
pub fn check(path: &Path, file: &ServersFile) -> Result<(), StoreError> {
    use json_store::Validate as _;
    file.validate().map_err(|e| StoreError::Invalid {
        path: path.display().to_string(),
        error: e.to_string(),
    })
}

/// An exclusive advisory lock on `<path>.lock`, held until drop. Every
/// read-modify-write of `relay-servers.json` holds it, so the CLI and the
/// GUI pairing or accepting at the same moment cannot overwrite each
/// other's records. Acquiring it blocks: an async caller acquires it off
/// the runtime (`spawn_blocking`), because a holder may await inside it.
pub struct ServersLock {
    path: PathBuf,
    _lock: std::fs::File,
}

impl ServersLock {
    pub fn acquire(path: &Path) -> Result<Self, StoreError> {
        let io = |source| StoreError::Io {
            path: path.display().to_string(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let mut lock_name = path.file_name().unwrap_or_default().to_os_string();
        lock_name.push(".lock");
        let lock = std::fs::File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_file_name(lock_name))
            .map_err(io)?;
        lock.lock().map_err(io)?;
        Ok(Self {
            path: path.to_owned(),
            _lock: lock,
        })
    }

    pub fn load(&self) -> Result<ServersFile, StoreError> {
        Ok(load(&self.path)?.value)
    }

    pub fn save(&self, file: &ServersFile) -> Result<(), StoreError> {
        save(&self.path, file)
    }
}

/// Load, change and save `relay-servers.json` under [`ServersLock`]. The
/// lock spans only the read-modify-write — never a network round trip —
/// so callers do their I/O first and pass the result in.
pub fn mutate<T>(path: &Path, f: impl FnOnce(&mut ServersFile) -> T) -> Result<T, StoreError> {
    let lock = ServersLock::acquire(path)?;
    let mut file = lock.load()?;
    let out = f(&mut file);
    lock.save(&file)?;
    Ok(out)
}

/// This client's salt, written to disk first if the file has none yet —
/// a token is bound to the salt it was paired with, so the salt must be
/// the persisted one, not a fresh default two concurrent pairs disagree on.
pub fn persisted_salt(path: &Path) -> Result<String, StoreError> {
    mutate(path, |f| f.client_salt.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, name: &str) -> ServerRecord {
        ServerRecord {
            server_id: id.into(),
            name: name.into(),
            base_url: "https://relay.test:8787".into(),
            ca_pem: Some("-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----\n".into()),
            ca_fingerprint: Some("ab".into()),
            device_id: "dev-1".into(),
            default_upstream: UpstreamId::parse("sub"),
            upstreams: vec![ApprovedUpstream {
                id: UpstreamId::parse("sub").unwrap(),
                fingerprint: "fp".into(),
                generation: 1,
            }],
            paired_at: Utc::now(),
        }
    }

    #[test]
    fn concurrent_mutations_keep_every_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SERVERS_FILENAME);
        let salt = persisted_salt(&path).unwrap();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    mutate(&path, |f| {
                        f.upsert(record(&format!("s{i}"), &format!("n{i}")))
                    })
                    .unwrap()
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let back = load(&path).unwrap().value;
        assert_eq!(back.servers.len(), 8);
        assert_eq!(
            back.client_salt, salt,
            "the salt a token was paired with must not move"
        );
        assert_eq!(persisted_salt(&path).unwrap(), salt);
    }

    #[test]
    fn a_round_trip_keeps_the_salt_and_the_pinned_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SERVERS_FILENAME);
        let mut f = ServersFile::default();
        f.upsert(record("s1", "alpha"));
        f.upsert(record("s2", "beta"));
        save(&path, &f).unwrap();
        let back = load(&path).unwrap();
        assert!(back.recovery.is_none());
        assert_eq!(back.value, f);
        assert_eq!(back.value.servers[1].name, "beta");
    }

    #[test]
    fn a_repair_keeps_its_place_in_the_pinned_order() {
        let mut f = ServersFile::default();
        f.upsert(record("s1", "alpha"));
        f.upsert(record("s2", "beta"));
        let mut again = record("s1", "alpha");
        again.device_id = "dev-2".into();
        f.upsert(again);
        assert_eq!(f.servers[0].device_id, "dev-2");
        assert_eq!(f.servers.len(), 2);
    }

    #[test]
    fn a_corrupt_file_is_reported_and_starts_with_a_fresh_salt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SERVERS_FILENAME);
        std::fs::write(&path, b"{not json").unwrap();
        let loaded = load(&path).unwrap();
        assert!(loaded.recovery.is_some(), "a reset must be reported");
        assert!(parse_salt(&loaded.value.client_salt).is_some());
    }

    #[test]
    fn a_remote_server_without_a_pinned_ca_is_refused() {
        use json_store::Validate;
        let mut f = ServersFile::default();
        let mut r = record("s1", "alpha");
        r.ca_pem = None;
        f.upsert(r);
        assert!(matches!(f.validate(), Err(ValidationError::Unpinned(_))));

        let mut f = ServersFile::default();
        let mut r = record("s1", "alpha");
        r.base_url = "http://relay.test:8787".into();
        f.upsert(r);
        assert!(matches!(f.validate(), Err(ValidationError::Unpinned(_))));

        let mut f = ServersFile::default();
        let mut r = record("s1", "alpha");
        r.base_url = "http://127.0.0.1:8787".into();
        r.ca_pem = None;
        f.upsert(r);
        assert!(f.validate().is_ok(), "loopback http needs no pin");
    }

    #[test]
    fn duplicate_names_are_refused() {
        use json_store::Validate;
        let mut f = ServersFile::default();
        f.upsert(record("s1", "alpha"));
        f.upsert(record("s2", "alpha"));
        assert!(matches!(f.validate(), Err(ValidationError::Duplicate(_))));
    }

    #[test]
    fn find_prefers_the_id_then_the_name() {
        let mut f = ServersFile::default();
        f.upsert(record("s1", "alpha"));
        assert_eq!(f.find("alpha").unwrap().server_id, "s1");
        assert_eq!(f.find("s1").unwrap().name, "alpha");
        assert!(f.find("nope").is_none());
    }
}

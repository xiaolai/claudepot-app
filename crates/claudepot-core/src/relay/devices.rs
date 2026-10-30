//! Relay devices: who may call, and which upstreams each may reach.
//!
//! A registry opened with [`DeviceRegistry::open`] writes every mutation
//! to `relay-devices.json` before it takes effect in memory, so a failed
//! write leaves both unchanged. [`DeviceRegistry::new`] is in-memory only
//! (tests).
//!
//! A relay token is `cprl_` + 64 hex characters. The prefix keeps it
//! visibly distinct from a remote-control device token (bare hex), and
//! [`authenticate`](DeviceRegistry::authenticate) refuses anything
//! without it before hashing, so a remote token is never even compared.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use super::cancel::CancelToken;
use super::store::{self, DevicesFile, StoreError};
use super::target::UpstreamId;
use crate::json_store::CorruptionRecovery;
use crate::remote::token::{hash_secret, new_device_token, verify_secret};

pub const TOKEN_PREFIX: &str = "cprl_";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Grant {
    pub upstream_id: UpstreamId,
    /// The target generation the device accepted. A newer generation
    /// (a different account or route behind the same id) is refused
    /// until it is accepted again.
    pub accepted_generation: u64,
    /// The config epoch the generation belongs to. A reset config
    /// restarts generations at 1 under a new epoch, so a grant from
    /// before the reset never matches a target published after it.
    #[serde(default)]
    pub epoch: String,
}

impl Grant {
    /// Whether this grant covers a target published at `generation` under
    /// the config `epoch`. The one acceptance test: authorization, the
    /// accept query and health all ask it, so they cannot disagree.
    pub fn covers(&self, generation: u64, epoch: &str) -> bool {
        self.accepted_generation == generation && self.epoch == epoch
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceRecord {
    pub id: String,
    pub name: String,
    /// SHA-256 of the full token, prefix included. Never the token.
    pub token_hash: String,
    #[serde(default)]
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub grants: Vec<Grant>,
    #[serde(default = "chrono::Utc::now")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Upstreams published after pairing are offered for acceptance
    /// (`acceptance_required`) instead of refused as ungranted. Off by
    /// default: a new publication can spend a new account.
    #[serde(default)]
    pub include_future: bool,
}

impl DeviceRecord {
    pub fn grant_for(&self, id: &UpstreamId) -> Option<&Grant> {
        self.grants.iter().find(|g| &g.upstream_id == id)
    }
}

/// `cprl_` + `bare`, with every copy of the bearer material zeroized:
/// `bare` is owned by a [`Secret`](super::secret::Secret) from the start,
/// and the prefixed buffer is sized up front so no growth leaves an
/// unscrubbed reallocation behind.
fn prefixed_token(bare: String) -> super::secret::Secret {
    let bare = super::secret::Secret::new(bare);
    let mut full = String::with_capacity(TOKEN_PREFIX.len() + bare.expose().len());
    full.push_str(TOKEN_PREFIX);
    full.push_str(bare.expose());
    super::secret::Secret::new(full)
}

/// A freshly issued token. The plaintext exists only here, to be shown
/// once; the registry keeps the hash.
pub struct IssuedToken {
    pub device_id: String,
    pub plaintext: super::secret::Secret,
}

#[derive(Default)]
struct Inner {
    path: Option<PathBuf>,
    devices: Vec<DeviceRecord>,
    /// One cancel token per device, shared by all of its in-flight
    /// requests. Revoke fires it; a fresh one replaces it on re-issue.
    cancels: HashMap<String, CancelToken>,
}

impl Inner {
    /// Persist `next`, and only then make it current.
    fn replace(&mut self, next: Vec<DeviceRecord>) -> Result<(), StoreError> {
        if let Some(path) = &self.path {
            store::save_devices(
                path,
                &DevicesFile {
                    schema_version: store::SCHEMA_VERSION,
                    devices: next.clone(),
                },
            )?;
        }
        self.devices = next;
        Ok(())
    }
}

/// What [`DeviceRegistry::revoke`] did to a device that exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revocation {
    /// It was live and is now revoked, on disk first.
    Revoked,
    /// It was already revoked; nothing was written.
    AlreadyRevoked,
}

#[derive(Clone, Default)]
pub struct DeviceRegistry(Arc<Mutex<Inner>>);

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the on-disk registry. A reset file is returned, not hidden:
    /// it erased every pairing and every recorded revocation.
    pub fn open(path: PathBuf) -> Result<(Self, Option<CorruptionRecovery>), StoreError> {
        let loaded = store::load_devices(&path)?;
        let cancels = loaded
            .value
            .devices
            .iter()
            .map(|d| (d.id.clone(), CancelToken::new()))
            .collect();
        let reg = Self(Arc::new(Mutex::new(Inner {
            path: Some(path),
            devices: loaded.value.devices,
            cancels,
        })));
        Ok((reg, loaded.recovery))
    }

    /// Apply `f` to a copy of the device list, persist the copy, and only
    /// then make it current.
    fn mutate<R>(&self, f: impl FnOnce(&mut Vec<DeviceRecord>) -> R) -> Result<R, StoreError> {
        let mut inner = self.0.lock();
        let mut next = inner.devices.clone();
        let r = f(&mut next);
        inner.replace(next)?;
        Ok(r)
    }

    pub fn issue(&self, name: &str, grants: Vec<Grant>) -> Result<IssuedToken, StoreError> {
        self.enroll(name, grants, false)
    }

    /// Issue a token. Pairing is the only enrollment path.
    pub fn enroll(
        &self,
        name: &str,
        grants: Vec<Grant>,
        include_future: bool,
    ) -> Result<IssuedToken, StoreError> {
        let plaintext = prefixed_token(new_device_token().plaintext);
        let id = uuid::Uuid::new_v4().to_string();
        let record = DeviceRecord {
            id: id.clone(),
            name: name.to_owned(),
            token_hash: hash_secret(plaintext.expose()),
            revoked_at: None,
            grants,
            created_at: chrono::Utc::now(),
            include_future,
        };
        self.mutate(|devices| devices.push(record))?;
        self.0.lock().cancels.insert(id.clone(), CancelToken::new());
        Ok(IssuedToken {
            device_id: id,
            plaintext,
        })
    }

    /// Record that `device_id` accepted `grant`'s generation of its
    /// upstream, replacing any older acceptance.
    pub fn accept(&self, device_id: &str, grant: Grant) -> Result<bool, StoreError> {
        self.mutate(|devices| {
            let Some(d) = devices
                .iter_mut()
                .find(|d| d.id == device_id && d.revoked_at.is_none())
            else {
                return false;
            };
            d.grants.retain(|g| g.upstream_id != grant.upstream_id);
            d.grants.push(grant);
            true
        })
    }

    /// The live device a presented token belongs to, with its cancel
    /// token. Every record is compared — no early exit on a match — so
    /// the time taken does not depend on which device matched.
    pub fn authenticate(&self, presented: &str) -> Option<(DeviceRecord, CancelToken)> {
        if !presented.starts_with(TOKEN_PREFIX) {
            return None;
        }
        let inner = self.0.lock();
        let mut found = None;
        for d in &inner.devices {
            if verify_secret(presented, &d.token_hash) && found.is_none() {
                found = Some(d);
            }
        }
        let d = found?;
        if d.revoked_at.is_some() {
            return None;
        }
        let cancel = inner.cancels.get(&d.id)?.clone();
        Some((d.clone(), cancel))
    }

    /// Refuse the device's next request and end its in-flight streams;
    /// `None` when no device has this id. The cancel fires only once the
    /// revocation is on disk.
    ///
    /// Idempotent: revoking a revoked device writes nothing, fires its
    /// cancel again and says [`Revocation::AlreadyRevoked`], so a retry
    /// after an unknown outcome is safe and reports what it found.
    pub fn revoke(&self, device_id: &str) -> Result<Option<Revocation>, StoreError> {
        let mut inner = self.0.lock();
        let Some(i) = inner.devices.iter().position(|d| d.id == device_id) else {
            return Ok(None);
        };
        let outcome = if inner.devices[i].revoked_at.is_some() {
            Revocation::AlreadyRevoked
        } else {
            let mut next = inner.devices.clone();
            next[i].revoked_at = Some(chrono::Utc::now());
            inner.replace(next)?;
            Revocation::Revoked
        };
        if let Some(c) = inner.cancels.get(device_id) {
            c.cancel();
        }
        Ok(Some(outcome))
    }

    pub fn list(&self) -> Vec<DeviceRecord> {
        self.0.lock().devices.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_authenticates_its_device_and_nothing_else() {
        let reg = DeviceRegistry::new();
        let a = reg.issue("a", vec![]).unwrap();
        let b = reg.issue("b", vec![]).unwrap();
        assert_eq!(
            reg.authenticate(a.plaintext.expose()).unwrap().0.id,
            a.device_id
        );
        assert_eq!(
            reg.authenticate(b.plaintext.expose()).unwrap().0.id,
            b.device_id
        );
        assert!(reg.authenticate("cprl_00").is_none());
        assert!(reg.authenticate("").is_none());
    }

    #[test]
    fn a_token_without_the_relay_prefix_is_refused_even_if_its_hash_matches() {
        let reg = DeviceRegistry::new();
        let a = reg.issue("a", vec![]).unwrap();
        let bare = a
            .plaintext
            .expose()
            .trim_start_matches(TOKEN_PREFIX)
            .to_owned();
        // Plant a record whose hash IS the bare token's, as a remote
        // token's would be: the prefix check must refuse it first.
        reg.0.lock().devices.push(DeviceRecord {
            id: "remote".into(),
            name: "remote".into(),
            token_hash: hash_secret(&bare),
            revoked_at: None,
            grants: vec![],
            created_at: chrono::Utc::now(),
            include_future: false,
        });
        assert!(reg.authenticate(&bare).is_none());
    }

    #[test]
    fn an_issued_token_is_the_prefix_and_64_hex() {
        let reg = DeviceRegistry::new();
        let a = reg.issue("a", vec![]).unwrap();
        let bare = a.plaintext.expose().strip_prefix(TOKEN_PREFIX).unwrap();
        assert_eq!(bare.len(), 64);
        assert!(bare.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn a_grant_covers_only_its_own_generation_and_epoch() {
        let g = Grant {
            upstream_id: UpstreamId::parse("sub").unwrap(),
            accepted_generation: 2,
            epoch: "e1".into(),
        };
        assert!(g.covers(2, "e1"));
        assert!(!g.covers(3, "e1"), "a newer publication");
        assert!(!g.covers(1, "e1"), "an older publication");
        assert!(!g.covers(2, "e2"), "same generation under a reset config");
    }

    /// Files written while `DeviceRecord` still carried `loopback` must
    /// keep loading, or an upgrade would move every pairing aside.
    #[test]
    fn a_devices_file_with_the_retired_loopback_field_still_loads() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join(store::DEVICES_FILENAME);
        let reg = DeviceRegistry::open(path.clone()).unwrap().0;
        let a = reg.issue("a", vec![]).unwrap();
        let mut v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        v["devices"][0]["loopback"] = serde_json::Value::Bool(true);
        std::fs::write(&path, v.to_string()).unwrap();

        let (again, recovery) = DeviceRegistry::open(path).unwrap();
        assert!(recovery.is_none(), "the old shape was treated as corrupt");
        assert!(again.authenticate(a.plaintext.expose()).is_some());
    }

    #[test]
    fn revoke_refuses_the_token_and_fires_the_cancel() {
        let reg = DeviceRegistry::new();
        let a = reg.issue("a", vec![]).unwrap();
        let (_, cancel) = reg.authenticate(a.plaintext.expose()).unwrap();
        assert_eq!(reg.revoke(&a.device_id).unwrap(), Some(Revocation::Revoked));
        assert!(cancel.is_cancelled());
        assert!(reg.authenticate(a.plaintext.expose()).is_none());
    }

    /// A retry after an unknown outcome must be safe and say what it
    /// found: an already-revoked device is not "no such device", and
    /// revoking it again writes nothing.
    #[test]
    fn revoking_twice_writes_nothing_and_says_already_revoked() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join(store::DEVICES_FILENAME);
        let reg = DeviceRegistry::open(path.clone()).unwrap().0;
        let a = reg.issue("a", vec![]).unwrap();
        assert_eq!(reg.revoke(&a.device_id).unwrap(), Some(Revocation::Revoked));
        // A directory where the file was: any further save would fail.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            reg.revoke(&a.device_id).unwrap(),
            Some(Revocation::AlreadyRevoked)
        );
        assert_eq!(reg.revoke("no-such-device").unwrap(), None);
    }

    /// `atomic_write` reports a landed rename as written even when the
    /// directory sync after it fails, so the registry and the file agree.
    #[cfg(unix)]
    #[test]
    fn a_revoke_whose_directory_cannot_be_synced_still_takes_effect() {
        use std::os::unix::fs::PermissionsExt;
        // SAFETY: geteuid has no preconditions and cannot fail.
        if unsafe { libc::geteuid() } == 0 {
            return; // root opens any directory, so the precondition cannot hold
        }
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("relay");
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join(store::DEVICES_FILENAME);
        let reg = DeviceRegistry::open(path.clone()).unwrap().0;
        let a = reg.issue("a", vec![]).unwrap();
        let mode = |m| std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(m));
        mode(0o300).unwrap();
        let revoked = reg.revoke(&a.device_id);
        let opened = std::fs::File::open(&dir).map(|_| ());
        mode(0o700).unwrap();
        assert!(
            opened.is_err(),
            "precondition: a 0300 directory cannot be opened"
        );
        assert_eq!(revoked.unwrap(), Some(Revocation::Revoked));
        assert!(reg.authenticate(a.plaintext.expose()).is_none());
        let on_disk = store::load_devices(&path).unwrap().value.devices;
        assert!(
            on_disk[0].revoked_at.is_some(),
            "the file and memory disagree"
        );
    }

    #[test]
    fn the_stored_record_never_holds_the_plaintext() {
        let reg = DeviceRegistry::new();
        let a = reg.issue("a", vec![]).unwrap();
        let json = serde_json::to_string(&reg.list()).unwrap();
        assert!(!json.contains(a.plaintext.expose()));
    }

    #[test]
    fn a_device_survives_reopening_and_a_revocation_too() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join(store::DEVICES_FILENAME);
        let (reg, rec) = DeviceRegistry::open(path.clone()).unwrap();
        assert!(rec.is_none());
        let a = reg.issue("a", vec![]).unwrap();
        let b = reg.issue("b", vec![]).unwrap();
        reg.revoke(&b.device_id).unwrap();

        let (again, _) = DeviceRegistry::open(path.clone()).unwrap();
        assert!(again.authenticate(a.plaintext.expose()).is_some());
        assert!(again.authenticate(b.plaintext.expose()).is_none());
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains(a.plaintext.expose()));
    }

    #[test]
    fn a_failed_write_leaves_the_registry_unchanged() {
        let d = tempfile::tempdir().unwrap();
        // A directory where the file should be: every save fails.
        let path = d.path().join("blocked");
        std::fs::create_dir(&path).unwrap();
        let reg = DeviceRegistry(Arc::new(Mutex::new(Inner {
            path: Some(path),
            ..Inner::default()
        })));
        assert!(reg.issue("a", vec![]).is_err());
        assert!(reg.list().is_empty());
    }

    #[test]
    fn accept_replaces_the_older_grant() {
        let reg = DeviceRegistry::new();
        let up = UpstreamId::parse("sub").unwrap();
        let g = |n| Grant {
            upstream_id: up.clone(),
            accepted_generation: n,
            epoch: "e".into(),
        };
        let a = reg.issue("a", vec![g(1)]).unwrap();
        assert!(reg.accept(&a.device_id, g(2)).unwrap());
        let rec = reg.list().pop().unwrap();
        assert_eq!(rec.grants, vec![g(2)]);
        assert!(!reg.accept("nope", g(3)).unwrap());
    }
}

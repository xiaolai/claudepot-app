//! What an upstream *is*, as a digest, and how a client sees it.
//!
//! Two values, for two audiences:
//!
//! - [`TargetSpec::digest`] is the server's own record: a plain SHA-256
//!   over what decides where a request goes and who pays. It is stored in
//!   `relay-config.json`, and a change to it bumps the upstream's
//!   generation. It has no per-installation key, so two servers serving
//!   one account compute the same digest.
//! - [`client_fingerprint`] and [`account_fingerprint`] are what a client
//!   sees: HMAC-SHA256 keyed by the client's own salt (plan §5.3). The
//!   same client gets equal values from every server serving one target,
//!   and two clients cannot correlate theirs.
//!
//! Never an input: a key or token. A route's credential enters as a
//! digest of its key *id*.

use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

const ACCOUNT_DOMAIN: &[u8] = b"claudepot-relay-account-v1";
const TARGET_DOMAIN: &[u8] = b"claudepot-relay-target-v1";

/// A client salt is 16–64 random bytes, sent hex-encoded.
pub const MIN_SALT_BYTES: usize = 16;
pub const MAX_SALT_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Subscription,
    PassThrough,
    Translated,
}

/// Who the upstream bills.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CredentialIdentity {
    /// The provider's identity from the OAuth profile — not Claudepot's
    /// local account id, which differs per machine.
    Account {
        account_uuid: String,
        org_uuid: String,
    },
    /// A digest of a route secret's key id, never the key.
    Key { key_id_digest: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TargetSpec {
    pub kind: TargetKind,
    pub origin: String,
    #[serde(default)]
    pub model_map: BTreeMap<String, String>,
    pub credential: CredentialIdentity,
    /// The wire protocol a translated target speaks (`translate::Translation`).
    /// Absent for subscription and pass-through targets, and left out of the
    /// canonical encoding then, so their digests are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
}

impl TargetSpec {
    pub fn subscription(origin: &str, account_uuid: &str, org_uuid: &str) -> Self {
        Self {
            kind: TargetKind::Subscription,
            origin: origin.to_owned(),
            model_map: BTreeMap::new(),
            credential: CredentialIdentity::Account {
                account_uuid: account_uuid.to_owned(),
                org_uuid: org_uuid.to_owned(),
            },
            protocol: None,
        }
    }

    /// Length-prefixed fields in a fixed order, so no two specs share an
    /// encoding.
    fn canonical(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut field = |b: &[u8]| {
            out.extend_from_slice(&(b.len() as u64).to_be_bytes());
            out.extend_from_slice(b);
        };
        field(match self.kind {
            TargetKind::Subscription => b"subscription",
            TargetKind::PassThrough => b"pass_through",
            TargetKind::Translated => b"translated",
        });
        field(self.origin.as_bytes());
        field(&(self.model_map.len() as u64).to_be_bytes());
        for (k, v) in &self.model_map {
            field(k.as_bytes());
            field(v.as_bytes());
        }
        match &self.credential {
            CredentialIdentity::Account {
                account_uuid,
                org_uuid,
            } => {
                field(b"account");
                field(account_uuid.to_ascii_lowercase().as_bytes());
                field(org_uuid.to_ascii_lowercase().as_bytes());
            }
            CredentialIdentity::Key { key_id_digest } => {
                field(b"key");
                field(key_id_digest.as_bytes());
            }
        }
        if let Some(p) = &self.protocol {
            field(b"protocol");
            field(p.as_bytes());
        }
        out
    }

    /// The server-side digest, lowercase hex.
    pub fn digest(&self) -> String {
        hex::encode(Sha256::digest(self.canonical()))
    }
}

/// Decode and bound a client salt.
pub fn parse_salt(hex_salt: &str) -> Option<Vec<u8>> {
    let bytes = hex::decode(hex_salt).ok()?;
    (MIN_SALT_BYTES..=MAX_SALT_BYTES)
        .contains(&bytes.len())
        .then_some(bytes)
}

fn hmac(salt: &[u8], parts: &[&[u8]]) -> String {
    // HMAC accepts a key of any length; the error arm is unreachable.
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(salt) else {
        return String::new();
    };
    for p in parts {
        mac.update(p);
    }
    hex::encode(mac.finalize().into_bytes())
}

/// Plan §5.3, exactly: `HMAC-SHA256(key = client_salt,
/// "claudepot-relay-account-v1" ‖ account_uuid ‖ org_uuid)`. UUIDs are
/// fixed-length in their canonical form, so plain concatenation is
/// unambiguous.
pub fn account_fingerprint(salt: &[u8], account_uuid: &str, org_uuid: &str) -> String {
    hmac(
        salt,
        &[
            ACCOUNT_DOMAIN,
            account_uuid.to_ascii_lowercase().as_bytes(),
            org_uuid.to_ascii_lowercase().as_bytes(),
        ],
    )
}

/// The target fingerprint a client approves, keyed by its salt over the
/// server-side digest.
pub fn client_fingerprint(salt: &[u8], digest: &str) -> String {
    hmac(salt, &[TARGET_DOMAIN, digest.as_bytes()])
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: (&str, &str) = (
        "0b6c4f7e-2a8d-4c3e-9f51-7d2e8a1b3c40",
        "c8f1a2b3-4d5e-4f60-8172-93a4b5c6d7e8",
    );
    const B: (&str, &str) = (
        "1d2e3f40-5a6b-4c7d-8e9f-a0b1c2d3e4f5",
        "c8f1a2b3-4d5e-4f60-8172-93a4b5c6d7e8",
    );

    #[test]
    fn two_servers_serving_one_account_give_one_client_equal_values() {
        let salt = [7u8; 32];
        // Two clean server profiles compute these independently: no
        // input is local to either installation.
        let server1 = TargetSpec::subscription("https://api.anthropic.com", A.0, A.1);
        let server2 = TargetSpec::subscription("https://api.anthropic.com", A.0, A.1);
        assert_eq!(
            client_fingerprint(&salt, &server1.digest()),
            client_fingerprint(&salt, &server2.digest())
        );
        assert_eq!(
            account_fingerprint(&salt, A.0, A.1),
            account_fingerprint(&salt, &A.0.to_uppercase(), A.1)
        );
    }

    #[test]
    fn different_accounts_differ() {
        let salt = [7u8; 32];
        assert_ne!(
            account_fingerprint(&salt, A.0, A.1),
            account_fingerprint(&salt, B.0, B.1)
        );
        let a = TargetSpec::subscription("https://api.anthropic.com", A.0, A.1).digest();
        let b = TargetSpec::subscription("https://api.anthropic.com", B.0, B.1).digest();
        assert_ne!(a, b);
    }

    #[test]
    fn two_clients_cannot_correlate_their_values() {
        let d = TargetSpec::subscription("https://api.anthropic.com", A.0, A.1).digest();
        assert_ne!(
            client_fingerprint(&[1; 32], &d),
            client_fingerprint(&[2; 32], &d)
        );
        assert_ne!(
            account_fingerprint(&[1; 32], A.0, A.1),
            account_fingerprint(&[2; 32], A.0, A.1)
        );
    }

    #[test]
    fn origin_mapping_and_key_all_change_the_digest() {
        let base = TargetSpec {
            kind: TargetKind::PassThrough,
            origin: "https://api.deepseek.com/anthropic".into(),
            model_map: [("sonnet".into(), "deepseek-chat".into())].into(),
            credential: CredentialIdentity::Key {
                key_id_digest: "k1".into(),
            },
            protocol: None,
        };
        let mut origin = base.clone();
        origin.origin = "https://elsewhere.example".into();
        let mut mapping = base.clone();
        mapping
            .model_map
            .insert("sonnet".into(), "deepseek-reasoner".into());
        let mut key = base.clone();
        key.credential = CredentialIdentity::Key {
            key_id_digest: "k2".into(),
        };
        for other in [origin, mapping, key] {
            assert_ne!(base.digest(), other.digest());
        }
    }

    #[test]
    fn protocol_changes_the_digest_only_when_present() {
        let base = TargetSpec::subscription("https://api.anthropic.com", A.0, A.1);
        // Pinned: adding the optional field must not re-key existing grants.
        let before = base.digest();
        let mut chat = base.clone();
        chat.protocol = Some("chat".into());
        let mut gemini = base.clone();
        gemini.protocol = Some("gemini".into());
        assert!(!base.canonical().windows(8).any(|w| w == b"protocol"));
        assert_ne!(chat.digest(), before);
        assert_ne!(chat.digest(), gemini.digest());
        let json = serde_json::to_value(&base).unwrap_or_default();
        assert!(json.get("protocol").is_none());
    }

    #[test]
    fn a_salt_outside_the_bounds_is_refused() {
        assert!(parse_salt(&"ab".repeat(15)).is_none());
        assert!(parse_salt(&"ab".repeat(16)).is_some());
        assert!(parse_salt(&"ab".repeat(64)).is_some());
        assert!(parse_salt(&"ab".repeat(65)).is_none());
        assert!(parse_salt("zz").is_none());
    }
}

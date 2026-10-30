//! The relay's own trust root: a CA and a leaf, minted by Claudepot.
//!
//! Separate from remote control's `remote-cert.pem` / `remote-key.pem`
//! on purpose (plan §3): rotating either trust root must leave the other
//! working, and a relay client pins *this* CA, never the remote one.
//! The vetting of on-disk material (PEM sniff, size, owner-only key) is
//! [`crate::remote::tls::load_from`], called with relay paths — a shared
//! function, not a shared loader.
//!
//! ## The CA key is only for renewal, and it fails closed
//!
//! `relay-ca-key.pem` signs leaves and nothing else. With a valid leaf it
//! is not needed at all, so a missing CA key does not stop a server
//! whose leaf is still fresh. Once the leaf needs renewing, a missing or
//! corrupt CA key is [`TrustError::CaKeyUnavailable`] — never a silently
//! minted new CA, which would break every pairing without saying so. A
//! new CA comes only from [`rotate_ca`], which the caller must present as
//! "every paired device must pair again".

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls::client::danger::ServerCertVerifier;
use rustls::client::WebPkiServerVerifier;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};

use crate::remote::tls::{load_from, TlsMaterialError};

pub const CA_CERT_FILENAME: &str = "relay-ca.pem";
pub const CA_KEY_FILENAME: &str = "relay-ca-key.pem";
pub const CERT_FILENAME: &str = "relay-cert.pem";
pub const KEY_FILENAME: &str = "relay-key.pem";

/// A leaf is valid for this long from minting.
pub const LEAF_VALIDITY: Duration = Duration::from_secs(90 * 24 * 3600);
/// A leaf with less than this left is renewed — at start, and by the
/// serving relay's periodic check ([`renew_serving`]).
pub const LEAF_RENEW_MARGIN: Duration = Duration::from_secs(30 * 24 * 3600);
const CA_VALIDITY: Duration = Duration::from_secs(10 * 365 * 24 * 3600);
/// Back-dating absorbs clock skew between this server and its clients.
const BACKDATE: Duration = Duration::from_secs(3600);

/// Fixed so a renewal can rebuild the issuer without parsing the CA
/// certificate; the CA's identity to a client is its fingerprint, not
/// its name.
const CA_COMMON_NAME: &str = "Claudepot Relay CA";
const LEAF_COMMON_NAME: &str = "Claudepot Relay";

#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    /// The leaf needs renewing and the CA key is missing, unreadable or
    /// does not sign for this CA. The stable code is the banner's.
    #[error(
        "relay_ca_key_unavailable: the relay's certificate needs renewing and \
         {path} is missing or unusable. Restore it from where it was kept, or \
         rotate the relay CA — which makes every paired device pair again."
    )]
    CaKeyUnavailable { path: String },

    /// Some trust files exist and the CA certificate does not: a partial
    /// directory is never completed by minting a new CA.
    #[error(
        "the relay CA certificate {path} is missing while other relay trust \
         files exist. Restore it, or rotate the relay CA explicitly."
    )]
    CaMissing { path: String },

    #[error("the relay CA certificate {path} is not a usable certificate")]
    CaInvalid { path: String },

    #[error("relay TLS material: {0}")]
    Material(#[from] TlsMaterialError),

    #[error("cannot write {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot mint relay certificates: {0}")]
    Mint(String),

    #[error("relay TLS configuration: {0}")]
    Config(String),
}

impl TrustError {
    pub fn code(&self) -> &'static str {
        match self {
            TrustError::CaKeyUnavailable { .. } => "relay_ca_key_unavailable",
            TrustError::CaMissing { .. } | TrustError::CaInvalid { .. } => "relay_ca_unavailable",
            TrustError::Material(_) | TrustError::Write { .. } => "relay_tls_material",
            TrustError::Mint(_) | TrustError::Config(_) => "relay_tls_config",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrustPaths {
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl TrustPaths {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            ca_cert: dir.join(CA_CERT_FILENAME),
            ca_key: dir.join(CA_KEY_FILENAME),
            cert: dir.join(CERT_FILENAME),
            key: dir.join(KEY_FILENAME),
        }
    }

    pub fn default_location() -> Self {
        Self::in_dir(&crate::paths::claudepot_data_dir())
    }

    fn all(&self) -> [&Path; 4] {
        [&self.ca_cert, &self.ca_key, &self.cert, &self.key]
    }
}

/// What [`ensure`] had to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ensured {
    /// Nothing existed: a new CA and leaf (first enable).
    Minted,
    /// The leaf was missing, near expiry, or lacked a name; a new one was
    /// signed by the existing CA.
    Renewed,
    /// The leaf on disk was good.
    Loaded,
}

/// Loaded trust material. The CA PEM and its fingerprint are public;
/// the leaf key never leaves this struct except into rustls.
pub struct Trust {
    pub ca_pem: String,
    /// SHA-256 of the CA certificate's DER, `AB:CD:…` — the same form
    /// `openssl x509 -fingerprint -sha256` prints, so it can be checked
    /// by hand.
    pub ca_fingerprint: String,
    /// Whether the CA key is present and owner-only: a `false` here means
    /// the next renewal will fail closed, which the UI raises before the
    /// leaf expires.
    pub renewal_possible: bool,
    leaf_chain_pem: Vec<u8>,
    leaf_key_pem: Vec<u8>,
}

impl std::fmt::Debug for Trust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trust")
            .field("ca_fingerprint", &self.ca_fingerprint)
            .field("renewal_possible", &self.renewal_possible)
            .finish_non_exhaustive()
    }
}

/// SHA-256 over a certificate's DER, colon-separated upper-case hex.
pub fn fingerprint_der(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The fingerprint of the first certificate in `pem`.
pub fn fingerprint_pem(pem: &[u8]) -> Option<String> {
    let der = CertificateDer::pem_slice_iter(pem).next()?.ok()?;
    Some(fingerprint_der(der.as_ref()))
}

/// A fingerprint typed or pasted by a person, reduced to bare upper-case
/// hex so `ab:cd…`, `ABCD…` and `AB CD …` compare equal.
pub fn normalize_fingerprint(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Load the relay's trust, renewing the leaf under the same CA when it
/// is missing, near expiry or does not cover `names`; mint everything
/// only when no relay trust file exists at all.
pub fn ensure(
    paths: &TrustPaths,
    names: &[String],
    now: SystemTime,
) -> Result<(Trust, Ensured), TrustError> {
    ensure_under(paths, names, now, None)
}

/// [`ensure`], refusing — before anything is written — a CA whose
/// fingerprint is not `pinned`. With a pin it never mints: an empty
/// directory is [`TrustError::CaMissing`].
fn ensure_under(
    paths: &TrustPaths,
    names: &[String],
    now: SystemTime,
    pinned: Option<&str>,
) -> Result<(Trust, Ensured), TrustError> {
    if pinned.is_none() && paths.all().iter().all(|p| !p.exists()) {
        let trust = mint_all(paths, names, now)?;
        return Ok((trust, Ensured::Minted));
    }
    if !paths.ca_cert.exists() {
        return Err(TrustError::CaMissing {
            path: paths.ca_cert.display().to_string(),
        });
    }
    let ca_pem = read_ca(&paths.ca_cert)?;
    let ca_der = ca_der(&ca_pem, &paths.ca_cert)?;
    // Checked on the bytes this call signs under, so a CA swapped in
    // after the check cannot be the one a renewal writes a leaf for.
    if let Some(pin) = pinned {
        if fingerprint_der(ca_der.as_ref()) != pin {
            return Err(TrustError::CaInvalid {
                path: paths.ca_cert.display().to_string(),
            });
        }
    }

    if paths.cert.exists() && paths.key.exists() {
        let leaf = load_from(&paths.cert, &paths.key)?;
        let horizon = now + LEAF_RENEW_MARGIN;
        // A key that is not the certificate's (an interrupted renewal
        // wrote one file and not the other) cannot serve: renew it.
        if leaf_is_valid(&ca_der, &leaf.cert_pem, names, horizon)
            && leaf_key_matches(&leaf.cert_pem, &leaf.key_pem)
        {
            return Ok((
                Trust {
                    renewal_possible: load_ca_key(paths).is_ok(),
                    ca_fingerprint: fingerprint_der(ca_der.as_ref()),
                    ca_pem,
                    leaf_chain_pem: leaf.cert_pem,
                    leaf_key_pem: leaf.key_pem,
                },
                Ensured::Loaded,
            ));
        }
    }

    let ca_key = load_ca_key(paths)?;
    let (chain, key) = mint_leaf(&ca_pem, &ca_key, names, now)?;
    // A key that is not this CA's signs a leaf that does not chain to it:
    // that is a corrupt CA key, and it must not reach the disk as a leaf.
    if !leaf_is_valid(&ca_der, chain.as_bytes(), names, now) {
        return Err(TrustError::CaKeyUnavailable {
            path: paths.ca_key.display().to_string(),
        });
    }
    write(&paths.cert, chain.as_bytes())?;
    write_key(&paths.key, key.as_bytes())?;
    Ok((
        Trust {
            renewal_possible: true,
            ca_fingerprint: fingerprint_der(ca_der.as_ref()),
            ca_pem,
            leaf_chain_pem: chain.into_bytes(),
            leaf_key_pem: key.into_bytes(),
        },
        Ensured::Renewed,
    ))
}

/// The serving relay's renewal check: a fresh leaf under the **same** CA
/// when the current one is due, `None` when it is not. Unlike [`ensure`]
/// it never mints — a trust directory emptied while serving is
/// [`TrustError::CaMissing`], and a CA that is no longer the one clients
/// pinned is [`TrustError::CaInvalid`]; either way the current leaf stays,
/// on disk as well as in the listener — the pin is checked before any
/// write.
pub fn renew_serving(
    paths: &TrustPaths,
    names: &[String],
    pinned_ca: &str,
    now: SystemTime,
) -> Result<Option<Trust>, TrustError> {
    let (trust, how) = ensure_under(paths, names, now, Some(pinned_ca))?;
    Ok((how == Ensured::Renewed).then_some(trust))
}

/// The listener's certificate, swappable while serving so a renewed leaf
/// takes effect on the next handshake without a restart. Connections
/// already open keep the leaf they shook hands with.
#[derive(Debug)]
pub struct LeafResolver(parking_lot::RwLock<Arc<CertifiedKey>>);

impl LeafResolver {
    pub fn new(trust: &Trust) -> Result<Arc<Self>, TrustError> {
        Ok(Arc::new(Self(parking_lot::RwLock::new(certified(trust)?))))
    }

    /// Serve `trust`'s leaf from the next handshake on.
    pub fn replace(&self, trust: &Trust) -> Result<(), TrustError> {
        let key = certified(trust)?;
        *self.0.write() = key;
        Ok(())
    }

    /// The fingerprint of the leaf now being served.
    pub fn leaf_fingerprint(&self) -> Option<String> {
        self.0
            .read()
            .cert
            .first()
            .map(|c| fingerprint_der(c.as_ref()))
    }
}

impl ResolvesServerCert for LeafResolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.read().clone())
    }
}

fn certified(trust: &Trust) -> Result<Arc<CertifiedKey>, TrustError> {
    certified_key(&trust.leaf_chain_pem, &trust.leaf_key_pem).map(Arc::new)
}

/// The one PEM → [`CertifiedKey`] conversion: what the listener serves,
/// and what [`ensure`] checks a leaf pair against before loading it.
fn certified_key(chain_pem: &[u8], key_pem: &[u8]) -> Result<CertifiedKey, TrustError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(chain_pem)
        .collect::<Result<_, _>>()
        .map_err(|e| TrustError::Config(format!("certificate: {e}")))?;
    let key = PrivateKeyDer::from_pem_slice(key_pem)
        .map_err(|e| TrustError::Config(format!("private key: {e}")))?;
    CertifiedKey::from_der(certs, key, &provider()).map_err(|e| TrustError::Config(e.to_string()))
}

/// Replace the CA and the leaf. Every existing pairing pinned the old CA
/// and stops working; the caller must say so before calling this.
pub fn rotate_ca(
    paths: &TrustPaths,
    names: &[String],
    now: SystemTime,
) -> Result<Trust, TrustError> {
    mint_all(paths, names, now)
}

/// The rustls server configuration for the relay listener: the leaf and
/// the CA as its chain, HTTP/1.1 only (the listener speaks nothing else).
pub fn server_config(trust: &Trust) -> Result<Arc<rustls::ServerConfig>, TrustError> {
    server_config_with(LeafResolver::new(trust)?)
}

/// [`server_config`] over a resolver the caller keeps, to renew through.
pub fn server_config_with(
    leaf: Arc<LeafResolver>,
) -> Result<Arc<rustls::ServerConfig>, TrustError> {
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| TrustError::Config(e.to_string()))?
        .with_no_client_auth()
        .with_cert_resolver(leaf);
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

pub(crate) fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn mint_all(paths: &TrustPaths, names: &[String], now: SystemTime) -> Result<Trust, TrustError> {
    let ca_key = KeyPair::generate().map_err(mint_err)?;
    let mut params = ca_params();
    params.not_before = offset(now, BACKDATE, false)?;
    params.not_after = offset(now, CA_VALIDITY, true)?;
    let ca_cert = params.self_signed(&ca_key).map_err(mint_err)?;
    let ca_pem = ca_cert.pem();
    let (chain, key) = mint_leaf(&ca_pem, &ca_key, names, now)?;

    // CA key first and leaf last: an interrupted rotation leaves a CA
    // without a leaf (renewable), never a leaf without its CA.
    write_key(&paths.ca_key, ca_key.serialize_pem().as_bytes())?;
    write(&paths.ca_cert, ca_pem.as_bytes())?;
    write(&paths.cert, chain.as_bytes())?;
    write_key(&paths.key, key.as_bytes())?;
    Ok(Trust {
        ca_fingerprint: fingerprint_der(ca_cert.der().as_ref()),
        renewal_possible: true,
        ca_pem,
        leaf_chain_pem: chain.into_bytes(),
        leaf_key_pem: key.into_bytes(),
    })
}

fn ca_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, CA_COMMON_NAME);
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params
}

/// A leaf for `names`, signed by the CA; returns (leaf + CA chain PEM,
/// leaf key PEM).
fn mint_leaf(
    ca_pem: &str,
    ca_key: &KeyPair,
    names: &[String],
    now: SystemTime,
) -> Result<(String, String), TrustError> {
    // The issuer is rebuilt from the one definition of the CA's name and
    // usages, so a renewal needs the key and nothing parsed from the CA.
    let ca = ca_params();
    let issuer = Issuer::from_params(&ca, ca_key);
    let mut params = CertificateParams::new(names.to_vec()).map_err(mint_err)?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, LEAF_COMMON_NAME);
    params.distinguished_name = dn;
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    params.not_before = offset(now, BACKDATE, false)?;
    params.not_after = offset(now, LEAF_VALIDITY, true)?;
    let leaf_key = KeyPair::generate().map_err(mint_err)?;
    let leaf = params.signed_by(&leaf_key, &issuer).map_err(mint_err)?;
    let chain = format!("{}{}", leaf.pem(), ca_pem);
    Ok((chain, leaf_key.serialize_pem()))
}

/// Whether `chain_pem`'s leaf chains to the CA and is valid at `at` for
/// every name in `names`.
fn leaf_is_valid(
    ca_der: &CertificateDer<'static>,
    chain_pem: &[u8],
    names: &[String],
    at: SystemTime,
) -> bool {
    let Ok(chain) = CertificateDer::pem_slice_iter(chain_pem).collect::<Result<Vec<_>, _>>() else {
        return false;
    };
    let Some((leaf, rest)) = chain.split_first() else {
        return false;
    };
    let mut roots = rustls::RootCertStore::empty();
    if roots.add(ca_der.clone()).is_err() {
        return false;
    }
    let Ok(verifier) =
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider()).build()
    else {
        return false;
    };
    let Ok(since) = at.duration_since(SystemTime::UNIX_EPOCH) else {
        return false;
    };
    let when = UnixTime::since_unix_epoch(since);
    names.iter().all(|name| {
        let Ok(server_name) = ServerName::try_from(name.clone()) else {
            return false;
        };
        verifier
            .verify_server_cert(leaf, rest, &server_name, &[], when)
            .is_ok()
    })
}

/// Whether the key is the leaf certificate's — what `certified` would
/// refuse at serve time, checked before trust is reported as loaded.
fn leaf_key_matches(chain_pem: &[u8], key_pem: &[u8]) -> bool {
    certified_key(chain_pem, key_pem).is_ok()
}

fn read_ca(path: &Path) -> Result<String, TrustError> {
    let bytes = std::fs::read(path).map_err(|_| TrustError::CaInvalid {
        path: path.display().to_string(),
    })?;
    String::from_utf8(bytes).map_err(|_| TrustError::CaInvalid {
        path: path.display().to_string(),
    })
}

fn ca_der(ca_pem: &str, path: &Path) -> Result<CertificateDer<'static>, TrustError> {
    CertificateDer::from_pem_slice(ca_pem.as_bytes()).map_err(|_| TrustError::CaInvalid {
        path: path.display().to_string(),
    })
}

fn load_ca_key(paths: &TrustPaths) -> Result<KeyPair, TrustError> {
    let unavailable = || TrustError::CaKeyUnavailable {
        path: paths.ca_key.display().to_string(),
    };
    if !paths.ca_key.exists() {
        return Err(unavailable());
    }
    // The shared vetting: owner-only key, plausible size, PEM.
    let material = load_from(&paths.ca_cert, &paths.ca_key).map_err(|_| unavailable())?;
    let pem = String::from_utf8(material.key_pem).map_err(|_| unavailable())?;
    KeyPair::from_pem(&pem).map_err(|_| unavailable())
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), TrustError> {
    crate::fs_utils::atomic_write(path, bytes).map_err(|source| TrustError::Write {
        path: path.display().to_string(),
        source,
    })
}

/// A private key: user-only on every platform before its bytes land.
fn write_key(path: &Path, bytes: &[u8]) -> Result<(), TrustError> {
    crate::fs_utils::atomic_write_secret(path, bytes).map_err(|source| TrustError::Write {
        path: path.display().to_string(),
        source,
    })
}

fn offset(
    now: SystemTime,
    by: Duration,
    forward: bool,
) -> Result<time::OffsetDateTime, TrustError> {
    let t = if forward {
        now.checked_add(by)
    } else {
        now.checked_sub(by)
    }
    .ok_or_else(|| TrustError::Mint("time out of range".into()))?;
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| TrustError::Mint("time before 1970".into()))?
        .as_secs();
    let secs = i64::try_from(secs).map_err(|_| TrustError::Mint("time out of range".into()))?;
    time::OffsetDateTime::from_unix_timestamp(secs).map_err(|e| TrustError::Mint(e.to_string()))
}

fn mint_err(e: rcgen::Error) -> TrustError {
    TrustError::Mint(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn names() -> Vec<String> {
        vec!["localhost".into(), "127.0.0.1".into()]
    }

    #[test]
    fn first_enable_mints_and_the_next_start_loads_the_same_ca() {
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        let now = SystemTime::now();
        let (a, how) = ensure(&paths, &names(), now).unwrap();
        assert_eq!(how, Ensured::Minted);
        let (b, how) = ensure(&paths, &names(), now).unwrap();
        assert_eq!(how, Ensured::Loaded);
        assert_eq!(a.ca_fingerprint, b.ca_fingerprint);
        assert!(b.renewal_possible);
        assert_eq!(
            fingerprint_pem(a.ca_pem.as_bytes()).unwrap(),
            a.ca_fingerprint
        );
        server_config(&b).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn both_private_keys_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        ensure(&paths, &names(), SystemTime::now()).unwrap();
        for p in [&paths.ca_key, &paths.key] {
            let mode = std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode & 0o077, 0, "{} is {mode:o}", p.display());
        }
    }

    #[test]
    fn an_expiring_leaf_is_renewed_under_the_same_ca() {
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        let t0 = SystemTime::now();
        let (a, _) = ensure(&paths, &names(), t0).unwrap();
        let leaf_before = std::fs::read(&paths.cert).unwrap();

        // Keep only the declared files, then restart past the margin.
        let later = t0 + LEAF_VALIDITY - LEAF_RENEW_MARGIN + Duration::from_secs(3600);
        let (b, how) = ensure(&paths, &names(), later).unwrap();
        assert_eq!(how, Ensured::Renewed);
        assert_eq!(a.ca_fingerprint, b.ca_fingerprint);
        assert_ne!(std::fs::read(&paths.cert).unwrap(), leaf_before);
        // And the renewed leaf is good at the later time.
        let (_, how) = ensure(&paths, &names(), later).unwrap();
        assert_eq!(how, Ensured::Loaded);
    }

    #[test]
    fn a_leaf_key_that_is_not_the_certificates_is_renewed_not_loaded() {
        // An interruption between the certificate write and the key
        // write leaves a pair that cannot serve; the CA key can fix it.
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        let now = SystemTime::now();
        ensure(&paths, &names(), now).unwrap();
        let stray = KeyPair::generate().unwrap().serialize_pem();
        write_key(&paths.key, stray.as_bytes()).unwrap();
        let (t, how) = ensure(&paths, &names(), now).unwrap();
        assert_eq!(how, Ensured::Renewed);
        server_config(&t).unwrap();
    }

    #[test]
    fn a_missing_ca_key_fails_closed_once_the_leaf_needs_renewing() {
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        let t0 = SystemTime::now();
        let (a, _) = ensure(&paths, &names(), t0).unwrap();
        std::fs::remove_file(&paths.ca_key).unwrap();

        // Leaf still fresh: serving continues, renewal flagged impossible.
        let (b, how) = ensure(&paths, &names(), t0).unwrap();
        assert_eq!(how, Ensured::Loaded);
        assert!(!b.renewal_possible);

        let later = t0 + LEAF_VALIDITY + Duration::from_secs(1);
        let err = ensure(&paths, &names(), later).unwrap_err();
        assert_eq!(err.code(), "relay_ca_key_unavailable", "{err}");
        // Never a silently minted CA.
        assert_eq!(
            fingerprint_pem(&std::fs::read(&paths.ca_cert).unwrap()).unwrap(),
            a.ca_fingerprint
        );
        assert!(!paths.ca_key.exists());
    }

    #[test]
    fn a_ca_key_that_is_not_this_cas_fails_closed() {
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        ensure(&paths, &names(), SystemTime::now()).unwrap();
        let other = KeyPair::generate().unwrap();
        write(&paths.ca_key, other.serialize_pem().as_bytes()).unwrap();
        std::fs::remove_file(&paths.cert).unwrap();
        let err = ensure(&paths, &names(), SystemTime::now()).unwrap_err();
        assert_eq!(err.code(), "relay_ca_key_unavailable", "{err}");
        assert!(
            !paths.cert.exists(),
            "a leaf that does not chain must not be written"
        );
    }

    #[test]
    fn a_missing_ca_certificate_is_never_replaced_silently() {
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        ensure(&paths, &names(), SystemTime::now()).unwrap();
        std::fs::remove_file(&paths.ca_cert).unwrap();
        assert!(matches!(
            ensure(&paths, &names(), SystemTime::now()).unwrap_err(),
            TrustError::CaMissing { .. }
        ));
        assert!(!paths.ca_cert.exists());
    }

    #[test]
    fn a_new_name_renews_the_leaf() {
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        let now = SystemTime::now();
        ensure(&paths, &names(), now).unwrap();
        let mut more = names();
        more.push("relay.example.ts.net".into());
        let (_, how) = ensure(&paths, &more, now).unwrap();
        assert_eq!(how, Ensured::Renewed);
    }

    #[test]
    fn a_serving_renewal_under_a_substituted_ca_writes_nothing() {
        // Both CA files replaced while serving: the old leaf no longer
        // chains, so `ensure` alone would sign and write a new leaf under
        // the substitute. The pin must refuse before any write.
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        let now = SystemTime::now();
        let (pinned, _) = ensure(&paths, &names(), now).unwrap();
        let other = TempDir::new().unwrap();
        let other_paths = TrustPaths::in_dir(other.path());
        ensure(&other_paths, &names(), now).unwrap();
        std::fs::copy(&other_paths.ca_cert, &paths.ca_cert).unwrap();
        std::fs::copy(&other_paths.ca_key, &paths.ca_key).unwrap();
        let (cert, key) = (
            std::fs::read(&paths.cert).unwrap(),
            std::fs::read(&paths.key).unwrap(),
        );

        let err = renew_serving(&paths, &names(), &pinned.ca_fingerprint, now).unwrap_err();
        assert!(matches!(err, TrustError::CaInvalid { .. }), "{err}");
        assert!(
            std::fs::read(&paths.cert).unwrap() == cert,
            "leaf rewritten"
        );
        assert!(
            std::fs::read(&paths.key).unwrap() == key,
            "leaf key rewritten"
        );
    }

    #[test]
    fn a_serving_renewal_under_the_pinned_ca_renews() {
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        let t0 = SystemTime::now();
        let (pinned, _) = ensure(&paths, &names(), t0).unwrap();
        assert!(renew_serving(&paths, &names(), &pinned.ca_fingerprint, t0)
            .unwrap()
            .is_none());
        let later = t0 + LEAF_VALIDITY - LEAF_RENEW_MARGIN + Duration::from_secs(3600);
        let renewed = renew_serving(&paths, &names(), &pinned.ca_fingerprint, later)
            .unwrap()
            .unwrap();
        assert_eq!(renewed.ca_fingerprint, pinned.ca_fingerprint);
    }

    #[test]
    fn rotation_changes_the_fingerprint() {
        let d = TempDir::new().unwrap();
        let paths = TrustPaths::in_dir(d.path());
        let now = SystemTime::now();
        let (a, _) = ensure(&paths, &names(), now).unwrap();
        let b = rotate_ca(&paths, &names(), now).unwrap();
        assert_ne!(a.ca_fingerprint, b.ca_fingerprint);
    }

    #[test]
    fn fingerprints_compare_regardless_of_how_they_were_typed() {
        assert_eq!(normalize_fingerprint("ab:cd ef"), "ABCDEF");
        assert_eq!(normalize_fingerprint("ABCDEF"), "ABCDEF");
    }

    #[test]
    fn relay_trust_files_are_not_remote_control_files() {
        let paths = TrustPaths::in_dir(Path::new("/d"));
        for p in paths.all() {
            let n = p.file_name().unwrap().to_str().unwrap();
            assert!(n.starts_with("relay-"), "{n}");
            assert_ne!(n, crate::remote::tls::CERT_FILENAME);
            assert_ne!(n, crate::remote::tls::KEY_FILENAME);
        }
    }
}

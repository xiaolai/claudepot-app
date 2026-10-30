//! The client half of pairing: fetch the server's CA, check it against a
//! fingerprint the user compared out of band, and only then talk over a
//! connection pinned to that CA.
//!
//! Two TLS configurations, never mixed:
//!
//! - [`fetch_ca`] accepts any certificate. It carries nothing secret
//!   either way (`GET /relay/ca` is public material), and its answer is
//!   worthless until the fingerprint check passes.
//! - Everything else goes through [`pinned_client`]: the server's CA is
//!   the only root, and the server name is checked against the leaf. A
//!   substituted chain fails the handshake, so no request — and no code,
//!   token or salt — is ever written to it.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use rustls_pki_types::pem::PemObject;

use super::secret::Secret;
use super::tls::{fingerprint_pem, normalize_fingerprint, provider};

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The CA the server presented is not the one the user approved.
    /// Raised before any secret is sent.
    #[error("the relay's CA fingerprint does not match the one you approved")]
    FingerprintMismatch,
    #[error("the relay's CA could not be read")]
    BadCa,
    #[error("could not reach the relay (or its certificate is not the approved one)")]
    Transport,
    #[error("the relay refused: {code} (HTTP {status})")]
    Refused { status: u16, code: String },
    #[error("the relay sent a response this client does not understand")]
    BadResponse,
    #[error("TLS configuration failed")]
    Tls,
    /// Health answered with liveness only: the token is unknown, revoked
    /// or not a relay token. The server does not say which.
    #[error("the relay did not accept this device's token")]
    NotPaired,
    /// No CA to pin, for a server that is not on loopback.
    #[error("refusing to probe a non-loopback relay without its pinned CA")]
    Unpinned,
}

impl ClientError {
    /// A stable code for surfaces that must not carry relay-supplied
    /// text: `Refused`'s `code` is whatever the server put in its error
    /// body, so it never leaves through this.
    pub fn code(&self) -> &'static str {
        match self {
            ClientError::FingerprintMismatch => "fingerprint_mismatch",
            ClientError::BadCa => "bad_ca",
            ClientError::Transport => "transport",
            ClientError::Refused { .. } => "refused",
            ClientError::BadResponse => "bad_response",
            ClientError::Tls => "tls",
            ClientError::NotPaired => "not_paired",
            ClientError::Unpinned => "unpinned",
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct FetchedCa {
    pub ca_pem: String,
    /// What the server *says* its fingerprint is. Never trusted:
    /// [`FetchedCa::computed_fingerprint`] is what gets compared.
    pub fingerprint: String,
    pub server_id: String,
}

impl FetchedCa {
    pub fn computed_fingerprint(&self) -> Result<String, ClientError> {
        fingerprint_pem(self.ca_pem.as_bytes()).ok_or(ClientError::BadCa)
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpstreamOffer {
    pub id: String,
    /// The target fingerprint under this client's salt.
    pub fingerprint: String,
    pub generation: u64,
}

pub struct Paired {
    pub device_id: String,
    pub token: Secret,
    pub server_id: String,
    pub default_upstream: Option<String>,
    pub upstreams: Vec<UpstreamOffer>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Acceptance {
    pub upstream: String,
    pub fingerprint: String,
    pub generation: u64,
    pub accepted: bool,
}

/// Accepts any certificate but still checks the handshake signatures,
/// so the session is a real TLS session with *someone*.
#[derive(Debug)]
struct AcceptAny(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn build(config: rustls::ClientConfig) -> Result<reqwest::Client, ClientError> {
    reqwest::Client::builder()
        .use_preconfigured_tls(config)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .build()
        .map_err(|_| ClientError::Tls)
}

/// A client whose only root is `ca_pem`. The URL's host is checked
/// against the leaf's names by the standard verifier.
pub fn pinned_client(ca_pem: &str) -> Result<reqwest::Client, ClientError> {
    let der = CertificateDer::from_pem_slice(ca_pem.as_bytes()).map_err(|_| ClientError::BadCa)?;
    let mut roots = rustls::RootCertStore::empty();
    roots.add(der).map_err(|_| ClientError::BadCa)?;
    let config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|_| ClientError::Tls)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    build(config)
}

/// `GET /relay/ca` over an unverified connection. Public material only.
pub async fn fetch_ca(base: &reqwest::Url) -> Result<FetchedCa, ClientError> {
    let p = provider();
    let config = rustls::ClientConfig::builder_with_provider(p.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| ClientError::Tls)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAny(p)))
        .with_no_client_auth();
    let client = build(config)?;
    let url = base
        .join("/relay/ca")
        .map_err(|_| ClientError::BadResponse)?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|_| ClientError::Transport)?;
    decode(resp).await
}

/// Whether `fetched`'s CA is the one the user approved. Compared on the
/// computed value, never on what the server claimed.
pub fn check_fingerprint(fetched: &FetchedCa, approved: &str) -> Result<(), ClientError> {
    if normalize_fingerprint(&fetched.computed_fingerprint()?) == normalize_fingerprint(approved) {
        Ok(())
    } else {
        Err(ClientError::FingerprintMismatch)
    }
}

/// Redeem a pairing code. The fingerprint is checked first; the code and
/// salt cross only the pinned connection.
pub async fn pair(
    base: &reqwest::Url,
    fetched: &FetchedCa,
    approved_fingerprint: &str,
    code: &str,
    device_name: &str,
    client_salt: &str,
) -> Result<Paired, ClientError> {
    check_fingerprint(fetched, approved_fingerprint)?;
    let client = pinned_client(&fetched.ca_pem)?;
    let url = base
        .join("/relay/pair")
        .map_err(|_| ClientError::BadResponse)?;
    let resp = client
        .post(url)
        .json(&serde_json::json!({
            "code": code,
            "device_name": device_name,
            "client_salt": client_salt,
        }))
        .send()
        .await
        .map_err(|_| ClientError::Transport)?;

    #[derive(serde::Deserialize)]
    struct Wire {
        device_id: String,
        token: String,
        server_id: String,
        #[serde(default)]
        default_upstream: Option<String>,
        #[serde(default)]
        upstreams: Vec<UpstreamOffer>,
    }
    let w: Wire = decode(resp).await?;
    if !w.token.starts_with(super::devices::TOKEN_PREFIX) {
        return Err(ClientError::BadResponse);
    }
    Ok(Paired {
        device_id: w.device_id,
        token: Secret::new(w.token),
        server_id: w.server_id,
        default_upstream: w.default_upstream,
        upstreams: w.upstreams,
    })
}

/// Ask for (`expected = None`) or record (`Some`) acceptance of an
/// upstream's current target, over the pinned connection.
pub async fn accept(
    base: &reqwest::Url,
    ca_pem: &str,
    token: &Secret,
    upstream: &str,
    expected: Option<&str>,
    client_salt: &str,
) -> Result<Acceptance, ClientError> {
    let client = pinned_client(ca_pem)?;
    let url = base
        .join("/relay/accept")
        .map_err(|_| ClientError::BadResponse)?;
    let resp = client
        .post(url)
        .bearer_auth(token.expose())
        .json(&serde_json::json!({
            "upstream": upstream,
            "expected_fingerprint": expected,
            "client_salt": client_salt,
        }))
        .send()
        .await
        .map_err(|_| ClientError::Transport)?;
    decode(resp).await
}

/// One health probe: a cold request and a warm one over the same
/// connection.
#[derive(Debug, Clone)]
pub struct Probe {
    /// The cold request, connection setup included (TCP, and TLS when
    /// pinned). Plan §7.2's connect tier.
    pub connect_ms: u64,
    /// The warm request on the now-open connection.
    pub rtt_ms: u64,
    pub health: super::health::HealthReport,
}

/// Probe `base`'s health as a paired device. A fresh client per call, so
/// the first request really is cold. `ca_pem: None` is accepted only for
/// a loopback server, which serves plain HTTP.
pub async fn probe(
    base: &reqwest::Url,
    ca_pem: Option<&str>,
    token: &Secret,
    client_salt: &str,
) -> Result<Probe, ClientError> {
    probe_within(base, ca_pem, token, client_salt, TIMEOUT).await
}

/// [`probe`] with each request bounded by `timeout`, so one stalled
/// server cannot hold up a probe of every paired server.
pub(crate) async fn probe_within(
    base: &reqwest::Url,
    ca_pem: Option<&str>,
    token: &Secret,
    client_salt: &str,
    timeout: std::time::Duration,
) -> Result<Probe, ClientError> {
    let client = match ca_pem {
        Some(pem) => pinned_client(pem)?,
        None => {
            let loopback = match base.host_str() {
                Some("localhost") => true,
                Some(h) => h
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback()),
                None => false,
            };
            if !loopback {
                return Err(ClientError::Unpinned);
            }
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| ClientError::Tls)?
        }
    };
    let url = base
        .join("/relay/health")
        .map_err(|_| ClientError::BadResponse)?;
    let get = || async {
        let started = std::time::Instant::now();
        let resp = client
            .get(url.clone())
            .bearer_auth(token.expose())
            .header(super::health::SALT_HEADER, client_salt)
            .timeout(timeout)
            .send()
            .await
            .map_err(|_| ClientError::Transport)?;
        let v: serde_json::Value = decode(resp).await?;
        Ok::<_, ClientError>((started.elapsed().as_millis() as u64, v))
    };
    let (connect_ms, _) = get().await?;
    let (rtt_ms, v) = get().await?;
    if v.get("server_id").is_none() {
        return Err(ClientError::NotPaired);
    }
    let health = serde_json::from_value(v).map_err(|_| ClientError::BadResponse)?;
    Ok(Probe {
        connect_ms,
        rtt_ms,
        health,
    })
}

/// The most a control response (pairing, acceptance, health) may carry.
/// They are small JSON objects; anything larger is not a relay.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

async fn decode<T: serde::de::DeserializeOwned>(
    mut resp: reqwest::Response,
) -> Result<T, ClientError> {
    let status = resp.status();
    if resp
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ClientError::BadResponse);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|_| ClientError::Transport)? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ClientError::BadResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let code = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(ClientError::Refused {
            status: status.as_u16(),
            code,
        });
    }
    serde_json::from_slice(&bytes).map_err(|_| ClientError::BadResponse)
}

/// A fresh 32-byte client salt, hex. `Uuid::new_v4` is the CSPRNG
/// `remote::token` already relies on.
pub fn new_client_salt() -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    hex::encode(bytes)
}

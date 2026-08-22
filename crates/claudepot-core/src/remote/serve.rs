//! Binding the socket and, when required, terminating TLS.
//!
//! Split from [`super::server`] so the router stays testable without a
//! port: everything about routing, middleware order and status codes is
//! exercised through `oneshot`, and only what genuinely needs a socket
//! lives here.
//!
//! ## TLS is decided by the address, not by configuration
//!
//! [`BindAddr::requires_tls`] is the whole policy: loopback is already
//! a secure context in every browser, so it needs no certificate and
//! loses no capability; anything else carries an admin password across
//! a wire someone else can be on.
//!
//! There is deliberately **no flag to serve a non-loopback address in
//! plaintext.** Missing TLS material stops the server. A downgrade
//! switch is what makes the guarantee meaningless — someone sets it "to
//! debug", it stays set, and the user goes on believing the traffic is
//! protected because nothing on screen ever says otherwise.
//!
//! ## The provider is `ring`, chosen not defaulted
//!
//! rustls 0.23 defaults to `aws-lc-rs`, which needs cmake and nasm to
//! build on Windows. `ring` is already in this tree and asks nothing of
//! the toolchain, so the cryptographic difference — none that matters
//! here — is not worth a broken Windows gate.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig as RustlsConfig;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::bind::BindAddr;
use super::config::ServerConfig;
use super::tls::{self, TlsMaterial, TlsMaterialError};

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("the remote surface is disabled; nothing was started")]
    Disabled,

    #[error(transparent)]
    Bind(#[from] super::bind::BindRefusal),

    #[error(transparent)]
    Tls(#[from] TlsMaterialError),

    #[error(
        "the TLS material is unusable: {0}. The remote surface will not \
         start — it does not fall back to plaintext."
    )]
    TlsInvalid(String),

    #[error("cannot listen on {addr}: {source}")]
    Listen {
        addr: SocketAddr,
        #[source]
        source: io::Error,
    },
}

/// What a caller needs to tell the user where to point a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listening {
    pub addr: SocketAddr,
    pub tls: bool,
}

impl Listening {
    /// Best-effort URL. The host is the bind address, which is right for
    /// an IP-reached appliance and wrong for one reached by name — the
    /// certificate's SAN is the authority there, so callers that know a
    /// name should prefer it.
    pub fn url(&self) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        format!("{scheme}://{}", self.addr)
    }
}

/// Build the rustls config from on-disk material.
///
/// Separate from [`serve`] so the failure modes are testable without
/// binding anything.
pub fn tls_config(material: &TlsMaterial) -> Result<RustlsConfig, ServeError> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&material.cert_pem)
        .collect::<Result<_, _>>()
        .map_err(|e| ServeError::TlsInvalid(format!("certificate: {e}")))?;
    if certs.is_empty() {
        return Err(ServeError::TlsInvalid(
            "the certificate file contains no certificates".into(),
        ));
    }

    let key = PrivateKeyDer::from_pem_slice(&material.key_pem)
        .map_err(|e| ServeError::TlsInvalid(format!("private key: {e}")))?;

    let mut config = RustlsConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| ServeError::TlsInvalid(e.to_string()))?;
    // The PWA is served over HTTP/1.1; advertising h2 without wiring it
    // would negotiate a protocol the connection handler does not speak.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

/// Resolve everything that can fail *before* a socket exists.
///
/// Returning the listener rather than serving from it lets a caller
/// report the bound port — useful when the configured port was taken —
/// and lets tests bind without accepting.
pub async fn listen(config: &ServerConfig) -> Result<(TcpListener, Listening), ServeError> {
    if !config.enabled {
        return Err(ServeError::Disabled);
    }
    let bind: BindAddr = config.checked_bind()?;
    let addr = SocketAddr::new(bind.addr(), config.port);

    // Certificate before socket: failing after the port is open leaves a
    // listener nobody can talk to, and a "port in use" error on the next
    // attempt.
    if bind.requires_tls() {
        let material = tls::load()?;
        tls_config(&material)?;
    }

    let listener = TcpListener::bind(addr)
        .await
        .map_err(|source| ServeError::Listen { addr, source })?;
    let local = listener.local_addr().unwrap_or(addr);
    Ok((
        listener,
        Listening {
            addr: local,
            tls: bind.requires_tls(),
        },
    ))
}

/// Serve until the process ends.
pub async fn serve(config: &ServerConfig, app: axum::Router) -> Result<(), ServeError> {
    let bind: BindAddr = config.checked_bind()?;
    let (listener, info) = listen(config).await?;
    tracing::info!(url = %info.url(), tls = info.tls, "remote surface listening");

    if !bind.requires_tls() {
        // Loopback: already a secure context, no certificate needed.
        let svc = app.into_make_service();
        return axum::serve(listener, svc)
            .await
            .map_err(|source| ServeError::Listen {
                addr: info.addr,
                source,
            });
    }

    let material = tls::load()?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config(&material)?));

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                // One bad accept must not end the server.
                tracing::warn!(error = %e, "remote: accept failed");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    // The common case is a plaintext request to a TLS
                    // port — a browser typed `http://`. Logged at debug
                    // because it is a user error, not a server fault,
                    // and it must never be answered in plaintext.
                    tracing::debug!(%peer, error = %e, "remote: TLS handshake failed");
                    return;
                }
            };
            // The Router is a tower Service; hyper wants its own trait.
            let svc = TowerToHyperService::new(app);
            let _ = ConnBuilder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls_stream), svc)
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn loopback(port: u16) -> ServerConfig {
        ServerConfig {
            enabled: true,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port,
            allowed_hosts: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_disabled_surface_does_not_listen() {
        let mut c = loopback(0);
        c.enabled = false;
        assert!(matches!(listen(&c).await, Err(ServeError::Disabled)));
    }

    #[tokio::test]
    async fn loopback_binds_without_any_certificate() {
        // Port 0 lets the OS pick, so the test never collides with a
        // real service.
        let (listener, info) = listen(&loopback(0)).await.unwrap();
        assert!(!info.tls, "loopback is already a secure context");
        assert!(info.url().starts_with("http://"));
        assert!(info.addr.port() > 0);
        drop(listener);
    }

    #[tokio::test]
    async fn a_public_bind_is_refused_before_a_socket_exists() {
        let c = ServerConfig {
            enabled: true,
            bind: "8.8.8.8".parse().unwrap(),
            port: 0,
            allowed_hosts: Vec::new(),
        };
        assert!(matches!(listen(&c).await, Err(ServeError::Bind(_))));
    }

    /// Not `#[tokio::test]`: the data-dir guard is a `std::sync`
    /// MutexGuard, and holding one across an `.await` is both a clippy
    /// error and a real deadlock hazard. Driving the future with
    /// `block_on` keeps the guard off any await point while still
    /// serialising the env-var mutation against other tests.
    #[test]
    fn a_non_loopback_bind_without_tls_material_refuses_to_start() {
        // The property that matters most in this module: no plaintext
        // fallback. 169.254.x is link-local, so it passes the bind
        // allowlist and requires TLS. Binding it would fail too — the
        // point is that the TLS check fires FIRST, before any socket
        // work happens.
        let _lock = crate::testing::lock_data_dir();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("CLAUDEPOT_DATA_DIR", dir.path());

        let c = ServerConfig {
            enabled: true,
            bind: "169.254.7.7".parse().unwrap(),
            port: 0,
            allowed_hosts: Vec::new(),
        };
        let err = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(listen(&c))
            .unwrap_err();
        assert!(
            matches!(err, ServeError::Tls(_)),
            "missing material must stop the server, never downgrade: {err:?}"
        );
    }

    #[test]
    fn a_certificate_file_with_no_certificate_is_rejected() {
        let material = TlsMaterial {
            cert_pem: b"-----BEGIN NOT A CERT-----\nxx\n-----END NOT A CERT-----\n".to_vec(),
            key_pem: b"-----BEGIN PRIVATE KEY-----\nxx\n-----END PRIVATE KEY-----\n".to_vec(),
            cert_path: "c.pem".into(),
            key_path: "k.pem".into(),
        };
        assert!(matches!(
            tls_config(&material),
            Err(ServeError::TlsInvalid(_))
        ));
    }

    #[test]
    fn the_url_scheme_follows_the_tls_decision() {
        let addr: SocketAddr = "127.0.0.1:8420".parse().unwrap();
        assert!(Listening { addr, tls: true }.url().starts_with("https://"));
        assert!(Listening { addr, tls: false }.url().starts_with("http://"));
    }
}

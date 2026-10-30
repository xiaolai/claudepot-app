//! Where a relay server may listen, and whether it must use TLS.
//!
//! The address allowlist is remote control's (`remote::bind::check`) —
//! the same function, not a copy, so the two surfaces cannot drift on what
//! "private" means. The refusal text is the relay's own, because
//! `BindRefusal`'s wording names the remote panel.

use std::net::IpAddr;

use crate::remote::bind::{self, BindAddr, BindRefusal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ListenError {
    #[error(
        "{addr} is publicly routable; the relay listens only on loopback, a private \
         network, link-local, the Tailscale range, or 0.0.0.0"
    )]
    PubliclyRoutable { addr: IpAddr },
    #[error("{addr} is not an address family the relay listens on")]
    UnsupportedFamily { addr: IpAddr },
    /// The relay always serves TLS, loopback included, with no switch to
    /// turn it off: a device token in plaintext is a stolen subscription,
    /// and one pairing path means loopback is not a second, weaker one.
    /// Plain HTTP exists only in tests, which call `server::serve` with
    /// no acceptor directly.
    #[error("the relay always serves TLS; plain HTTP on {addr} is refused")]
    TlsRequired { addr: IpAddr },
}

impl ListenError {
    pub fn code(&self) -> &'static str {
        match self {
            ListenError::PubliclyRoutable { .. } | ListenError::UnsupportedFamily { .. } => {
                "relay_bind_refused"
            }
            ListenError::TlsRequired { .. } => "relay_tls_required",
        }
    }
}

/// Validate a bind address and that TLS will be served on it. `tls:
/// false` is refused on every address: unlike remote control, the relay
/// has no plain-loopback mode (AGENTS.md, "Always TLS").
pub fn check(addr: IpAddr, tls: bool) -> Result<BindAddr, ListenError> {
    let b = bind::check(addr).map_err(|e| match e {
        BindRefusal::PubliclyRoutable { addr } => ListenError::PubliclyRoutable { addr },
        BindRefusal::UnsupportedFamily { addr } => ListenError::UnsupportedFamily { addr },
    })?;
    if !tls {
        return Err(ListenError::TlsRequired { addr });
    }
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn plain_http_is_refused_everywhere_loopback_included() {
        for a in [
            "127.0.0.1",
            "::1",
            "192.168.1.5",
            "100.64.0.9",
            "0.0.0.0",
            "169.254.1.1",
        ] {
            assert_eq!(
                check(ip(a), false),
                Err(ListenError::TlsRequired { addr: ip(a) }),
                "{a}"
            );
            assert!(check(ip(a), true).is_ok(), "{a}");
        }
    }

    #[test]
    fn a_public_address_is_refused_even_with_tls() {
        let e = check(ip("8.8.8.8"), true).unwrap_err();
        assert_eq!(e.code(), "relay_bind_refused");
        assert!(!e.to_string().contains("remote"));
    }
}

//! Where the remote surface is allowed to listen.
//!
//! This is the highest-consequence decision in the feature and it is
//! one line of configuration, so it gets its own module and a wall of
//! tests. The surface behind this socket can inject prompts into Claude
//! Code sessions, some running `bypassPermissions` — binding it to the
//! wrong interface is remote code execution offered to whoever is on
//! the same wifi.
//!
//! ## The allowlist, and why it is an allowlist
//!
//! Only two kinds of address are permitted:
//!
//! - **Loopback** — reachable by this machine only.
//! - **100.64.0.0/10** — the CGNAT range Tailscale assigns. Reachable
//!   only by peers already authenticated into the tailnet by WireGuard.
//!
//! Everything else is refused, including the machine's own LAN address.
//! A denylist ("not 0.0.0.0") would have to enumerate every way to say
//! "everyone", and misses the ones that matter: a LAN address is
//! specific, looks deliberate, and is exactly the mistake this guards
//! against. `0.0.0.0` at least looks alarming; `192.168.1.42` looks
//! like someone thought about it.
//!
//! ## What this does not do
//!
//! It does not confirm the address is *currently* assigned to a
//! Tailscale interface — that is a runtime property the bind call
//! itself will report by failing. The check here is about intent:
//! refuse a class of address that must never be used, before the socket
//! exists.

use std::net::{IpAddr, Ipv4Addr};

/// Tailscale's CGNAT range: 100.64.0.0/10 (RFC 6598).
const CGNAT_BASE: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 0);
const CGNAT_PREFIX_BITS: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BindRefusal {
    #[error(
        "refusing to bind {addr} — it accepts connections on every \
         interface, including the LAN. The remote surface can drive \
         Claude Code sessions; it may only listen on loopback or a \
         Tailscale address (100.64.0.0/10)."
    )]
    Unspecified { addr: IpAddr },

    #[error(
        "refusing to bind {addr} — anyone on that network segment could \
         reach it. The remote surface can drive Claude Code sessions; it \
         may only listen on loopback or a Tailscale address \
         (100.64.0.0/10)."
    )]
    NotPrivateToThisUser { addr: IpAddr },

    #[error(
        "refusing to bind {addr} — IPv6 is not supported for the remote \
         surface, because the Tailscale-range check that makes this safe \
         is only implemented for IPv4"
    )]
    UnsupportedFamily { addr: IpAddr },
}

/// An address that passed the allowlist. Constructing one is the only
/// way to name a bind target, so a caller cannot skip the check by
/// passing a bare `IpAddr` around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindAddr(IpAddr);

impl BindAddr {
    pub fn addr(self) -> IpAddr {
        self.0
    }

    /// True when only this machine can reach it.
    pub fn is_loopback(self) -> bool {
        self.0.is_loopback()
    }
}

impl std::fmt::Display for BindAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn is_tailscale_range(ip: Ipv4Addr) -> bool {
    let mask = u32::MAX << (32 - CGNAT_PREFIX_BITS);
    (u32::from(ip) & mask) == (u32::from(CGNAT_BASE) & mask)
}

/// The gate. Every path to a listening socket goes through here.
pub fn check(addr: IpAddr) -> Result<BindAddr, BindRefusal> {
    if addr.is_unspecified() {
        return Err(BindRefusal::Unspecified { addr });
    }
    if addr.is_loopback() {
        return Ok(BindAddr(addr));
    }
    match addr {
        IpAddr::V4(v4) if is_tailscale_range(v4) => Ok(BindAddr(addr)),
        IpAddr::V4(_) => Err(BindRefusal::NotPrivateToThisUser { addr }),
        // ::1 was already accepted as loopback above.
        IpAddr::V6(_) => Err(BindRefusal::UnsupportedFamily { addr }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn ip(s: &str) -> IpAddr {
        IpAddr::from_str(s).unwrap()
    }

    #[test]
    fn loopback_is_allowed() {
        assert!(check(ip("127.0.0.1")).is_ok());
        assert!(check(ip("127.0.0.53")).is_ok());
        assert!(check(ip("::1")).is_ok());
    }

    #[test]
    fn tailscale_addresses_are_allowed() {
        for a in ["100.64.0.1", "100.96.42.7", "100.127.255.254"] {
            assert!(check(ip(a)).is_ok(), "{a} is inside 100.64.0.0/10");
        }
    }

    #[test]
    fn every_way_of_saying_everyone_is_refused() {
        for a in ["0.0.0.0", "::"] {
            assert!(
                matches!(check(ip(a)), Err(BindRefusal::Unspecified { .. })),
                "{a} must be refused"
            );
        }
    }

    #[test]
    fn lan_addresses_are_refused() {
        // The mistake this module exists for. A LAN address looks
        // deliberate in a config file, which is precisely why it must
        // be rejected by an allowlist rather than a denylist.
        for a in [
            "192.168.1.42",
            "10.1.2.3",
            "172.16.5.5",
            "172.31.255.1",
            "169.254.1.1",
        ] {
            assert!(
                matches!(check(ip(a)), Err(BindRefusal::NotPrivateToThisUser { .. })),
                "{a} must be refused"
            );
        }
    }

    #[test]
    fn public_addresses_are_refused() {
        for a in ["1.1.1.1", "76.76.21.21", "8.8.8.8"] {
            assert!(check(ip(a)).is_err(), "{a} must be refused");
        }
    }

    #[test]
    fn the_cgnat_boundaries_are_exact() {
        // 100.64.0.0/10 spans 100.64.0.0 .. 100.127.255.255. The
        // neighbours on either side are ordinary public space, and an
        // off-by-one here would silently allow a routable address.
        assert!(is_tailscale_range(Ipv4Addr::new(100, 64, 0, 0)));
        assert!(is_tailscale_range(Ipv4Addr::new(100, 127, 255, 255)));
        assert!(!is_tailscale_range(Ipv4Addr::new(100, 63, 255, 255)));
        assert!(!is_tailscale_range(Ipv4Addr::new(100, 128, 0, 0)));
    }

    #[test]
    fn a_hundred_dot_address_outside_the_range_is_refused() {
        // 100.x is not automatically Tailscale — 100.0.0.0/10 and
        // 100.128.0.0/9 are ordinary public space. Matching on the
        // first octet alone would allow real internet addresses.
        assert!(check(ip("100.200.1.1")).is_err());
        assert!(check(ip("100.5.5.5")).is_err());
    }

    #[test]
    fn non_loopback_ipv6_is_refused_rather_than_guessed() {
        // Tailscale also assigns fd7a:115c:a1e0::/48, but the range
        // check here is IPv4-only. Refusing loudly beats accepting an
        // address this module cannot actually vet.
        assert!(matches!(
            check(ip("fd7a:115c:a1e0::1")),
            Err(BindRefusal::UnsupportedFamily { .. })
        ));
        assert!(check(ip("2001:4860:4860::8888")).is_err());
    }

    #[test]
    fn a_bind_addr_can_only_come_from_check() {
        // Compile-time property, asserted here as documentation: the
        // tuple field is private, so `BindAddr` cannot be constructed
        // outside this module and every listener must pass the gate.
        let ok = check(ip("100.96.42.7")).unwrap();
        assert_eq!(ok.addr(), ip("100.96.42.7"));
        assert!(!ok.is_loopback());
        assert!(check(ip("127.0.0.1")).unwrap().is_loopback());
    }

    #[test]
    fn refusal_messages_say_what_and_why() {
        let msg = check(ip("0.0.0.0")).unwrap_err().to_string();
        assert!(msg.contains("0.0.0.0"));
        assert!(
            msg.contains("100.64.0.0/10"),
            "the fix must be in the message"
        );
    }
}

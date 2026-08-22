//! Where the remote surface is allowed to listen.
//!
//! This is the highest-consequence decision in the feature and it is
//! one line of configuration, so it gets its own module and a wall of
//! tests. The surface behind this socket can inject prompts into Claude
//! Code sessions, some running `bypassPermissions` — binding it to the
//! wrong interface is remote code execution offered to whoever is on
//! the same wifi.
//!
//! ## The allowlist
//!
//! Claudepot is an appliance: reachable from other machines on the LAN,
//! over Tailscale or not, behind an admin password. So the permitted
//! set is broad — loopback, RFC1918 private space, link-local, and
//! Tailscale's CGNAT range — plus `0.0.0.0`, because "listen on every
//! interface" is what an appliance normally does and refusing it would
//! only push users to hardcode an address that changes with DHCP.
//!
//! What stays refused is any **globally routable** address. Binding one
//! puts a password prompt on the public internet, and behind that
//! password is arbitrary code execution on this machine. There is no
//! configuration in which that is what someone meant.
//!
//! `0.0.0.0` is accepted but reported as `Exposure::EveryInterface`,
//! because on a host that later acquires a public address it becomes a
//! public listener with no config change. The caller is expected to say
//! so rather than silently comply.
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
        "refusing to bind {addr} — it is reachable from the public \
         internet. Behind this surface's password is the ability to run \
         commands on this machine. Bind a private address (LAN, \
         Tailscale, or loopback), or 0.0.0.0 for every interface."
    )]
    PubliclyRoutable { addr: IpAddr },

    #[error(
        "refusing to bind {addr} — IPv6 is not supported for the remote \
         surface, because the private-range check that makes this safe is \
         only implemented for IPv4"
    )]
    UnsupportedFamily { addr: IpAddr },
}

/// How widely a permitted address listens. Returned rather than left to
/// be inferred, so a caller cannot forget that `0.0.0.0` is different.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exposure {
    /// This machine only.
    Loopback,
    /// One private network segment.
    PrivateNetwork,
    /// Every interface this host has, including any public one it
    /// acquires later. Warn.
    EveryInterface,
}

/// An address that passed the allowlist. Constructing one is the only
/// way to name a bind target, so a caller cannot skip the check by
/// passing a bare `IpAddr` around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindAddr(IpAddr, Exposure);

impl BindAddr {
    pub fn addr(self) -> IpAddr {
        self.0
    }

    pub fn exposure(self) -> Exposure {
        self.1
    }

    /// True when only this machine can reach it.
    pub fn is_loopback(self) -> bool {
        matches!(self.1, Exposure::Loopback)
    }

    /// TLS is mandatory unless only this machine can connect.
    ///
    /// `http://127.0.0.1` is a secure context in every browser, so
    /// loopback development needs no certificate and loses no browser
    /// capability. Every other exposure crosses a wire someone else can
    /// be on, carrying an admin password.
    pub fn requires_tls(self) -> bool {
        !self.is_loopback()
    }
}

impl std::fmt::Display for BindAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Private to some network: RFC1918, link-local, or Tailscale's CGNAT
/// range.
pub fn is_private_v4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local() || is_tailscale_range(ip)
}

pub fn is_tailscale_range(ip: Ipv4Addr) -> bool {
    let mask = u32::MAX << (32 - CGNAT_PREFIX_BITS);
    (u32::from(ip) & mask) == (u32::from(CGNAT_BASE) & mask)
}

/// The gate. Every path to a listening socket goes through here.
pub fn check(addr: IpAddr) -> Result<BindAddr, BindRefusal> {
    if addr.is_unspecified() {
        // Accepted, never silently: on a host with a public interface
        // this is a public listener.
        return Ok(BindAddr(addr, Exposure::EveryInterface));
    }
    if addr.is_loopback() {
        return Ok(BindAddr(addr, Exposure::Loopback));
    }
    match addr {
        IpAddr::V4(v4) if is_private_v4(v4) => Ok(BindAddr(addr, Exposure::PrivateNetwork)),
        IpAddr::V4(_) => Err(BindRefusal::PubliclyRoutable { addr }),
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
    fn loopback_is_allowed_and_needs_no_tls() {
        for a in ["127.0.0.1", "127.0.0.53", "::1"] {
            let b = check(ip(a)).unwrap();
            assert!(b.is_loopback(), "{a}");
            assert_eq!(b.exposure(), Exposure::Loopback);
            // http://127.0.0.1 is already a secure context.
            assert!(!b.requires_tls(), "{a} needs no certificate");
        }
    }

    #[test]
    fn lan_and_tailscale_addresses_are_allowed_and_require_tls() {
        for a in [
            "192.168.1.42",
            "10.1.2.3",
            "172.16.5.5",
            "172.31.255.1",
            "169.254.1.1",
            "100.64.0.1",
            "100.96.42.7",
            "100.127.255.254",
        ] {
            let b = check(ip(a)).unwrap();
            assert_eq!(b.exposure(), Exposure::PrivateNetwork, "{a}");
            assert!(
                b.requires_tls(),
                "{a} carries an admin password across a wire someone else can be on"
            );
        }
    }

    #[test]
    fn every_interface_is_allowed_but_flagged() {
        // An appliance normally does this. It is permitted, but the
        // caller is told, because on a host that later gets a public
        // address it silently becomes a public listener.
        for a in ["0.0.0.0", "::"] {
            let b = check(ip(a)).unwrap();
            assert_eq!(b.exposure(), Exposure::EveryInterface, "{a}");
            assert!(b.requires_tls());
            assert!(!b.is_loopback());
        }
    }

    #[test]
    fn publicly_routable_addresses_are_refused() {
        // The one thing no configuration could have meant: a password
        // prompt on the internet, with code execution behind it.
        for a in ["1.1.1.1", "76.76.21.21", "8.8.8.8", "203.0.113.7"] {
            assert!(
                matches!(check(ip(a)), Err(BindRefusal::PubliclyRoutable { .. })),
                "{a} must be refused"
            );
        }
    }

    #[test]
    fn the_cgnat_boundaries_are_exact() {
        // 100.64.0.0/10 spans 100.64.0.0 ..= 100.127.255.255. The
        // neighbours are ordinary public space; an off-by-one would
        // silently allow a routable address.
        assert!(is_tailscale_range(Ipv4Addr::new(100, 64, 0, 0)));
        assert!(is_tailscale_range(Ipv4Addr::new(100, 127, 255, 255)));
        assert!(!is_tailscale_range(Ipv4Addr::new(100, 63, 255, 255)));
        assert!(!is_tailscale_range(Ipv4Addr::new(100, 128, 0, 0)));
    }

    #[test]
    fn a_hundred_dot_address_outside_the_range_is_still_public() {
        // 100.x is not automatically Tailscale: 100.0.0.0/10 and
        // 100.128.0.0/9 are ordinary public space.
        assert!(check(ip("100.200.1.1")).is_err());
        assert!(check(ip("100.5.5.5")).is_err());
    }

    #[test]
    fn non_loopback_ipv6_is_refused_rather_than_guessed() {
        // Tailscale also assigns fd7a:115c:a1e0::/48 and RFC4193 has
        // fc00::/7, but the private-range check here is IPv4-only.
        // Refusing loudly beats accepting an address this cannot vet.
        assert!(matches!(
            check(ip("fd7a:115c:a1e0::1")),
            Err(BindRefusal::UnsupportedFamily { .. })
        ));
        assert!(check(ip("2001:4860:4860::8888")).is_err());
    }

    #[test]
    fn a_bind_addr_can_only_come_from_check() {
        // Compile-time property, asserted here as documentation: the
        // fields are private, so every listener passes the gate.
        let ok = check(ip("100.96.42.7")).unwrap();
        assert_eq!(ok.addr(), ip("100.96.42.7"));
    }

    #[test]
    fn tls_is_required_for_everything_except_loopback() {
        // The invariant the server depends on, stated once.
        for a in ["0.0.0.0", "192.168.1.42", "100.96.42.7", "10.1.2.3"] {
            assert!(check(ip(a)).unwrap().requires_tls(), "{a}");
        }
        for a in ["127.0.0.1", "::1"] {
            assert!(!check(ip(a)).unwrap().requires_tls(), "{a}");
        }
    }

    #[test]
    fn refusal_messages_say_what_and_how_to_fix_it() {
        let msg = check(ip("8.8.8.8")).unwrap_err().to_string();
        assert!(msg.contains("8.8.8.8"));
        assert!(msg.contains("0.0.0.0"), "the remedy must be in the message");
    }
}

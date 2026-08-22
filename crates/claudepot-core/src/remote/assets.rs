//! The client, embedded in the binary.
//!
//! `include_str!` rather than serving a directory. Three reasons, and
//! the first is the one that matters:
//!
//! - **No path-traversal surface.** A static-file handler has to defend
//!   against `..`, symlinks, and percent-encoding tricks. A fixed match
//!   over five known paths has nothing to defend.
//! - **No missing-file-at-runtime.** An appliance that starts and then
//!   404s its own UI because a directory was not shipped is worse than
//!   one that fails to build.
//! - **No new dependency.** `tower-http`'s `ServeDir` would be one.
//!
//! ## What this client is
//!
//! A probe, not the product. Its job is to answer questions that cannot
//! be answered from a development machine: does this browser treat a
//! privately-trusted certificate on a `.internal` name as a **secure
//! context**, will it install as a PWA, and is a platform authenticator
//! available for a future passkey login. It reports browser
//! capabilities and nothing about the host.

/// One embedded file: its body and the Content-Type it must be served
/// with.
pub struct Asset {
    pub body: &'static str,
    pub content_type: &'static str,
}

/// Resolve a request path to an embedded asset.
///
/// Exhaustive match, no fallthrough to a filesystem lookup. An unknown
/// path is `None` and the caller 404s.
pub fn get(path: &str) -> Option<Asset> {
    let (body, content_type) = match path {
        "/" | "/index.html" => (
            include_str!("assets/index.html"),
            "text/html; charset=utf-8",
        ),
        "/app.js" => (
            include_str!("assets/app.js"),
            "text/javascript; charset=utf-8",
        ),
        // Must be served from the root path or its scope cannot cover
        // the app; a worker at /static/sw.js controls only /static.
        "/sw.js" => (
            include_str!("assets/sw.js"),
            "text/javascript; charset=utf-8",
        ),
        // The dedicated type matters: some browsers ignore a manifest
        // served as application/json.
        "/manifest.webmanifest" => (
            include_str!("assets/manifest.webmanifest"),
            "application/manifest+json",
        ),
        "/icon.svg" => (include_str!("assets/icon.svg"), "image/svg+xml"),
        _ => return None,
    };
    Some(Asset { body, content_type })
}

/// `Cache-Control` for every asset.
///
/// `no-store`, not merely `no-cache`. A service worker that outlives a
/// change to itself is the classic way a PWA becomes unupdatable, and
/// while this build caches nothing, the header is the habit to start
/// with rather than the one to remember to add later.
pub const CACHE_CONTROL: &str = "no-store";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_the_html_references_is_embedded() {
        // The failure this prevents: shipping an index.html that asks
        // for a file nobody added to `get`, which 404s only on a real
        // device.
        let html = get("/").unwrap().body;
        for path in ["/app.js", "/manifest.webmanifest", "/icon.svg"] {
            assert!(
                html.contains(path),
                "index.html no longer references {path} — is it still needed?"
            );
            assert!(get(path).is_some(), "{path} is referenced but not embedded");
        }
        // Registered by app.js rather than referenced by the HTML.
        assert!(get("/sw.js").is_some());
        assert!(get("/app.js").unwrap().body.contains("/sw.js"));
    }

    #[test]
    fn unknown_paths_resolve_to_nothing() {
        for p in [
            "/nope",
            "/../Cargo.toml",
            "/assets/index.html",
            "",
            "/index.html/",
        ] {
            assert!(get(p).is_none(), "{p:?} must not resolve");
        }
    }

    #[test]
    fn traversal_shapes_cannot_reach_anything() {
        // There is no filesystem lookup to attack, but assert it anyway
        // so a future refactor to ServeDir has to delete this test on
        // purpose rather than by accident.
        for p in [
            "/../../etc/passwd",
            "/..%2f..%2fetc%2fpasswd",
            "/./app.js/../../secret",
        ] {
            assert!(get(p).is_none(), "{p:?} must not resolve");
        }
    }

    #[test]
    fn content_types_are_the_ones_browsers_require() {
        assert_eq!(
            get("/manifest.webmanifest").unwrap().content_type,
            "application/manifest+json",
            "some browsers ignore a manifest served as application/json"
        );
        assert!(get("/sw.js")
            .unwrap()
            .content_type
            .starts_with("text/javascript"));
        assert!(get("/").unwrap().content_type.starts_with("text/html"));
        assert_eq!(get("/icon.svg").unwrap().content_type, "image/svg+xml");
    }

    #[test]
    fn the_probe_reports_the_three_unknowns() {
        // This client exists to answer exactly these. If a refactor
        // drops one, the trip to the phone was wasted.
        let js = get("/app.js").unwrap().body;
        assert!(
            js.contains("isSecureContext"),
            "secure context is the whole question"
        );
        assert!(
            js.contains("serviceWorker.register"),
            "registration is the real test"
        );
        assert!(
            js.contains("isUserVerifyingPlatformAuthenticatorAvailable"),
            "the passkey question is answered by the same trip"
        );
    }

    #[test]
    fn the_client_makes_no_third_party_requests() {
        // A probe that phones out would be a strange thing to install on
        // someone's phone, and the CSP would break it anyway.
        //
        // `xmlns="http://www.w3.org/2000/svg"` is exempt: an XML
        // namespace is an identifier, not a URL — no browser resolves
        // it. The exemption is narrow and named rather than a blanket
        // "ignore http in svg", so a real remote reference in the icon
        // would still fail.
        const XML_NS: &str = "http://www.w3.org/2000/svg";
        for path in [
            "/",
            "/app.js",
            "/sw.js",
            "/manifest.webmanifest",
            "/icon.svg",
        ] {
            let body = get(path).unwrap().body.replace(XML_NS, "");
            for scheme in ["http://", "https://", "//cdn", "//unpkg"] {
                assert!(
                    !body.contains(scheme),
                    "{path} references a remote origin ({scheme})"
                );
            }
        }
    }
}

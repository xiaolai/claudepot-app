//! The client, embedded in the binary.
//!
//! `include_str!` / `include_bytes!` rather than serving a directory.
//! Three reasons, and the first is the one that matters:
//!
//! - **No path-traversal surface.** A static-file handler has to defend
//!   against `..`, symlinks, and percent-encoding tricks. A fixed match
//!   over known paths has nothing to defend.
//! - **No missing-file-at-runtime.** An appliance that starts and then
//!   404s its own UI because a directory was not shipped is worse than
//!   one that fails to build.
//! - **No new dependency.** `tower-http`'s `ServeDir` would be one.
//!
//! ## Two clients, and why both stay
//!
//! - **The panel**, at `/`. The product: sessions, threads, one-tap
//!   answers. Built from `panel/` by Vite into `assets/panel/` and
//!   committed, so `cargo build` needs no Node.
//! - **The probe**, at `/probe`. A capability readout — secure context,
//!   service worker, platform authenticator. It is not the product and
//!   never was, but it has twice been the only thing that could answer a
//!   question a development machine cannot: it is what established that
//!   a privately-trusted certificate on a tailnet name yields a full
//!   secure context, and what caught the passkey check measuring the
//!   device rather than the origin. Keeping it reachable costs one route
//!   and two files.
//!
//! ## Lazily-loaded chunks
//!
//! The panel dynamically imports mermaid, which splits itself into ~60
//! diagram packs. They are served from `/panel/chunks/` through a
//! **generated** table (`panel_chunks.rs`), because sixty hand-written
//! arms would be wrong within one upgrade of mermaid — and because a
//! directory walk would give this module the traversal surface it exists
//! not to have.
//!
//! The cost is stated rather than buried: those chunks are ~3.4 MB of
//! committed bytes, embedded in every binary whether or not the remote
//! surface is ever switched on. They buy per-diagram loading — a
//! flowchart pulls ~60 KB, not the whole of mermaid — on a client that
//! is a phone on a LAN and is served `no-store`. The base bundle is
//! unchanged at ~413 KB: a thread with no diagram in it pays nothing.
//!
//! ## Fonts
//!
//! Three families, self-hosted, latin + latin-ext. The prototype pulled
//! them from Google Fonts, which fails twice over here: an appliance on a
//! tailnet has no route to `fonts.gstatic.com`, and a private surface
//! that beacons a third party on every load is not private. See
//! `panel/src/fonts.css` for the subsetting decisions.

mod panel_chunks;

/// Prefix for the lazily-loaded panel chunks. See `panel_chunks`.
const CHUNK_PREFIX: &str = "/panel/chunks/";

/// One embedded file: its bytes, the Content-Type it must be served
/// with, and how long a client may keep it.
pub struct Asset {
    pub body: &'static [u8],
    pub content_type: &'static str,
    pub cache_control: &'static str,
}

impl Asset {
    /// The body as text. Panics on a non-UTF-8 asset, which is what the
    /// tests want — every text asset here is authored, not user data.
    #[cfg(test)]
    fn text(&self) -> &'static str {
        std::str::from_utf8(self.body).expect("text asset is not UTF-8")
    }
}

/// `Cache-Control` for everything that is not a font.
///
/// `no-store`, not merely `no-cache`. A service worker that outlives a
/// change to itself is the classic way a PWA becomes unupdatable, and
/// while this build caches nothing, the header is the habit to start
/// with rather than the one to remember to add later. It also covers
/// `panel.js`, which carries no content hash and therefore must never be
/// held across a release.
pub const CACHE_CONTROL: &str = "no-store";

/// `Cache-Control` for the fonts, and only the fonts.
///
/// 107 KB re-downloaded on every page load is a real cost on a phone
/// over a LAN, and these bytes cannot go stale: a font file's name here
/// carries its family and subset, so different bytes mean a different
/// name. Nothing else on this surface has that property, which is why
/// the exception is one line rather than a policy.
pub const FONT_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

const HTML: &str = "text/html; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const WOFF2: &str = "font/woff2";

/// Resolve a request path to an embedded asset.
///
/// Exhaustive match, no fallthrough to a filesystem lookup. An unknown
/// path is `None` and the caller 404s.
pub fn get(path: &str) -> Option<Asset> {
    // Chunks first: there are sixty of them and they are generated, so
    // they cannot live in the match below without that file becoming
    // unreadable.
    if let Some(name) = path.strip_prefix(CHUNK_PREFIX) {
        // The generated match is exhaustive, so a traversal attempt
        // simply misses — but a name containing a separator would mean
        // the caller and this function disagree about what a chunk name
        // is, and that is worth refusing rather than probing.
        if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
            return None;
        }
        return panel_chunks::chunk(name).map(|body| Asset {
            body,
            content_type: JS,
            cache_control: CACHE_CONTROL,
        });
    }

    let (body, content_type, cache_control): (&'static [u8], _, _) = match path {
        // ── The panel ────────────────────────────────────────────────
        "/" | "/index.html" => (
            include_bytes!("assets/panel/index.html"),
            HTML,
            CACHE_CONTROL,
        ),
        "/panel/panel.js" => (include_bytes!("assets/panel/panel.js"), JS, CACHE_CONTROL),
        "/panel/panel.css" => (include_bytes!("assets/panel/panel.css"), CSS, CACHE_CONTROL),

        "/panel/fonts/InstrumentSans-latin.woff2" => (
            include_bytes!("assets/panel/fonts/InstrumentSans-latin.woff2"),
            WOFF2,
            FONT_CACHE_CONTROL,
        ),
        "/panel/fonts/InstrumentSans-latin-ext.woff2" => (
            include_bytes!("assets/panel/fonts/InstrumentSans-latin-ext.woff2"),
            WOFF2,
            FONT_CACHE_CONTROL,
        ),
        "/panel/fonts/InstrumentSerif-latin.woff2" => (
            include_bytes!("assets/panel/fonts/InstrumentSerif-latin.woff2"),
            WOFF2,
            FONT_CACHE_CONTROL,
        ),
        "/panel/fonts/InstrumentSerif-latin-ext.woff2" => (
            include_bytes!("assets/panel/fonts/InstrumentSerif-latin-ext.woff2"),
            WOFF2,
            FONT_CACHE_CONTROL,
        ),
        "/panel/fonts/JetBrainsMono-latin.woff2" => (
            include_bytes!("assets/panel/fonts/JetBrainsMono-latin.woff2"),
            WOFF2,
            FONT_CACHE_CONTROL,
        ),
        "/panel/fonts/JetBrainsMono-latin-ext.woff2" => (
            include_bytes!("assets/panel/fonts/JetBrainsMono-latin-ext.woff2"),
            WOFF2,
            FONT_CACHE_CONTROL,
        ),

        // ── The probe ────────────────────────────────────────────────
        "/probe" | "/probe.html" => (include_bytes!("assets/probe.html"), HTML, CACHE_CONTROL),
        "/probe.js" => (include_bytes!("assets/probe.js"), JS, CACHE_CONTROL),

        // ── Shared shell ─────────────────────────────────────────────
        // Must be served from the root path or its scope cannot cover
        // the app; a worker at /panel/sw.js controls only /panel.
        "/sw.js" => (include_bytes!("assets/sw.js"), JS, CACHE_CONTROL),
        // The dedicated type matters: some browsers ignore a manifest
        // served as application/json.
        "/manifest.webmanifest" => (
            include_bytes!("assets/manifest.webmanifest"),
            "application/manifest+json",
            CACHE_CONTROL,
        ),
        "/icon.svg" => (
            include_bytes!("assets/icon.svg"),
            "image/svg+xml",
            CACHE_CONTROL,
        ),
        _ => return None,
    };
    Some(Asset {
        body,
        content_type,
        cache_control,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every path `get` answers. The tests iterate this rather than
    /// their own copies — a list that has to be kept in sync with the
    /// match is a second source of truth waiting to disagree with it.
    const EVERY_PATH: &[&str] = &[
        "/",
        "/panel/panel.js",
        "/panel/panel.css",
        "/panel/fonts/InstrumentSans-latin.woff2",
        "/panel/fonts/InstrumentSans-latin-ext.woff2",
        "/panel/fonts/InstrumentSerif-latin.woff2",
        "/panel/fonts/InstrumentSerif-latin-ext.woff2",
        "/panel/fonts/JetBrainsMono-latin.woff2",
        "/panel/fonts/JetBrainsMono-latin-ext.woff2",
        "/probe",
        "/probe.js",
        "/sw.js",
        "/manifest.webmanifest",
        "/icon.svg",
    ];

    #[test]
    fn every_route_the_html_references_is_embedded() {
        // The failure this prevents: shipping an index.html that asks
        // for a file nobody added to `get`, which 404s only on a real
        // device.
        for html_path in ["/", "/probe"] {
            let html = get(html_path).unwrap().text();
            for line in html.lines() {
                for attr in ["src=\"", "href=\""] {
                    let Some(rest) = line.split(attr).nth(1) else {
                        continue;
                    };
                    let Some(target) = rest.split('"').next() else {
                        continue;
                    };
                    // Only root-absolute paths are ours to serve.
                    if !target.starts_with('/') {
                        continue;
                    }
                    assert!(
                        get(target).is_some(),
                        "{html_path} references {target}, which is not embedded"
                    );
                }
            }
        }
    }

    #[test]
    fn the_panel_bundle_is_reachable_from_its_own_html() {
        // Vite writes the script and stylesheet tags. If its output
        // naming ever drifts from `assets.rs`'s literals, this is the
        // test that says so at build time rather than on a phone.
        let html = get("/").unwrap().text();
        assert!(
            html.contains("/panel/panel.js"),
            "panel html lost its bundle"
        );
        assert!(
            html.contains("/panel/panel.css"),
            "panel html lost its stylesheet"
        );
    }

    #[test]
    fn every_font_the_stylesheet_asks_for_is_embedded() {
        let css = get("/panel/panel.css").unwrap().text();
        let mut found = 0;
        for chunk in css.split("url(") {
            let Some(url) = chunk.split(')').next() else {
                continue;
            };
            let url = url.trim_matches(['"', '\'']);
            if !url.ends_with(".woff2") {
                continue;
            }
            found += 1;
            assert!(get(url).is_some(), "{url} is referenced but not embedded");
        }
        assert_eq!(found, 6, "the stylesheet no longer asks for six subsets");
    }

    #[test]
    fn unknown_paths_resolve_to_nothing() {
        for p in [
            "/nope",
            "/../Cargo.toml",
            "/assets/index.html",
            "",
            "/index.html/",
            "/panel/",
            "/panel/fonts/",
            "/app.js",
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
            "/panel/../../../Cargo.toml",
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
        assert!(get("/probe").unwrap().content_type.starts_with("text/html"));
        assert_eq!(get("/icon.svg").unwrap().content_type, "image/svg+xml");
        assert_eq!(
            get("/panel/panel.css").unwrap().content_type,
            "text/css; charset=utf-8"
        );
        assert_eq!(
            get("/panel/fonts/JetBrainsMono-latin.woff2")
                .unwrap()
                .content_type,
            "font/woff2"
        );
    }

    #[test]
    fn only_fonts_are_cacheable() {
        for p in EVERY_PATH {
            let a = get(p).unwrap();
            let want = if p.ends_with(".woff2") {
                FONT_CACHE_CONTROL
            } else {
                CACHE_CONTROL
            };
            assert_eq!(a.cache_control, want, "{p} has the wrong cache policy");
        }
    }

    #[test]
    fn the_probe_reports_the_three_unknowns() {
        // This client exists to answer exactly these. If a refactor
        // drops one, the trip to the phone was wasted.
        let js = get("/probe.js").unwrap().text();
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
    fn the_panel_registers_the_service_worker() {
        // PWA install and web push both hang off a worker at root
        // scope. The panel is the client that will want them.
        assert!(get("/panel/panel.js").unwrap().text().contains("/sw.js"));
    }

    /// Off-origin-looking strings that are **not** requests, each named
    /// with the reason it cannot be one.
    ///
    /// A blanket "ignore anything in a bundle" would defeat the check
    /// entirely. Every entry here is an **exact literal**, and
    /// `every_exemption_is_still_needed` asserts each one still occurs —
    /// so an exemption cannot outlive the dependency that earned it,
    /// the same discipline `verify_docs`'s `UNSUBSCRIBED_BY_DESIGN`
    /// applies to event channels.
    const NOT_A_REQUEST: &[(&str, &str)] = &[
        (
            "http://www.w3.org/2000/svg",
            "XML namespace: an identifier, never resolved",
        ),
        ("http://www.w3.org/1999/xlink", "XML namespace"),
        ("http://www.w3.org/1998/Math/MathML", "XML namespace"),
        ("http://www.w3.org/XML/1998/namespace", "XML namespace"),
        (
            "https://react.dev/errors/",
            "React interpolates this into a thrown Error's message; nothing fetches it",
        ),
        (
            "https://github.com/syntax-tree/hast-util-to-jsx-runtime",
            "text of an exception react-markdown throws on a removed option",
        ),
        (
            "https://github.com/remarkjs/react-markdown/blob/main/changelog.md",
            "same: a pointer printed in an error message",
        ),
        (
            "\"http://\"",
            "remark-gfm's autolink scheme prefix. It is prepended to text found in the \
             TRANSCRIPT (`www.example.com` becomes a link), so it builds a URL the user may \
             tap — it is never a URL this bundle fetches. Exempt in its quoted, standalone \
             form only: a real remote reference always has a host after the slashes, so it \
             cannot match this literal",
        ),
    ];

    #[test]
    fn the_client_makes_no_third_party_requests() {
        // A client that phones out would be a strange thing to install
        // on someone's phone, and the CSP would break it anyway.
        //
        // **The lazily-loaded chunks are deliberately out of scope**,
        // and saying so is the point of this paragraph. They are ~3.4 MB
        // of vendored third-party code — mermaid, cytoscape, katex,
        // chevrotain — whose error strings alone name a dozen
        // documentation URLs. Asserting over them would need an
        // exemption list long enough to defeat the check, which is worse
        // than not running it there.
        //
        // What guards them instead is the CSP, and it is the stronger
        // guarantee anyway: `default-src 'self'; connect-src 'self';
        // font-src 'self'` blocks a *request*, where this test only ever
        // saw a *string*. Inspected once by hand at the time mermaid
        // landed: no chunk reaches the network (katex's `fetch()` is its
        // parser's token reader, not `window.fetch`), and every external
        // URL sits in an error message. If a chunk ever did try, the
        // browser would refuse it and the diagram would fail visibly.
        for path in EVERY_PATH {
            if path.ends_with(".woff2") {
                continue; // binary; no URLs inside a woff2 we serve
            }
            let mut body = get(path).unwrap().text().to_string();
            for (literal, _why) in NOT_A_REQUEST {
                body = body.replace(literal, "");
            }
            for scheme in ["http://", "https://", "//cdn", "//unpkg", "//fonts."] {
                if let Some(at) = body.find(scheme) {
                    // Show what was found. A bare "references a remote
                    // origin" leaves the next person grepping a 400 KB
                    // minified bundle by hand.
                    //
                    // Windowed by CHARACTER, not by byte. `body[a..b]`
                    // on a minified bundle panics the moment an offset
                    // lands inside a multi-byte sequence — and it would
                    // do that while building a panic message, replacing
                    // a useful failure with a confusing one.
                    let window: String = body[at..]
                        .chars()
                        .take(140)
                        .filter(|c| !c.is_control())
                        .collect();
                    panic!("{path} references a remote origin ({scheme}) near: {window}…");
                }
            }
        }
    }

    /// The chunk directory, resolved from the crate root so the test
    /// reads the same files `include_bytes!` compiled in.
    fn chunk_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/remote/assets/panel/chunks")
    }

    #[test]
    fn every_chunk_on_disk_is_reachable() {
        // Direction one: a chunk Vite emitted that nobody routed is a
        // diagram that 404s on a phone and nowhere else.
        let dir = chunk_dir();
        let entries: Vec<String> = std::fs::read_dir(&dir)
            .expect("chunk directory — run scripts/build-panel.sh")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".js"))
            .collect();

        assert!(
            entries.len() > 20,
            "only {} chunks on disk — did the build emit them?",
            entries.len()
        );
        for name in &entries {
            assert!(
                get(&format!("{CHUNK_PREFIX}{name}")).is_some(),
                "{name} is on disk but not routed — regenerate with scripts/build-panel.sh"
            );
        }
    }

    #[test]
    fn every_routed_chunk_is_on_disk() {
        // Direction two: an arm whose file is gone would not compile, so
        // this is really about the table being regenerated from a build
        // that produced *these* files rather than an older set. Parsing
        // the generated source is the only way to enumerate the arms.
        let src = include_str!("assets/panel_chunks.rs");
        let dir = chunk_dir();
        let mut routed = 0;
        for line in src.lines() {
            let Some(rest) = line.trim().strip_prefix('"') else {
                continue;
            };
            let Some((name, _)) = rest.split_once('"') else {
                continue;
            };
            routed += 1;
            assert!(
                dir.join(name).exists(),
                "{name} is routed but not on disk — regenerate with scripts/build-panel.sh"
            );
        }
        assert!(
            routed > 20,
            "the generated table lists only {routed} chunks"
        );
    }

    #[test]
    fn a_chunk_name_cannot_escape_its_directory() {
        for name in [
            "../panel.js",
            "..%2Fpanel.js",
            "sub/dir.js",
            "sub\\dir.js",
            "",
        ] {
            assert!(
                get(&format!("{CHUNK_PREFIX}{name}")).is_none(),
                "{name:?} must not resolve"
            );
        }
    }

    #[test]
    fn every_exemption_is_still_needed() {
        // An exemption that no longer matches anything is a hole waiting
        // for a real URL to fall into. Dependencies come and go; the
        // list has to shrink with them.
        let all: String = EVERY_PATH
            .iter()
            .filter(|p| !p.ends_with(".woff2"))
            .map(|p| get(p).unwrap().text())
            .collect::<Vec<_>>()
            .join("\n");
        for (literal, why) in NOT_A_REQUEST {
            assert!(
                all.contains(literal),
                "the exemption for {literal:?} ({why}) no longer matches anything — delete it"
            );
        }
    }

    #[test]
    fn the_panel_is_at_the_root_and_the_probe_is_not() {
        // The decision recorded in the plan: the panel replaces the
        // probe as the landing page, and the probe stays reachable.
        assert!(get("/").unwrap().text().contains("/panel/panel.js"));
        assert!(get("/probe").unwrap().text().contains("Connectivity check"));
    }
}

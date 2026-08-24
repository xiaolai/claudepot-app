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

/// One file: its bytes, the Content-Type it must be served with, and
/// how long a client may keep it.
///
/// `Cow` rather than `&'static [u8]` because a **debug** build can be
/// told to serve the panel from disk instead of from the binary — see
/// [`dev_panel_dir`]. A release build only ever produces `Borrowed`.
pub struct Asset {
    pub body: std::borrow::Cow<'static, [u8]>,
    pub content_type: &'static str,
    pub cache_control: &'static str,
}

impl Asset {
    /// The body as text. Panics on a non-UTF-8 asset, which is what the
    /// tests want — every text asset here is authored, not user data.
    #[cfg(test)]
    fn text(&self) -> String {
        String::from_utf8(self.body.to_vec()).expect("text asset is not UTF-8")
    }
}

/// Where to read the panel from instead of the binary, if anywhere.
///
/// **Debug builds only.** A release binary returns `None` whatever the
/// environment says, so a shipped appliance cannot be pointed at a
/// directory by anyone who can set a variable on it.
///
/// This exists because the edit-to-see loop was `pnpm build` →
/// `build-panel.sh` → `cargo build` → restart the server, and the
/// `cargo build` and the restart are pure overhead for a CSS tweak.
/// Vite already writes straight into the embedded directory, so those
/// bytes are on disk the moment the build finishes; this just reads
/// them.
///
/// **It does not widen what can be served.** The exhaustive match still
/// runs first and decides whether a path exists at all — this only
/// changes where an already-approved path's bytes come from. That is
/// the whole point: `remote::assets` exists so there is no runtime
/// directory walk and therefore no traversal surface, and a dev
/// convenience that reintroduced one would be a bad trade at any speed.
fn dev_panel_dir() -> Option<std::path::PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var_os("CLAUDEPOT_PANEL_DIR").map(std::path::PathBuf::from)
}

/// The on-disk name for a request path the match already approved.
///
/// Only the panel's own files: fonts and the probe do not change while
/// someone is iterating on a screen, and every path not named here
/// falls back to the embedded bytes.
fn dev_relative_path(path: &str) -> Option<&str> {
    match path {
        "/" | "/index.html" => Some("index.html"),
        _ => path.strip_prefix("/panel/"),
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
const PNG: &str = "image/png";
const WOFF2: &str = "font/woff2";

/// Resolve a request path to an embedded asset.
///
/// Exhaustive match, no fallthrough to a filesystem lookup. An unknown
/// path is `None` and the caller 404s.
pub fn get(path: &str) -> Option<Asset> {
    get_from(path, dev_panel_dir().as_deref())
}

/// The body of [`get`], with the dev directory passed in.
///
/// Split out so the tests never touch `CLAUDEPOT_PANEL_DIR`. A test
/// that mutates a process global is a test that fails when the suite
/// runs in parallel — and this one took the service-worker assertion
/// down with it, because that test read `panel.js` while another had
/// the override pointed at a temp directory. Second time this session;
/// the fix is not a lock, it is not reading a global from code under
/// test.
fn get_from(path: &str, dev: Option<&std::path::Path>) -> Option<Asset> {
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
        return panel_chunks::chunk(name).map(|body| {
            with_dev_override(
                path,
                dev,
                Asset {
                    body: std::borrow::Cow::Borrowed(body),
                    content_type: JS,
                    cache_control: CACHE_CONTROL,
                },
            )
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
        // iOS accepts PNG only for `apple-touch-icon` — an SVG there is
        // ignored outright and the home screen shows a tile generated
        // from the first letter of the title. See panel/index.html.
        "/apple-touch-icon.png" => (
            include_bytes!("assets/apple-touch-icon.png"),
            PNG,
            CACHE_CONTROL,
        ),
        // The manifest's raster icons. Android and desktop PWA installers
        // read these; the SVG beside them is the browser-tab favicon.
        "/icon-192.png" => (include_bytes!("assets/icon-192.png"), PNG, CACHE_CONTROL),
        "/icon-512.png" => (include_bytes!("assets/icon-512.png"), PNG, CACHE_CONTROL),
        _ => return None,
    };
    Some(with_dev_override(
        path,
        dev,
        Asset {
            body: std::borrow::Cow::Borrowed(body),
            content_type,
            cache_control,
        },
    ))
}

/// Swap in the bytes on disk for a path the match already approved.
///
/// Falls back to the embedded asset on any failure — a half-written
/// file mid-build, a directory that does not exist, a typo in the
/// variable. A dev convenience that could 500 the app is worse than one
/// that occasionally serves a stale byte.
fn with_dev_override(path: &str, dev: Option<&std::path::Path>, embedded: Asset) -> Asset {
    let Some(dir) = dev else {
        return embedded;
    };
    let Some(rel) = dev_relative_path(path) else {
        return embedded;
    };
    match std::fs::read(dir.join(rel)) {
        Ok(bytes) => Asset {
            body: std::borrow::Cow::Owned(bytes),
            ..embedded
        },
        Err(_) => embedded,
    }
}

#[cfg(test)]
mod tests {

    /// The mapping, without touching the environment.
    ///
    /// The security property is that this is only ever consulted for a
    /// path the exhaustive match has ALREADY approved — so what matters
    /// here is that it never invents a name outside the panel.
    #[test]
    fn the_dev_mapping_covers_the_panel_and_nothing_else() {
        assert_eq!(dev_relative_path("/"), Some("index.html"));
        assert_eq!(dev_relative_path("/index.html"), Some("index.html"));
        assert_eq!(dev_relative_path("/panel/panel.js"), Some("panel.js"));
        assert_eq!(
            dev_relative_path("/panel/chunks/Mermaid.js"),
            Some("chunks/Mermaid.js")
        );
        // Not the panel: fonts and the probe do not change while
        // someone is iterating on a screen.
        assert_eq!(dev_relative_path("/sw.js"), None);
        assert_eq!(dev_relative_path("/probe"), None);
        assert_eq!(dev_relative_path("/api/health"), None);
    }

    /// **The override cannot widen what is servable.**
    ///
    /// The whole reason `remote::assets` is an exhaustive match is to
    /// have no runtime directory walk and therefore no traversal
    /// surface. A dev convenience that reintroduced one would be a bad
    /// trade at any speed, so the match runs first and this asserts it
    /// still refuses everything it refused before — with the override
    /// pointed at a directory that really does contain the file.
    #[test]
    fn a_path_the_match_refuses_stays_refused_under_the_override() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret.txt"), b"should never be served").unwrap();
        std::fs::create_dir_all(dir.path().join("chunks")).unwrap();
        std::fs::write(dir.path().join("chunks/evil.js"), b"nope").unwrap();

        for path in [
            "/panel/secret.txt",
            "/panel/chunks/evil.js",
            "/panel/../secret.txt",
            "/panel/chunks/../secret.txt",
            "/../../etc/passwd",
            "/panel/",
        ] {
            assert!(
                get_from(path, Some(dir.path())).is_none(),
                "{path} must stay unservable"
            );
        }
    }

    #[test]
    fn the_override_serves_disk_bytes_for_an_approved_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("panel.js"), b"// from disk").unwrap();

        let got = get_from("/panel/panel.js", Some(dir.path())).expect("still an approved path");
        assert_eq!(got.text(), "// from disk");
        // And the headers are the embedded asset's — only the bytes move.
        assert_eq!(got.content_type, JS);
        assert_eq!(got.cache_control, CACHE_CONTROL);
    }

    #[test]
    fn a_missing_file_falls_back_to_the_embedded_bytes() {
        // A half-written file mid-build must not 500 the app.
        let dir = tempfile::tempdir().unwrap();

        let got = get_from("/panel/panel.js", Some(dir.path())).expect("embedded fallback");
        assert!(!got.body.is_empty());
        assert!(
            got.text().len() > 1000,
            "that is the real bundle, not an empty read"
        );
    }

    #[test]
    fn without_the_variable_nothing_changes() {
        assert!(matches!(
            get_from("/panel/panel.js", None).unwrap().body,
            std::borrow::Cow::Borrowed(_)
        ));
    }
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
        "/apple-touch-icon.png",
        "/icon-192.png",
        "/icon-512.png",
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

    /// Assets whose bytes are not text.
    ///
    /// Three tests decode every asset as UTF-8 to look for URLs, and
    /// each one used to name `.woff2` itself. Adding the home-screen
    /// PNGs meant editing all three, and missing one is a panic that
    /// reads as "text asset is not UTF-8" from a test whose subject is
    /// third-party requests — which is what happened. One predicate, so
    /// the next binary asset is one edit.
    fn is_binary(path: &str) -> bool {
        path.ends_with(".woff2") || path.ends_with(".png")
    }

    /// iOS accepts PNG only for `apple-touch-icon`.
    ///
    /// Pointed at an SVG it ignores the tag and generates a tile from
    /// the first letter of the title — which is exactly what shipped:
    /// adding the panel to an iPhone home screen produced a plain "C".
    /// Nothing failed anywhere. The asset was served, the HTML was
    /// valid, `every_route_the_html_references_is_embedded` was green
    /// because the SVG *was* embedded; the format was simply one iOS
    /// declines to read.
    ///
    /// Asserted on the magic bytes rather than the extension, because a
    /// `.png` containing SVG would satisfy the filename and fail on the
    /// device, which is the same failure with a different disguise.
    #[test]
    fn the_home_screen_icon_is_a_real_png() {
        let html = get("/").unwrap().text();
        let at = html
            .find("apple-touch-icon")
            .expect("index.html declares no apple-touch-icon");
        let tail = &html[at..];
        let href_at = tail.find("href=\"").expect("apple-touch-icon has no href");
        let href: String = tail[href_at + 6..]
            .chars()
            .take_while(|c| *c != '"')
            .collect();
        assert!(
            href.ends_with(".png"),
            "apple-touch-icon points at {href}, which iOS ignores — it accepts PNG only"
        );

        let asset = get(&href).unwrap_or_else(|| panic!("{href} is referenced but not embedded"));
        assert_eq!(asset.content_type, PNG);
        assert_eq!(
            &asset.body[..8],
            b"\x89PNG\r\n\x1a\n",
            "{href} is not actually a PNG"
        );
    }

    /// The manifest's raster icons exist too. An installer that cannot
    /// fetch them falls back to whatever the platform invents, which is
    /// the same class of silent wrong icon.
    #[test]
    fn every_manifest_icon_is_embedded() {
        let manifest = get("/manifest.webmanifest").unwrap().text();
        let mut found = 0;
        for chunk in manifest.split("\"src\": \"").skip(1) {
            let src: String = chunk.chars().take_while(|c| *c != '"').collect();
            assert!(
                get(&src).is_some(),
                "the manifest lists {src}, which is not embedded"
            );
            found += 1;
        }
        assert!(
            found >= 2,
            "expected the manifest to list icons, found {found}"
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
            if is_binary(path) {
                continue; // no URLs inside bytes we do not decode
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
            .filter(|p| !is_binary(p))
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

use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use crate::routes::{AuthScheme, GatewayConfig, VertexConfig};

#[derive(Default)]
struct FakeKeychain {
    items: Mutex<HashMap<RouteId, String>>,
    locked: bool,
}

impl KeychainReader for FakeKeychain {
    fn read(&self, route: RouteId) -> Result<Option<Secret>, RouteSecretError> {
        if self.locked {
            return Err(RouteSecretError::Locked);
        }
        Ok(self
            .items
            .lock()
            .unwrap()
            .get(&route)
            .cloned()
            .map(Secret::new))
    }
}

fn id() -> RouteId {
    uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
}

fn gateway(use_keychain: bool, api_key: &str) -> Route {
    let mut r: Route = serde_json::from_value(serde_json::json!({
        "id": id(),
        "name": "DeepSeek",
        "provider": {"kind": "gateway", "base_url": "https://api.deepseek.com/anthropic", "api_key": ""},
        "model": "deepseek-flash",
        "wrapper_name": "claude-deepseek",
        "deployment_organization_uuid": id(),
    }))
    .unwrap();
    r.provider = RouteProvider::Gateway(GatewayConfig {
        base_url: "https://api.deepseek.com/anthropic".into(),
        api_key: api_key.into(),
        auth_scheme: AuthScheme::Bearer,
        enable_tool_search: false,
        use_keychain,
    });
    r
}

fn keychain_with(value: &str) -> FakeKeychain {
    let k = FakeKeychain::default();
    k.items.lock().unwrap().insert(id(), value.into());
    k
}

#[test]
fn a_keychain_route_serves_the_keychain_value() {
    let route = gateway(true, "");
    let got = read_with(&route, &keychain_with("sk-kc")).unwrap().unwrap();
    assert_eq!(got.expose(), "sk-kc");
}

#[test]
fn a_rotation_is_what_the_next_read_sees() {
    // Edits write the same cell the server reads, so there is no second
    // copy to fall out of date.
    let route = gateway(true, "");
    let k = keychain_with("sk-old");
    k.items.lock().unwrap().insert(id(), "sk-rotated".into());
    assert_eq!(
        read_with(&route, &k).unwrap().unwrap().expose(),
        "sk-rotated"
    );
}

#[test]
fn a_keychain_route_ignores_a_stale_plaintext_field() {
    // `routes.json` may still hold the key from before the user opted into
    // the keychain; the keychain is authoritative.
    let route = gateway(true, "sk-stale-plain");
    assert_eq!(
        read_with(&route, &keychain_with("sk-kc"))
            .unwrap()
            .unwrap()
            .expose(),
        "sk-kc"
    );
}

#[test]
fn no_keychain_entry_is_missing_not_an_empty_key() {
    let route = gateway(true, "");
    assert_eq!(
        read_with(&route, &FakeKeychain::default()).unwrap_err(),
        RouteSecretError::Missing(id())
    );
    assert_eq!(
        read_with(&route, &keychain_with("")).unwrap_err(),
        RouteSecretError::Missing(id())
    );
}

#[test]
fn a_locked_keychain_is_reported_not_skipped() {
    let locked = FakeKeychain {
        locked: true,
        ..Default::default()
    };
    assert_eq!(
        read_with(&gateway(true, "sk-stale-plain"), &locked).unwrap_err(),
        RouteSecretError::Locked
    );
}

#[test]
fn plaintext_routes_serve_their_own_key_without_the_keychain() {
    let locked = FakeKeychain {
        locked: true,
        ..Default::default()
    };
    let route = gateway(false, "sk-plain");
    assert_eq!(
        read_with(&route, &locked).unwrap().unwrap().expose(),
        "sk-plain"
    );
    // A local server's empty key sends no credential, rather than an
    // empty header.
    assert!(read_with(&gateway(false, ""), &locked).unwrap().is_none());
}

#[test]
fn the_production_path_serves_and_checks_a_plaintext_route() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let route = gateway(false, "sk-plain");
    rt.block_on(check_for_publish(&route)).unwrap();
    assert_eq!(
        rt.block_on(read_for_serving(&route))
            .unwrap()
            .unwrap()
            .expose(),
        "sk-plain"
    );
}

#[test]
fn only_gateway_routes_are_relay_capable() {
    let mut route = gateway(false, "k");
    route.provider = RouteProvider::Vertex(VertexConfig {
        project_id: "p".into(),
        region: None,
        base_url: None,
        skip_gcp_auth: false,
    });
    assert_eq!(
        read_with(&route, &FakeKeychain::default()).unwrap_err(),
        RouteSecretError::NotRelayCapable(id())
    );
}

#[test]
fn errors_never_carry_the_secret() {
    for e in [
        RouteSecretError::Locked,
        RouteSecretError::Missing(id()),
        RouteSecretError::NotRelayCapable(id()),
        RouteSecretError::Backend("x".into()),
    ] {
        assert!(!e.to_string().contains("sk-"));
    }
}

/// A `routes.json` holding `route`, in a fresh temp dir.
fn routes_file_with(route: &Route) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("routes.json");
    RouteStore::open_at(file.clone())
        .unwrap()
        .add(route.clone())
        .unwrap();
    (dir, file)
}

fn rewrite(file: &Path, route: Route) {
    RouteStore::open_at(file.to_owned())
        .unwrap()
        .update(route)
        .unwrap();
}

#[test]
fn a_plaintext_key_rotated_in_routes_json_is_what_the_next_read_sees() {
    let approved = gateway(false, "sk-old");
    let (_dir, file) = routes_file_with(&approved);
    let k = FakeKeychain::default();
    assert_eq!(
        read_current(&approved, Some(&file), &k)
            .unwrap()
            .unwrap()
            .expose(),
        "sk-old"
    );
    rewrite(&file, gateway(false, "sk-rotated"));
    assert_eq!(
        read_current(&approved, Some(&file), &k)
            .unwrap()
            .unwrap()
            .expose(),
        "sk-rotated"
    );
}

#[test]
fn a_route_re_targeted_or_moved_to_the_keychain_is_changed_not_served() {
    let approved = gateway(false, "sk-old");
    let (_dir, file) = routes_file_with(&approved);
    let k = keychain_with("sk-kc");

    let mut elsewhere = gateway(false, "sk-new");
    if let RouteProvider::Gateway(cfg) = &mut elsewhere.provider {
        cfg.base_url = "https://attacker.invalid/anthropic".into();
    }
    rewrite(&file, elsewhere);
    assert_eq!(
        read_current(&approved, Some(&file), &k).unwrap_err(),
        RouteSecretError::Changed(id())
    );

    rewrite(&file, gateway(true, ""));
    assert_eq!(
        read_current(&approved, Some(&file), &k).unwrap_err(),
        RouteSecretError::Changed(id())
    );
}

#[test]
fn a_route_removed_from_routes_json_is_changed() {
    let approved = gateway(false, "sk-old");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("routes.json");
    std::fs::write(&file, b"").unwrap();
    assert_eq!(
        read_current(&approved, Some(&file), &FakeKeychain::default()).unwrap_err(),
        RouteSecretError::Changed(id())
    );
}

#[test]
fn an_unparseable_routes_json_is_reported_without_its_contents() {
    let approved = gateway(false, "sk-old");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("routes.json");
    std::fs::write(&file, b"{\"routes\": [sk-leaked").unwrap();
    let e = read_current(&approved, Some(&file), &FakeKeychain::default()).unwrap_err();
    assert_eq!(e, RouteSecretError::RoutesFile);
    assert!(!e.to_string().contains("sk-"));
}

#[test]
fn without_a_routes_file_the_approved_snapshot_is_read() {
    let approved = gateway(false, "sk-snap");
    assert_eq!(
        read_current(&approved, None, &FakeKeychain::default())
            .unwrap()
            .unwrap()
            .expose(),
        "sk-snap"
    );
}

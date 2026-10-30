//! Real credential-store round-trip for `relay::token_store` —
//! `#[ignore]` by default per `.claude/rules/rust-conventions.md`.
//!
//! On macOS this exercises the `/usr/bin/security` verified-write
//! pattern against the login keychain; elsewhere, `keyring`. Run on the
//! test host (see AGENTS.md "Test"):
//!
//! ```bash
//! cargo test -p claudepot-core --test relay_token_store -- --ignored
//! ```
//!
//! Items live under a test-only service with a fresh UUID account per
//! test, and each test deletes what it created.

use claudepot_core::relay::secret::Secret;
use claudepot_core::relay::token_store;

const SERVICE: &str = "claudepot-relay-device-test";

#[tokio::test]
#[ignore]
async fn a_token_round_trips_and_deletes_idempotently() {
    let account = uuid::Uuid::new_v4().to_string();
    assert!(token_store::read(SERVICE, &account)
        .await
        .unwrap()
        .is_none());

    let token = Secret::new(format!("cprl_{}", "ab".repeat(32)));
    token_store::write(SERVICE, &account, &token).await.unwrap();
    let back = token_store::read(SERVICE, &account).await.unwrap().unwrap();
    assert!(back == token);

    // An overwrite replaces, it does not duplicate.
    let next = Secret::new(format!("cprl_{}", "cd".repeat(32)));
    token_store::write(SERVICE, &account, &next).await.unwrap();
    assert!(token_store::read(SERVICE, &account).await.unwrap().unwrap() == next);

    token_store::delete(SERVICE, &account).await.unwrap();
    token_store::delete(SERVICE, &account).await.unwrap();
    assert!(token_store::read(SERVICE, &account)
        .await
        .unwrap()
        .is_none());
}

/// A keychain route as the relay serves it (plan §5.5): a secret stored
/// through `routes::keychain` lands in the real keychain under the
/// service the wrapper's helper script reads, a separate process (the
/// `security` CLI) can read it, the relay's serving path reads it, and a
/// rotation is what the next read sees. Before 2026-10 the store went
/// through `keyring`'s in-memory mock and this test failed at the second
/// assertion. Uses a fresh route id and removes the item afterwards.
#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore]
async fn a_keychain_route_secret_reaches_the_keychain_and_the_relay() {
    use claudepot_core::relay::route_secret;
    use claudepot_core::routes::{
        delete_keychain_secret, keychain_account, read_keychain_secret, store_keychain_secret,
        AuthScheme, GatewayConfig, Route, RouteProvider, SecretField, KEYCHAIN_SERVICE,
    };

    let id = uuid::Uuid::new_v4();
    let field = SecretField::GatewayApiKey;
    let mut route: Route = serde_json::from_value(serde_json::json!({
        "id": id,
        "name": "keychain test",
        "provider": {"kind": "gateway", "base_url": "https://example.invalid", "api_key": ""},
        "model": "m",
        "wrapper_name": "claude-keychain-test",
        "deployment_organization_uuid": uuid::Uuid::new_v4(),
    }))
    .unwrap();
    route.provider = RouteProvider::Gateway(GatewayConfig {
        base_url: "https://example.invalid".into(),
        api_key: String::new(),
        auth_scheme: AuthScheme::Bearer,
        enable_tool_search: false,
        use_keychain: true,
    });

    let result = async {
        store_keychain_secret(id, field, "sk-test-before").unwrap();
        // Another process: what the wrapper's helper script runs.
        let out = std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a"])
            .arg(keychain_account(id, field))
            .arg("-w")
            .output()
            .unwrap();
        assert!(out.status.success(), "item not in the keychain");
        assert!(out.stdout.trim_ascii_end() == b"sk-test-before");

        route_secret::check_for_publish(&route).await.unwrap();
        let served = route_secret::read_for_serving(&route)
            .await
            .unwrap()
            .unwrap();
        assert!(served == Secret::new("sk-test-before"));

        store_keychain_secret(id, field, "sk-test-after").unwrap();
        let served = route_secret::read_for_serving(&route)
            .await
            .unwrap()
            .unwrap();
        assert!(served == Secret::new("sk-test-after"));
        assert!(read_keychain_secret(id, field).unwrap().as_deref() == Some("sk-test-after"));

        delete_keychain_secret(id, field).unwrap();
        assert_eq!(
            route_secret::read_for_serving(&route).await.unwrap_err(),
            route_secret::RouteSecretError::Missing(id)
        );
    };
    let outcome = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(result)).await;
    delete_keychain_secret(id, field).unwrap();
    if let Err(p) = outcome {
        std::panic::resume_unwind(p);
    }
}

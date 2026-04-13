//! Verify the keyring backend actually persists credentials.
//! Must be run as a SIGNED binary on macOS for keyring access.
//!
//!   cargo build --release --example keyring-probe
//!   scripts/sign-macos.sh target/release/examples/keyring-probe
//!   CLAUDEPOT_CREDENTIAL_BACKEND=keyring target/release/examples/keyring-probe

use claudepot_core::cli_backend::swap::{delete_private, load_private, save_private};
use uuid::Uuid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Fixed test UUID so repeat runs clean up properly.
    let id = Uuid::parse_str("aaaaaaaa-1234-4567-8901-cccccccccccc")?;
    let blob = r#"{"probe":"keyring-roundtrip"}"#;

    println!(
        "backend env: {:?}",
        std::env::var("CLAUDEPOT_CREDENTIAL_BACKEND")
    );
    println!("test uuid:   {id}");

    println!("1. save_private...");
    save_private(id, blob)?;

    println!("2. load_private...");
    let loaded = load_private(id)?;
    println!("   loaded: {loaded}");
    assert_eq!(loaded, blob, "roundtrip mismatch");

    println!("3. verify in real Keychain via `security find-generic-password`...");
    let out = std::process::Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            &id.to_string(),
            "-s",
            "com.claudepot.credentials",
            "-w",
        ])
        .output()?;
    let in_keychain = out.status.success();
    println!(
        "   in keychain: {} (stdout: {:?})",
        in_keychain,
        String::from_utf8_lossy(&out.stdout).trim()
    );

    println!("4. delete_private + cleanup");
    delete_private(id)?;

    println!();
    if in_keychain {
        println!("✓ keyring backend is functional — credentials persist to the real Keychain");
    } else {
        println!(
            "⚠ blob roundtripped via save_private/load_private, but not visible in `security`"
        );
        println!("  → probably fell back to file storage (sign the binary to enable Keychain)");
    }
    Ok(())
}

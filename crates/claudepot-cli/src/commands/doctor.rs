use anyhow::Result;
use crate::AppContext;

pub async fn run(ctx: &AppContext) -> Result<()> {
    println!("Claudepot v{} — Health Check\n", env!("CARGO_PKG_VERSION"));

    let mut warnings = 0u32;
    let mut errors = 0u32;

    // Platform
    println!("  Platform:     {} ({})", std::env::consts::OS, std::env::consts::ARCH);

    // Data dir
    let data_dir = claudepot_core::paths::claudepot_data_dir();
    if data_dir.exists() {
        ok("Data dir", &data_dir.display().to_string());
    } else {
        warn("Data dir", &format!("{} (does not exist)", data_dir.display()));
        warnings += 1;
    }

    // Accounts DB
    match ctx.store.list() {
        Ok(accounts) => ok("Accounts", &format!("{} registered", accounts.len())),
        Err(e) => { err("Accounts", &e.to_string()); errors += 1; }
    }

    // Claude CLI
    let claude_paths = [
        dirs::home_dir().map(|h| h.join(".local/bin/claude")),
        Some(std::path::PathBuf::from("/usr/local/bin/claude")),
        Some(std::path::PathBuf::from("/usr/bin/claude")),
    ];
    let mut cli_found = false;
    for path in claude_paths.iter().flatten() {
        if path.exists() {
            let version = std::process::Command::new(path)
                .arg("--version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_else(|| "?".to_string());
            ok("Claude CLI", &format!("{} ({})", path.display(), version.trim()));
            cli_found = true;
            break;
        }
    }
    if !cli_found {
        warn("Claude CLI", "not found"); warnings += 1;
    }

    // Claude Desktop
    #[cfg(target_os = "macos")]
    {
        let desktop_path = std::path::Path::new("/Applications/Claude.app");
        if desktop_path.exists() {
            let version = std::process::Command::new("defaults")
                .args(["read", "/Applications/Claude.app/Contents/Info.plist", "CFBundleShortVersionString"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_else(|| "?".to_string());
            ok("Claude Desktop", &format!("v{}", version.trim()));
        } else {
            warn("Claude Desktop", "not installed"); warnings += 1;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        println!("  Claude Desktop: (check skipped on {})", std::env::consts::OS);
    }

    // Keychain access (macOS)
    #[cfg(target_os = "macos")]
    {
        match claudepot_core::cli_backend::keychain::read_default().await {
            Ok(Some(_)) => ok("Keychain", "Claude Code-credentials readable"),
            Ok(None) => { warn("Keychain", "Claude Code-credentials empty (not logged in)"); warnings += 1; },
            Err(e) => { err("Keychain", &e.to_string()); errors += 1; },
        }
    }

    // Beta header
    let beta = claudepot_core::oauth::beta_header::get_or_default();
    ok("Beta header", beta);

    // API reachability
    match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap()
        .get("https://api.anthropic.com/api/oauth/profile")
        .header("Authorization", "Bearer test")
        .header("anthropic-beta", beta)
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if status == 401 {
                ok("API reachable", "api.anthropic.com (401 = reachable, token rejected as expected)");
            } else if status == 403 {
                err("API blocked", "403 Forbidden — likely geo-restricted. Use HTTPS_PROXY.");
                errors += 1;
            } else {
                ok("API reachable", &format!("api.anthropic.com (HTTP {})", status));
            }
        }
        Err(e) => { err("API", &format!("unreachable: {e}")); errors += 1; }
    }

    // Per-account health
    println!();
    let accounts = ctx.store.list().unwrap_or_default();
    if !accounts.is_empty() {
        println!("  Account health:");
        for a in &accounts {
            let cred = claudepot_core::cli_backend::swap::load_private(a.uuid);
            match cred {
                Ok(blob_str) => {
                    match claudepot_core::blob::CredentialBlob::from_json(&blob_str) {
                        Ok(blob) => {
                            let remaining = (blob.claude_ai_oauth.expires_at
                                - chrono::Utc::now().timestamp_millis()) / 60_000;
                            if remaining > 0 {
                                println!("    {}  ✓ token valid ({}m)", a.email, remaining);
                            } else {
                                println!("    {}  ✗ token expired", a.email);
                                warnings += 1;
                            }
                        }
                        Err(_) => {
                            println!("    {}  ✗ corrupt credential blob", a.email);
                            errors += 1;
                        }
                    }
                }
                Err(_) => {
                    println!("    {}  ✗ no stored credentials", a.email);
                    warnings += 1;
                }
            }
        }
    }

    // Desktop profiles
    let profile_base = claudepot_core::paths::claudepot_data_dir().join("desktop");
    if profile_base.exists() {
        println!("\n  Desktop profiles:");
        for a in &accounts {
            let p = claudepot_core::paths::desktop_profile_dir(a.uuid);
            if p.exists() {
                let count = std::fs::read_dir(&p).map(|d| d.count()).unwrap_or(0);
                println!("    {}  ✓ {} items", a.email, count);
            } else {
                println!("    {}  — no profile", a.email);
            }
        }
    }

    // Summary
    println!();
    if errors > 0 {
        println!("{} error(s), {} warning(s).", errors, warnings);
    } else if warnings > 0 {
        println!("All checks passed ({} warning(s)).", warnings);
    } else {
        println!("All checks passed.");
    }

    Ok(())
}

fn ok(label: &str, detail: &str) {
    println!("  {:<16}✓ {}", label, detail);
}

fn warn(label: &str, detail: &str) {
    println!("  {:<16}⚠ {}", label, detail);
}

fn err(label: &str, detail: &str) {
    println!("  {:<16}✗ {}", label, detail);
}

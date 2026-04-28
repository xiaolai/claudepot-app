use crate::error::DesktopSwapError;
use once_cell::sync::Lazy;
use std::path::PathBuf;
use std::sync::RwLock;

pub struct WindowsDesktop;

/// Known-good MSIX package family fallback. Anthropic's current
/// shipped Desktop uses this exact name; we fall back to it when
/// runtime AppX discovery fails (no PowerShell, non-elevated,
/// foreign shell). Treat it as a compile-time constant that can be
/// overridden at runtime by `discover_package_family_cached`.
const KNOWN_PACKAGE_FAMILY: &str = "Claude_pzs8sxrjxfjjc";

/// Cached result of the MSIX package-family discovery. Populated on
/// first call to [`package_family`] per process. RwLock over Option
/// so the fast path is a read-only lookup after the first probe.
static DISCOVERED_FAMILY: Lazy<RwLock<Option<String>>> = Lazy::new(|| RwLock::new(None));

/// Return the MSIX package family name for Claude Desktop. Discovery
/// algorithm (runs at most once per process):
///
/// 1. Query `Get-AppxPackage -Name "Claude*"` via PowerShell and
///    parse the `PackageFamilyName` — the authoritative answer.
/// 2. If PowerShell fails, fall back to scanning
///    `%LocalAppData%\Packages\Claude_*` for any matching directory.
/// 3. If that also fails, return the compile-time
///    [`KNOWN_PACKAGE_FAMILY`] constant — preserves today's hard-coded
///    behavior on systems where both probes fail (guest users,
///    locked-down SKUs without PowerShell).
///
/// Cached in `DISCOVERED_FAMILY` after the first successful call.
/// Subsequent calls are a RwLock read — sub-microsecond.
pub fn package_family() -> String {
    if let Some(cached) = DISCOVERED_FAMILY.read().ok().and_then(|g| g.clone()) {
        return cached;
    }
    let discovered = discover_package_family().unwrap_or_else(|| {
        tracing::warn!(
            "AppX discovery failed — falling back to known package family `{KNOWN_PACKAGE_FAMILY}`"
        );
        KNOWN_PACKAGE_FAMILY.to_string()
    });
    if let Ok(mut guard) = DISCOVERED_FAMILY.write() {
        *guard = Some(discovered.clone());
    }
    discovered
}

#[cfg(target_os = "windows")]
fn discover_package_family() -> Option<String> {
    // 1. PowerShell probe — authoritative. Use the fully qualified
    // System32 path (derived from %SystemRoot%) rather than the
    // unqualified `powershell` so a hijacked PATH or a dropped
    // `powershell.exe` in CWD can't run here. Falls back to the
    // filesystem scan if PowerShell is unavailable.
    let system_root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("C:\\Windows"));
    let ps_path = system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let ps = std::process::Command::new(&ps_path)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            // Filter on PackageFamilyName (Claude_*) rather than
            // a permissive Name='Claude*' wildcard, so ambient
            // installs like "ClaudeCompanion" or "ClaudeSync" can't
            // be selected instead of Claude Desktop itself.
            "(Get-AppxPackage | Where-Object { $_.PackageFamilyName -like 'Claude_*' } | Select-Object -First 1).PackageFamilyName",
        ])
        .output();
    if let Ok(out) = ps {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Some(name) = parse_package_family_output(&stdout) {
                return Some(name);
            }
        }
    }
    // 2. Filesystem fallback — scan the Packages dir.
    let packages = dirs::data_local_dir()?.join("Packages");
    let entries = std::fs::read_dir(&packages).ok()?;
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with("Claude_") && entry.path().is_dir() {
                return Some(name.to_string());
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn discover_package_family() -> Option<String> {
    Some(KNOWN_PACKAGE_FAMILY.to_string())
}

/// Parse a `Get-AppxPackage | Select PackageFamilyName` stdout into
/// a validated family name. Returns the first non-empty line that
/// starts with `Claude_` and contains no whitespace (a family name
/// is a single token). Everything else → `None`.
///
/// Extracted for testability — the PowerShell pipeline itself is
/// mocked in unit tests by passing its captured stdout.
fn parse_package_family_output(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Reject multi-token lines (e.g. PowerShell's `Name :
        // ...` tabular format leaking through) — a family name
        // never contains whitespace.
        if trimmed.split_whitespace().count() != 1 {
            continue;
        }
        if trimmed.starts_with("Claude_") {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Claude Desktop's MSIX-virtualized data dir, derived from the
/// discovered package family so cross-machine / cross-version
/// installs don't break on rename.
fn package_data_dir() -> Option<PathBuf> {
    let family = package_family();
    dirs::data_local_dir().map(|d| {
        d.join("Packages")
            .join(&family)
            .join("LocalCache")
            .join("Roaming")
            .join("Claude")
    })
}

/// Path to Electron's `Local State` file — Chromium OSCrypt's keyring
/// when DPAPI is the scheme. Phase 6 reads `os_crypt.encrypted_key`
/// from here.
fn local_state_path() -> Option<PathBuf> {
    package_data_dir().map(|d| d.join("Local State"))
}

/// DPAPI `CryptUnprotectData` wrapper (Windows only). Returns the
/// unprotected bytes. Fails with a human-readable error on any of
/// the three invalidation modes called out in reference.md:
///   1. Local State encrypted under a different machine.
///   2. Local State encrypted under a different Windows user.
///   3. Windows user's password was reset out-of-band (DPAPI master
///      key irrecoverable).
#[cfg(target_os = "windows")]
fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>, super::DesktopKeyError> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let mut input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut _,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &mut input,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(super::DesktopKeyError::DpapiFailed(
            "CryptUnprotectData returned FALSE — Local State is \
             bound to a different machine or user, or the Windows \
             user's password was reset out-of-band"
                .into(),
        ));
    }
    let slice =
        unsafe { std::slice::from_raw_parts(output.pbData as *const u8, output.cbData as usize) };
    let result = slice.to_vec();
    unsafe {
        LocalFree(output.pbData as _);
    }
    Ok(result)
}

/// Parse `Local State` JSON, extract and DPAPI-unprotect the
/// `os_crypt.encrypted_key` field. Common entry point shared by the
/// live-identity probe and the DPAPI-invalidation detector.
#[cfg(target_os = "windows")]
fn read_os_crypt_key() -> Result<Vec<u8>, super::DesktopKeyError> {
    use base64::Engine as _;
    let path = local_state_path().ok_or_else(|| {
        super::DesktopKeyError::LocalState("could not resolve Local State path".into())
    })?;
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| super::DesktopKeyError::LocalState(e.to_string()))?;
    let v: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| super::DesktopKeyError::LocalState(format!("JSON parse: {e}")))?;
    let b64 = v
        .pointer("/os_crypt/encrypted_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            super::DesktopKeyError::LocalState(
                "os_crypt.encrypted_key missing — Desktop has no keyring yet".into(),
            )
        })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .map_err(|e| super::DesktopKeyError::LocalState(format!("base64 decode: {e}")))?;
    // Chromium writes the literal ASCII prefix "DPAPI" (5 bytes)
    // before the DPAPI ciphertext. Strip it before unprotecting.
    if bytes.len() < 5 || &bytes[..5] != b"DPAPI" {
        return Err(super::DesktopKeyError::LocalState(
            "encrypted_key missing DPAPI prefix".into(),
        ));
    }
    dpapi_unprotect(&bytes[5..])
}

#[async_trait::async_trait]
impl super::DesktopPlatform for WindowsDesktop {
    fn data_dir(&self) -> Option<PathBuf> {
        // MSIX-virtualized path. Package family is discovered at
        // runtime via `Get-AppxPackage Claude_*` and cached; the
        // compile-time `Claude_pzs8sxrjxfjjc` is only used as a
        // fallback. Codex follow-up D3-5 "Windows AUMID discovery".
        package_data_dir()
    }

    fn session_items(&self) -> &[&str] {
        // Windows adjustments from reference.md §II.3:
        // Cookies under Network/, no fcache, add git-worktrees.json
        &[
            "config.json",
            "Network/Cookies",
            "Network/Cookies-journal",
            "Network/Network Persistent State",
            "DIPS",
            "DIPS-wal",
            "Preferences",
            "ant-did",
            "git-worktrees.json",
            "Local Storage",
            "Session Storage",
            "IndexedDB",
        ]
    }

    fn is_installed(&self) -> bool {
        // Best-effort probe: on Windows the authoritative answer is
        // `Get-AppxPackage Claude_*`, but spawning PowerShell on every
        // invocation is expensive (~200-400 ms cold). We approximate
        // by checking for the LocalCache path that MSIX creates on
        // first install — that directory exists even before first
        // launch populates `data_dir`, so it distinguishes
        // installed-but-never-launched from not-installed.
        let family = package_family();
        dirs::data_local_dir()
            .map(|d| d.join("Packages").join(&family).is_dir())
            .unwrap_or(false)
    }

    async fn safe_storage_secret(&self) -> Result<Vec<u8>, super::DesktopKeyError> {
        // Windows algorithm (per reference.md §II.3):
        //   1. Parse `Local State` JSON → `os_crypt.encrypted_key`.
        //   2. base64-decode; strip the leading 5-byte "DPAPI" tag.
        //   3. CryptUnprotectData → 32-byte AES key.
        //
        // DPAPI is user-scoped; a subprocess inherits the same scope
        // so there's no benefit to spawning. We use `windows-sys`
        // for the direct API call.
        //
        // Failure here is the load-bearing signal for Codex D3-5:
        // if the Local State was encrypted under a different
        // machine/user, or if the Windows password was reset
        // out-of-band, CryptUnprotectData returns FALSE. The error
        // propagates to the caller which surfaces a "re-sign in to
        // Claude Desktop" modal (Phase 6 UX).
        read_os_crypt_key()
    }

    async fn is_running(&self) -> bool {
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        sys.processes()
            .values()
            .any(|p| p.name().to_string_lossy() == "Claude.exe")
    }

    async fn quit(&self) -> Result<(), DesktopSwapError> {
        // Audit H3: send a graceful WM_CLOSE first via taskkill without
        // /F — this asks Claude.exe to close cleanly, matching the
        // macOS AppleScript quit. Only if graceful close doesn't land
        // within the timeout do we fall back to /F.
        //
        // Graceful close lets Electron flush IndexedDB / Local Storage
        // / Session Storage writes in progress; a hard kill can leave
        // partially-written Chromium profile state that breaks the
        // profile on next launch.
        let _ = tokio::process::Command::new("taskkill")
            .args(["/IM", "Claude.exe", "/T"])
            .output()
            .await
            .map_err(DesktopSwapError::Io)?;

        // Poll for graceful exit.
        let graceful_deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < graceful_deadline {
            if !self.is_running().await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // Graceful quit didn't land in 8s. Escalate to /F as a last
        // resort — still better than force-killing immediately, which
        // was the prior behaviour.
        tracing::warn!("graceful taskkill didn't land in 8s; escalating to /F");
        let _ = tokio::process::Command::new("taskkill")
            .args(["/IM", "Claude.exe", "/T", "/F"])
            .output()
            .await
            .map_err(DesktopSwapError::Io)?;

        // Poll for forced exit.
        let force_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < force_deadline {
            if !self.is_running().await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        Err(DesktopSwapError::DesktopStillRunning)
    }

    async fn launch(&self) -> Result<(), DesktopSwapError> {
        // AUMID shape: `<PackageFamily>!<AppId>`. Claude's AppId is
        // the stable string `Claude`; the family is discovered at
        // runtime (Codex D3-5).
        let aumid = format!("{}!Claude", package_family());
        // Audit M8: check exit status. explorer.exe shell:AppsFolder\...
        // returns non-zero if the AUMID doesn't resolve (Claude not
        // installed, MSIX package name changed, permission issue) —
        // silently dropping that made launch() return Ok even when
        // nothing was launched, and the switch reported success.
        let out = tokio::process::Command::new("explorer.exe")
            .arg(format!("shell:AppsFolder\\{aumid}"))
            .output()
            .await
            .map_err(DesktopSwapError::Io)?;
        // Explorer commonly returns 1 even on success for shell:
        // protocol activations. Accept exit codes 0 and 1 as success;
        // fail on anything higher (known-fatal codes on Windows).
        let code = out.status.code().unwrap_or(0);
        if code > 1 {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(DesktopSwapError::Io(std::io::Error::other(format!(
                "explorer shell:AppsFolder launch exited {code}: {}",
                stderr.trim()
            ))));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_backend::DesktopPlatform;

    #[test]
    fn test_is_installed_matches_discovered_package_dir() {
        let p = WindowsDesktop;
        let family = super::package_family();
        let expected = dirs::data_local_dir()
            .map(|d| d.join("Packages").join(&family).is_dir())
            .unwrap_or(false);
        assert_eq!(p.is_installed(), expected);
    }

    #[test]
    fn test_package_family_fallback_is_stable() {
        // Even when discovery fails (CI sandboxes, non-Windows cargo
        // test runs), `package_family()` must return a non-empty
        // string starting with "Claude_". The compile-time constant
        // is the floor.
        let family = super::package_family();
        assert!(
            family.starts_with("Claude_"),
            "package_family() returned unexpected value: {family}"
        );
    }

    #[test]
    fn parse_package_family_output_accepts_single_valid_line() {
        // The happy path: one line, trimmed, correct prefix.
        let out = "Claude_pzs8sxrjxfjjc\n";
        assert_eq!(
            super::parse_package_family_output(out),
            Some("Claude_pzs8sxrjxfjjc".to_string())
        );
    }

    #[test]
    fn parse_package_family_output_trims_crlf_and_whitespace() {
        // PowerShell on Windows emits CRLF; the parser must not leak
        // the trailing \r into the returned family name.
        assert_eq!(
            super::parse_package_family_output("Claude_xyz123\r\n"),
            Some("Claude_xyz123".to_string())
        );
        assert_eq!(
            super::parse_package_family_output("  Claude_xyz123  \n"),
            Some("Claude_xyz123".to_string())
        );
    }

    #[test]
    fn parse_package_family_output_rejects_empty_and_missing_prefix() {
        // An empty probe, a blank line, or a probe that selected a
        // package that isn't actually Claude must all be rejected so
        // the filesystem fallback has a chance to run.
        assert_eq!(super::parse_package_family_output(""), None);
        assert_eq!(super::parse_package_family_output("\n\n"), None);
        assert_eq!(
            super::parse_package_family_output("Microsoft.Edge_8wekyb3d8bbwe"),
            None,
        );
    }

    #[test]
    fn parse_package_family_output_rejects_tabular_leakage() {
        // Even with `-NoProfile -NonInteractive`, some CI PowerShell
        // configurations wrap output in a formatted table. A
        // multi-token line must be skipped rather than coerced.
        let tabular = "\nPackageFamilyName\n-----------------\nClaude_pzs8sxrjxfjjc\n";
        assert_eq!(
            super::parse_package_family_output(tabular),
            Some("Claude_pzs8sxrjxfjjc".to_string())
        );
        assert_eq!(
            super::parse_package_family_output("Name : Claude_abc"),
            None
        );
    }

    #[test]
    fn parse_package_family_output_picks_first_matching_line() {
        // If multiple Claude_* packages are installed, the caller has
        // already filtered with `Select-Object -First 1`. The parser
        // must still honour the first Claude_ line even if leading
        // lines are blank or unrelated.
        let out = "\nClaudeCompanion_aaa\nClaude_primary\nClaude_secondary\n";
        // ClaudeCompanion_ doesn't match the `Claude_` underscore
        // boundary, so the parser skips it and takes the real one.
        assert_eq!(
            super::parse_package_family_output(out),
            Some("Claude_primary".to_string())
        );
    }
}

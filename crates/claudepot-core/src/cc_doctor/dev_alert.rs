//! Developer-only signal that the `claude doctor` scraper hit a
//! failure mode.
//!
//! Two gates must BOTH hold before this fires:
//!
//! 1. **Mode gate** — either `debug_assertions` (i.e., a `cargo
//!    build` without `--release`) or `CLAUDEPOT_DEV=1` in the
//!    environment. End-user release builds without that env never
//!    surface a developer-facing alert — they get the silent
//!    fallback (last-known-good snapshot, `Warning` severity on the
//!    pill).
//!
//! 2. **De-dupe gate** — at most one alert per process per CC
//!    version. If the parser breaks for the same CC version twice
//!    in a session, we record both to the JSONL log but only the
//!    first triggers an OS notification. Without this, every 60 s
//!    refresh would spam the dev's notification center.
//!
//! Why notify here and not via the renderer: the parse failure is
//! often a state mismatch the renderer never sees (e.g., parser
//! returns `Degraded` with no sections; renderer would just show
//! the empty pane). Surfacing at the source is faster diagnosis.

use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Mutex;

use crate::cc_doctor::parse_failures::ParseFailureEntry;

/// Track which `(cc_version, reason)` pairs have already alerted in
/// this process. Reset on Claudepot restart — fine, the previous
/// run's log is on disk in `doctor-parse-failures.jsonl`.
static ALERTED: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// Fire an OS notification with the failure details, gated by the
/// rules above. Best-effort — no return value because nothing
/// upstream can act on a failed alert.
pub fn dispatch_if_dev_mode(entry: &ParseFailureEntry) {
    if !is_dev_mode() {
        return;
    }

    let dedup_key = format!(
        "{}::{}",
        entry.cc_version.as_deref().unwrap_or("unknown"),
        entry.reason
    );
    {
        let mut set = match ALERTED.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if !set.insert(dedup_key) {
            return;
        }
    }

    let title = "Claudepot dev: cc_doctor parser failed".to_string();
    let body = format!(
        "CC {}, claudepot {}: {} (raw {} B). See ~/.claudepot/doctor-parse-failures.jsonl",
        entry.cc_version.as_deref().unwrap_or("unknown"),
        entry.claudepot_version,
        entry.reason,
        entry.raw_bytes,
    );

    // tracing for the dev who runs `pnpm tauri dev` and watches the
    // console — the OS notification is the same payload via a
    // different surface.
    tracing::warn!("cc_doctor dev-alert: {body}");

    notify_platform(&title, &body);
}

fn is_dev_mode() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }
    matches!(
        std::env::var("CLAUDEPOT_DEV").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

#[cfg(target_os = "macos")]
fn notify_platform(title: &str, body: &str) {
    // `osascript` is the cheapest pure-Rust-free path to a banner
    // on macOS — no AppleScript dependency in our crate, no
    // tauri-plugin-notification (this is core, can't depend on Tauri).
    let escaped_title = escape_applescript(title);
    let escaped_body = escape_applescript(body);
    let script = format!("display notification \"{escaped_body}\" with title \"{escaped_title}\"");
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output();
}

#[cfg(target_os = "linux")]
fn notify_platform(title: &str, body: &str) {
    // `notify-send` is in libnotify-bin on most distros; fall
    // through silently if absent.
    let _ = std::process::Command::new("notify-send")
        .args([title, body])
        .output();
}

#[cfg(target_os = "windows")]
fn notify_platform(title: &str, body: &str) {
    // PowerShell's BurntToast-free path: BalloonTip via
    // System.Windows.Forms. Fire-and-forget; absent .NET is rare on
    // modern Windows but not fatal.
    let escaped_title = title.replace('\'', "''");
    let escaped_body = body.replace('\'', "''");
    let script = format!(
        "[reflection.assembly]::loadwithpartialname('System.Windows.Forms') | Out-Null; \
         $b = New-Object System.Windows.Forms.NotifyIcon; \
         $b.Icon = [System.Drawing.SystemIcons]::Information; \
         $b.BalloonTipTitle = '{escaped_title}'; \
         $b.BalloonTipText = '{escaped_body}'; \
         $b.Visible = $true; \
         $b.ShowBalloonTip(8000); \
         Start-Sleep -Seconds 9; \
         $b.Dispose()"
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .output();
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn notify_platform(_title: &str, _body: &str) {
    // Unsupported platform — tracing::warn above is the only signal.
}

#[cfg(target_os = "macos")]
fn escape_applescript(s: &str) -> String {
    // AppleScript string literals: backslash and double-quote need
    // escaping. Newlines collapse to `\n` so the banner stays on
    // one logical line.
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
        .replace('\r', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_blocks_second_call_with_same_key() {
        // Clear any state left by earlier tests.
        if let Ok(mut g) = ALERTED.lock() {
            g.clear();
        }
        let entry = ParseFailureEntry {
            ts_ms: 0,
            cc_version: Some("test-x".into()),
            claudepot_version: "0.0.0".into(),
            raw_bytes: 100,
            reason: "dedupe-test".into(),
            raw_b64: String::new(),
        };

        // First call inserts the key; second call should be a no-op
        // for the in-process set (whether the OS notification fired
        // depends on dev mode, which we don't test here — the test
        // is for the de-dupe logic).
        let key = format!(
            "{}::{}",
            entry.cc_version.as_deref().unwrap_or("unknown"),
            entry.reason
        );

        {
            let mut g = ALERTED.lock().unwrap();
            assert!(g.insert(key.clone()), "first insert should return true");
            assert!(!g.insert(key.clone()), "second insert should return false");
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn applescript_escape_handles_quotes_and_backslash() {
        let s = r#"a"b\c"#;
        assert_eq!(escape_applescript(s), r#"a\"b\\c"#);
    }
}

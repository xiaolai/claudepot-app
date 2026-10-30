//! The relay verbs' security gates, through the real binary: each
//! refusal must happen before anything is written, and the authorized
//! path must still work. Every run gets its own `CLAUDEPOT_DATA_DIR`
//! (set on the child, never process-wide) and touches no keychain — the
//! refusals tested here all come before any credential-store access, and
//! `relay serve` with nothing published reaches no credential backend.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

fn run(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_claudepot"))
        .env("CLAUDEPOT_DATA_DIR", data)
        .args(args)
        // Not a terminal: no prompt may stand in for a missing flag.
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Every `relay-*` file in the data dir, with its bytes. Not directories:
/// `relay-revoke/` holds only a serving relay's socket.
fn relay_files(data: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(data)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("relay-"))
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn pairing_without_a_fingerprint_off_a_terminal_is_refused_before_any_contact() {
    let data = TempDir::new().unwrap();
    // Port 9 (discard) on loopback: nothing should ever try it.
    let out = run(
        data.path(),
        &["relay", "pair", "127.0.0.1:9", "--code", "000000"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "pairing without a fingerprint ran");
    assert!(stderr.contains("--fingerprint"), "{stderr}");
    assert!(
        relay_files(data.path()).is_empty(),
        "a refused pairing wrote relay state"
    );
}

/// `relay serve` in the background. It is killed on drop, so a failed
/// assertion never leaves a server running.
#[cfg(unix)]
struct Serving {
    child: std::process::Child,
    /// Every stdout line, as it arrives.
    lines: std::sync::mpsc::Receiver<String>,
}

#[cfg(unix)]
impl Serving {
    /// Bind to an ephemeral loopback port first, so tests never collide
    /// with each other or with a relay this machine really runs.
    fn start(data: &Path, args: &[&str]) -> Self {
        let out = run(data, &["relay", "bind", "127.0.0.1", "--port", "0"]);
        assert!(
            out.status.success(),
            "relay bind failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_claudepot"))
            .env("CLAUDEPOT_DATA_DIR", data)
            .args(["relay", "serve"])
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines() {
                if tx.send(line.unwrap_or_default()).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    /// Stdout up to and including the first line `done` accepts.
    fn read_until(&self, done: impl Fn(&str) -> bool) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut text = String::new();
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|e| panic!("relay serve printed no offer ({e}); so far: {text}"));
            text.push_str(&line);
            text.push('\n');
            if done(&line) {
                return text;
            }
        }
    }

    /// SIGTERM, then the exit status and stderr.
    fn terminate(mut self) -> (std::process::ExitStatus, String) {
        let pid = self.child.id().to_string();
        let killed = Command::new("kill").args(["-TERM", &pid]).status().unwrap();
        assert!(killed.success(), "could not signal relay serve");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let status = loop {
            if let Some(s) = self.child.try_wait().unwrap() {
                break s;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "relay serve did not stop on SIGTERM"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let mut stderr = String::new();
        use std::io::Read;
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        (status, stderr)
    }
}

#[cfg(unix)]
impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A refusal that names the live server, as every offline edit and a
/// second serve give while `relay-serve.lock` is held.
#[cfg(unix)]
fn refused_as_serving(out: &Output) -> bool {
    !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("a relay is serving")
}

#[cfg(unix)]
#[test]
fn serve_with_pair_and_json_prints_one_offer_holds_the_lock_and_releases_it_on_sigterm() {
    let data = TempDir::new().unwrap();
    let server = Serving::start(data.path(), &["--pair", "--json"]);
    // `print_json` pretty-prints: the object ends at the first bare `}`.
    let text = server.read_until(|l| l == "}");
    let offer: serde_json::Value = serde_json::from_str(&text).unwrap();
    let addr: std::net::SocketAddr = offer["addr"].as_str().unwrap().parse().unwrap();
    assert!(addr.ip().is_loopback(), "{offer}");
    assert_ne!(
        addr.port(),
        0,
        "the bound port, not the requested 0: {offer}"
    );
    let fingerprint = offer["ca_fingerprint"].as_str().unwrap();
    assert!(!fingerprint.is_empty(), "{offer}");
    assert_eq!(offer["pairing"]["ca_fingerprint"], fingerprint, "{offer}");
    assert!(
        !offer["pairing"]["code"].as_str().unwrap().is_empty(),
        "{offer}"
    );
    assert!(offer["pairing"]["expires_in_secs"].as_u64().unwrap() > 0);

    // While it serves, this process owns the relay: a second serve and an
    // offline edit are both refused.
    assert!(refused_as_serving(&run(data.path(), &["relay", "serve"])));
    assert!(refused_as_serving(&run(
        data.path(),
        &["relay", "bind", "127.0.0.1", "--port", "0"]
    )));

    let (status, stderr) = server.terminate();
    assert!(status.success(), "relay serve exited {status}: {stderr}");
    assert!(stderr.contains("relay stopped"), "{stderr}");
    // "stopped" was said only after the serve lock was released.
    let out = run(data.path(), &["relay", "bind", "127.0.0.1", "--port", "0"]);
    assert!(
        out.status.success(),
        "the serve lock outlived the server: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A paired device, written straight into the data dir as pairing leaves
/// it, so no client and no keychain are involved.
#[cfg(unix)]
fn seed_device(data: &Path, name: &str) -> String {
    use claudepot_core::relay::devices::DeviceRegistry;
    use claudepot_core::relay::runtime::RelayPaths;
    let (devices, _) = DeviceRegistry::open(RelayPaths::in_dir(data).devices).unwrap();
    devices.issue(name, vec![]).unwrap().device_id
}

/// `relay revoke --json`, which must succeed.
#[cfg(unix)]
fn revoke_json(data: &Path, device_id: &str) -> serde_json::Value {
    let out = run(data, &["relay", "revoke", device_id, "--json"]);
    assert!(
        out.status.success(),
        "relay revoke {device_id} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[cfg(unix)]
#[test]
fn revoke_from_another_shell_goes_through_the_serving_relay() {
    use claudepot_core::relay::revoke_socket::socket_path;
    use serde_json::json;

    let data = TempDir::new().unwrap();
    let laptop = seed_device(data.path(), "laptop");
    let phone = seed_device(data.path(), "phone");
    let server = Serving::start(data.path(), &["--json"]);
    server.read_until(|l| l == "}");

    // Not refused as serving: the serving relay revokes it, and a repeat
    // reports what it found.
    assert_eq!(
        revoke_json(data.path(), &laptop),
        json!({ "revoked": laptop, "via": "serving_relay", "already_revoked": false })
    );
    assert_eq!(
        revoke_json(data.path(), &laptop),
        json!({ "revoked": laptop, "via": "serving_relay", "already_revoked": true })
    );

    // An unknown device is refused, and nothing is written.
    let before = relay_files(data.path());
    let out = run(data.path(), &["relay", "revoke", "no-such-device"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "revoking an unknown device succeeded"
    );
    assert!(stderr.contains("no such relay device"), "{stderr}");
    assert_eq!(relay_files(data.path()), before, "a refused revoke wrote");

    let (status, stderr) = server.terminate();
    assert!(status.success(), "relay serve exited {status}: {stderr}");
    assert!(
        !socket_path(data.path()).exists(),
        "the revoke socket outlived the relay"
    );

    // Offline again. The serving relay's revocation is on disk, and the
    // device it never touched is still live until revoked here.
    assert_eq!(
        revoke_json(data.path(), &laptop),
        json!({ "revoked": laptop, "via": "offline", "already_revoked": true })
    );
    assert_eq!(
        revoke_json(data.path(), &phone),
        json!({ "revoked": phone, "via": "offline", "already_revoked": false })
    );
}

#[cfg(unix)]
#[test]
fn serve_quiet_still_prints_the_pairing_code_and_nothing_else() {
    let data = TempDir::new().unwrap();
    let server = Serving::start(data.path(), &["--pair", "--quiet"]);
    let text = server.read_until(|l| l.starts_with("Valid for"));
    let code = text
        .lines()
        .find_map(|l| l.strip_prefix("Pairing code: "))
        .unwrap_or_else(|| panic!("no pairing code under --quiet: {text}"));
    assert!(!code.trim().is_empty(), "{text}");
    assert!(text.contains("Fingerprint:"), "{text}");
    assert!(
        text.contains(&format!("--code {}", code.trim())),
        "the client hint does not carry the code: {text}"
    );
    let (status, stderr) = server.terminate();
    assert!(status.success(), "relay serve exited {status}: {stderr}");
    // --quiet hides progress: neither the listen line nor "stopped".
    assert!(!stderr.contains("Relay serving"), "{stderr}");
    assert!(!stderr.contains("relay stopped"), "{stderr}");
}

#[test]
fn rotating_the_ca_needs_yes_and_a_refusal_changes_nothing() {
    let data = TempDir::new().unwrap();

    // The authorized path: mints a CA.
    let out = run(data.path(), &["relay", "rotate-ca", "--yes", "--json"]);
    assert!(
        out.status.success(),
        "rotate-ca --yes failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rotated: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(rotated["ca_fingerprint"].is_string(), "{rotated}");
    let before = relay_files(data.path());
    assert!(
        before.contains_key("relay-ca.pem"),
        "no CA after rotate-ca --yes"
    );

    // Without --yes: refused, and every relay file is byte-identical.
    let out = run(data.path(), &["relay", "rotate-ca"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "rotate-ca ran without --yes");
    assert!(stderr.contains("--yes"), "{stderr}");
    assert_eq!(
        relay_files(data.path()),
        before,
        "a refused rotate-ca changed relay state"
    );
}

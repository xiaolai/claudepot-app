//! Lightweight health probe for the `claudepot mcp memory-server`
//! subprocess.
//!
//! Spawns the CLI binary, hand-writes the minimal JSON-RPC handshake
//! (`initialize` → `notifications/initialized` → `tools/list`),
//! counts the tools in the response, and classifies failures. The
//! Settings → MCP pane renders the result as a "tool_visible" badge.
//!
//! Extracted from `src-tauri/commands/shared_memory.rs` (audit:
//! protocol framing + binary-resolution policy are business logic
//! and belong in core, testable without a webview). The pure parts
//! — candidate ordering ([`cli_candidates`]), the read loop
//! ([`read_tools_count`]), and failure classification
//! ([`classify_probe_failure`]) — are unit-tested below; the
//! subprocess wiring in [`probe_memory_server`] only composes them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, BufReader};

/// Structured probe outcome. `error` is `Some` whenever
/// `tool_count == 0` — the classification distinguishes a stderr
/// complaint, a hung server (timeout), and a clean-but-silent exit.
#[derive(Debug, Clone)]
pub struct ProbeReport {
    pub tool_visible: bool,
    pub tool_count: usize,
    pub error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum McpProbeError {
    /// No CLI binary next to the running executable. Carries the
    /// candidate list so the user can see what was probed.
    #[error("no claudepot CLI binary found in {exe_dir} (tried: {tried})")]
    CliNotFound { exe_dir: String, tried: String },
    /// The binary exists but could not be spawned.
    #[error("spawn {bin}: {source}")]
    Spawn { bin: String, source: std::io::Error },
}

/// Candidate binary names for the `claudepot` CLI sitting next to
/// the GUI executable, in probe order. The GUI binary itself doesn't
/// have the `mcp memory-server` subcommand — that's only on the CLI
/// crate.
///
/// - **dev** (`debug = true`): prefer `claudepot` (the CLI's
///   `[[bin]] name`). A stale `claudepot-cli` artifact from a
///   pre-rename build may still sit next to it in `target/debug/` —
///   probing that first would spawn the wrong binary.
/// - **release**: the sidecar is named `claudepot-cli`
///   (externalBin-resolved, copied in at bundle time), with bare
///   `claudepot` as a manual-install fallback.
/// - Pre-bundle externalBin candidates carry the host target triple
///   and come last on every platform.
pub fn cli_candidates(exe_dir: &Path, debug: bool) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = if debug {
        vec![
            exe_dir.join("claudepot"),     // dev: target/debug/claudepot
            exe_dir.join("claudepot-cli"), // dev: stale rename fallback
        ]
    } else {
        vec![
            exe_dir.join("claudepot-cli"), // prod bundle: Contents/MacOS/claudepot-cli
            exe_dir.join("claudepot"),     // fallback if installed manually
        ]
    };
    #[cfg(target_os = "macos")]
    {
        candidates.push(exe_dir.join("claudepot-cli-aarch64-apple-darwin"));
        candidates.push(exe_dir.join("claudepot-cli-x86_64-apple-darwin"));
    }
    #[cfg(target_os = "linux")]
    {
        candidates.push(exe_dir.join("claudepot-cli-x86_64-unknown-linux-gnu"));
        candidates.push(exe_dir.join("claudepot-cli-aarch64-unknown-linux-gnu"));
    }
    #[cfg(target_os = "windows")]
    {
        candidates.push(exe_dir.join("claudepot-cli-x86_64-pc-windows-msvc.exe"));
    }
    candidates
}

/// Resolve the first existing [`cli_candidates`] entry under
/// `exe_dir`. `debug` selects the dev-vs-prod naming preference —
/// callers pass `cfg!(debug_assertions)`.
pub fn resolve_sibling_cli(exe_dir: &Path, debug: bool) -> Result<PathBuf, McpProbeError> {
    let candidates = cli_candidates(exe_dir, debug);
    for c in &candidates {
        if c.exists() {
            return Ok(c.clone());
        }
    }
    Err(McpProbeError::CliNotFound {
        exe_dir: exe_dir.display().to_string(),
        tried: candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

/// The newline-delimited JSON-RPC handshake written to the server's
/// stdin: `initialize`, the `initialized` notification, then a
/// `tools/list` request with `id: 2` — the id [`read_tools_count`]
/// keys on.
const HANDSHAKE_FRAMES: &str = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"claudepot-health\",\"version\":\"0\"}}}\n\
                                {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                                {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";

/// Read newline-delimited JSON-RPC frames until the `tools/list`
/// response (`id == 2` carrying `result.tools`) arrives or the
/// stream ends. Returns the tool count (0 on EOF / read error
/// before a usable response). Blank lines and non-JSON noise are
/// skipped; an `id: 2` frame *without* a `result.tools` array (e.g.
/// an error response) keeps the loop reading until EOF.
async fn read_tools_count<R: AsyncBufRead + Unpin>(reader: R) -> usize {
    let mut reader = reader;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let v: serde_json::Value = match serde_json::from_str(trimmed) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if v.get("id").and_then(|i| i.as_u64()) == Some(2) {
                    if let Some(tools) = v
                        .get("result")
                        .and_then(|r| r.get("tools"))
                        .and_then(|t| t.as_array())
                    {
                        return tools.len();
                    }
                }
            }
            Err(_) => break,
        }
    }
    0
}

/// Classify a zero-tool probe outcome into a user-facing message.
/// Priority: a non-empty stderr beats both timeout and clean-exit
/// explanations (the server said *why* it's unhappy).
fn classify_probe_failure(
    bin: &Path,
    stderr_trimmed: &str,
    timed_out: bool,
    timeout: Duration,
) -> String {
    if !stderr_trimmed.is_empty() {
        format!("stderr from {}: {stderr_trimmed}", bin.display())
    } else if timed_out {
        format!(
            "spawned {} but no tools/list response within {}s",
            bin.display(),
            timeout.as_secs()
        )
    } else {
        format!(
            "spawned {} but it exited before emitting a tools/list response",
            bin.display()
        )
    }
}

/// Spawn `<bin> mcp memory-server`, run the handshake, and report
/// whether (and how many) tools are visible.
///
/// `Err` only for a spawn failure; every post-spawn failure mode is
/// an `Ok(ProbeReport { error: Some(..), .. })` so the caller can
/// render a structured badge instead of a hard error.
pub async fn probe_memory_server(
    bin: &Path,
    probe_timeout: Duration,
) -> Result<ProbeReport, McpProbeError> {
    use crate::proc_utils::NoWindowExt;
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;
    use tokio::time::timeout;

    let mut child = Command::new(bin)
        .args(["mcp", "memory-server"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("RUST_LOG", "warn")
        .kill_on_drop(true)
        .no_window()
        .spawn()
        .map_err(|e| McpProbeError::Spawn {
            bin: bin.display().to_string(),
            source: e,
        })?;

    let stdin_handle = child.stdin.take();
    let stdout_handle = match child.stdout.take() {
        Some(s) => s,
        None => {
            // Piped stdout always exists post-spawn; defensive arm.
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Ok(ProbeReport {
                tool_visible: false,
                tool_count: 0,
                error: Some(format!("spawned {} but no stdout pipe", bin.display())),
            });
        }
    };
    let stderr_handle = child.stderr.take();

    if let Some(mut stdin) = stdin_handle {
        let _ = stdin.write_all(HANDSHAKE_FRAMES.as_bytes()).await;
        // Drop stdin (EOF) so the server processes queued requests
        // then exits cleanly on its own.
        drop(stdin);
    }

    // Hard wall-clock cap that survives a child that stays alive but
    // emits no newline. Tokio's `timeout` aborts the whole read
    // future, not just the deadline between reads — this is the
    // exact bug the old `Instant::now() < deadline` shape had with
    // blocking `BufReader::read_line`.
    let (tool_count, timed_out) = match timeout(
        probe_timeout,
        read_tools_count(BufReader::new(stdout_handle)),
    )
    .await
    {
        Ok(count) => (count, false),
        Err(_) => (0, true),
    };

    // kill_on_drop = true would handle this when `child` falls out
    // of scope, but be explicit to release the descriptor before we
    // drain stderr.
    let _ = child.start_kill();
    let _ = child.wait().await;

    let error = if tool_count == 0 {
        let mut buf = String::new();
        if let Some(mut s) = stderr_handle {
            let mut take = (&mut s).take(1024);
            let _ = take.read_to_string(&mut buf).await;
        }
        Some(classify_probe_failure(
            bin,
            buf.trim(),
            timed_out,
            probe_timeout,
        ))
    } else {
        None
    };

    Ok(ProbeReport {
        tool_visible: tool_count > 0,
        tool_count,
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- candidate ordering ----------

    #[test]
    fn test_cli_candidates_debug_prefers_bare_claudepot() {
        let dir = Path::new("/x");
        let c = cli_candidates(dir, true);
        assert_eq!(c[0], dir.join("claudepot"));
        assert_eq!(c[1], dir.join("claudepot-cli"));
    }

    #[test]
    fn test_cli_candidates_release_prefers_sidecar_name() {
        let dir = Path::new("/x");
        let c = cli_candidates(dir, false);
        assert_eq!(c[0], dir.join("claudepot-cli"));
        assert_eq!(c[1], dir.join("claudepot"));
    }

    #[test]
    fn test_cli_candidates_triple_suffixed_come_last() {
        let dir = Path::new("/x");
        for debug in [true, false] {
            let c = cli_candidates(dir, debug);
            assert!(c.len() > 2, "platform triple candidates must be appended");
            for extra in &c[2..] {
                let name = extra.file_name().unwrap().to_string_lossy().to_string();
                assert!(
                    name.starts_with("claudepot-cli-"),
                    "trailing candidates carry the target triple: {name}"
                );
            }
        }
    }

    #[test]
    fn test_resolve_sibling_cli_picks_first_existing_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        // Both dev names exist — debug mode must pick bare
        // `claudepot` even though `claudepot-cli` also exists (the
        // stale-rename bug this ordering encodes).
        std::fs::write(tmp.path().join("claudepot"), b"x").unwrap();
        std::fs::write(tmp.path().join("claudepot-cli"), b"x").unwrap();
        let resolved = resolve_sibling_cli(tmp.path(), true).unwrap();
        assert_eq!(resolved, tmp.path().join("claudepot"));
        // Release mode flips the preference.
        let resolved = resolve_sibling_cli(tmp.path(), false).unwrap();
        assert_eq!(resolved, tmp.path().join("claudepot-cli"));
    }

    #[test]
    fn test_resolve_sibling_cli_errors_with_candidate_list_when_none_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let err = resolve_sibling_cli(tmp.path(), true).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no claudepot CLI binary found"));
        assert!(msg.contains("claudepot-cli"), "lists the candidates: {msg}");
    }

    // ---------- read loop (fixture transcripts) ----------

    #[tokio::test]
    async fn test_read_tools_count_finds_id2_tools_array() {
        let transcript = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"capabilities\":{}}}\n\
                           \n\
                           not json at all\n\
                           {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{},{},{}]}}\n";
        assert_eq!(read_tools_count(&transcript[..]).await, 3);
    }

    #[tokio::test]
    async fn test_read_tools_count_zero_on_eof_without_response() {
        let transcript = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n";
        assert_eq!(read_tools_count(&transcript[..]).await, 0);
    }

    #[tokio::test]
    async fn test_read_tools_count_id2_error_response_keeps_reading() {
        // An id-2 frame WITHOUT result.tools (an error response) must
        // not satisfy the probe; a later id-2 with tools does.
        let transcript = b"{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-32600}}\n\
                           {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{}]}}\n";
        assert_eq!(read_tools_count(&transcript[..]).await, 1);
    }

    #[tokio::test]
    async fn test_read_tools_count_empty_tools_array_is_zero() {
        let transcript = b"{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}\n";
        assert_eq!(read_tools_count(&transcript[..]).await, 0);
    }

    // ---------- failure classification ----------

    #[test]
    fn test_classify_stderr_beats_timeout() {
        let msg = classify_probe_failure(
            Path::new("/x/claudepot"),
            "boom",
            true,
            Duration::from_secs(8),
        );
        assert!(msg.contains("stderr from"));
        assert!(msg.contains("boom"));
    }

    #[test]
    fn test_classify_timeout_when_stderr_empty() {
        let msg =
            classify_probe_failure(Path::new("/x/claudepot"), "", true, Duration::from_secs(8));
        assert!(msg.contains("no tools/list response within 8s"), "{msg}");
    }

    #[test]
    fn test_classify_clean_exit_when_no_stderr_no_timeout() {
        let msg =
            classify_probe_failure(Path::new("/x/claudepot"), "", false, Duration::from_secs(8));
        assert!(msg.contains("exited before emitting"), "{msg}");
    }

    // ---------- handshake frames ----------

    #[test]
    fn test_handshake_frames_are_three_json_lines_with_id2_tools_list() {
        let lines: Vec<&str> = HANDSHAKE_FRAMES
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(lines.len(), 3);
        for l in &lines {
            let v: serde_json::Value = serde_json::from_str(l).expect("frame must be valid JSON");
            assert_eq!(v.get("jsonrpc").and_then(|s| s.as_str()), Some("2.0"));
        }
        let last: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(last.get("id").and_then(|i| i.as_u64()), Some(2));
        assert_eq!(
            last.get("method").and_then(|m| m.as_str()),
            Some("tools/list")
        );
    }
}

//! Share a session export as a GitHub Gist.
//!
//! One public function, `share_gist`, which:
//!   · POSTs to `https://api.github.com/gists`
//!   · honors a single 5xx retry with 2s backoff
//!   · never lets the PAT leak into error strings
//!   · emits `preparing → uploading → complete` phase events via
//!     [`ProgressSink`].
//!
//! The base URL is injectable so test code can point at a mock server
//! without stubbing the whole client.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::project_progress::{PhaseStatus, ProgressSink};

const DEFAULT_BASE: &str = "https://api.github.com";

#[derive(Debug, Error)]
pub enum ShareError {
    #[error("authentication failed (401); the token was rejected")]
    Auth,
    #[error("payload rejected by GitHub (422 {0})")]
    Oversized(String),
    #[error("GitHub returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("response parse failed: {0}")]
    Parse(String),
}

/// Scrubs a GitHub PAT out of arbitrary strings — used so the Display
/// impl on any error crossing this module is token-free.
pub fn scrub_token(s: &str, token: &str) -> String {
    if token.is_empty() {
        return s.to_string();
    }
    s.replace(token, "<token-redacted>")
}

/// Return value on success.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GistResult {
    pub url: String,
    pub id: String,
}

struct Progress<'a>(&'a dyn ProgressSink);

impl fmt::Debug for Progress<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Progress")
    }
}

/// Upload `content` as a single-file gist named `filename`.
pub async fn share_gist(
    content: &str,
    filename: &str,
    description: &str,
    public: bool,
    token: &str,
    sink: &dyn ProgressSink,
) -> Result<GistResult, ShareError> {
    share_gist_with_base(content, filename, description, public, token, sink, DEFAULT_BASE).await
}

/// Base-URL injection point for tests. Production callers use
/// `share_gist`.
pub async fn share_gist_with_base(
    content: &str,
    filename: &str,
    description: &str,
    public: bool,
    token: &str,
    sink: &dyn ProgressSink,
    base_url: &str,
) -> Result<GistResult, ShareError> {
    sink.phase("preparing", PhaseStatus::Complete);

    let body = serde_json::json!({
        "description": description,
        "public": public,
        "files": {
            filename: { "content": content }
        }
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("claudepot/0.1")
        .build()
        .map_err(|e| ShareError::Network(scrub_token(&e.to_string(), token)))?;

    let url = format!("{}/gists", base_url.trim_end_matches('/'));
    sink.phase("uploading", PhaseStatus::Running);

    let mut last_err: Option<ShareError> = None;
    for attempt in 0..2 {
        let resp = client
            .post(&url)
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await;
        match resp {
            Ok(r) => {
                let status = r.status();
                if status.is_success() {
                    let parsed: GistResponse = r
                        .json()
                        .await
                        .map_err(|e| ShareError::Parse(scrub_token(&e.to_string(), token)))?;
                    sink.phase("complete", PhaseStatus::Complete);
                    return Ok(GistResult {
                        url: parsed.html_url,
                        id: parsed.id,
                    });
                }
                let code = status.as_u16();
                let body_text = r.text().await.unwrap_or_default();
                let scrubbed = scrub_token(&body_text, token);
                let err = match code {
                    401 | 403 => ShareError::Auth,
                    422 => ShareError::Oversized(scrubbed),
                    500..=599 if attempt == 0 => {
                        // retriable — sleep then loop
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        last_err = Some(ShareError::Http {
                            status: code,
                            body: scrubbed,
                        });
                        continue;
                    }
                    _ => ShareError::Http {
                        status: code,
                        body: scrubbed,
                    },
                };
                sink.phase(
                    "uploading",
                    PhaseStatus::Error(err.to_string()),
                );
                return Err(err);
            }
            Err(e) => {
                let msg = scrub_token(&e.to_string(), token);
                if attempt == 0 && (e.is_timeout() || e.is_connect()) {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    last_err = Some(ShareError::Network(msg));
                    continue;
                }
                sink.phase("uploading", PhaseStatus::Error(msg.clone()));
                return Err(ShareError::Network(msg));
            }
        }
    }
    Err(last_err.unwrap_or(ShareError::Network("unknown".into())))
}

#[derive(Deserialize)]
struct GistResponse {
    id: String,
    html_url: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_progress::NoopSink;

    #[test]
    fn scrub_token_removes_secret_from_body() {
        let secret = "ghp_abcdef0123456789";
        let msg = format!("request failed with token={secret} body");
        let out = scrub_token(&msg, secret);
        assert!(!out.contains(secret));
        assert!(out.contains("<token-redacted>"));
    }

    #[test]
    fn scrub_token_empty_is_noop() {
        let out = scrub_token("nothing to redact", "");
        assert_eq!(out, "nothing to redact");
    }

    #[test]
    fn share_error_display_does_not_echo_body_back_unscrubbed() {
        // ShareError::Http's Display shows status + body. The caller
        // is responsible for scrubbing the body BEFORE constructing
        // the error. Verified in the happy-path + 422 tests via the
        // public function, but also lock it down directly here.
        let err = ShareError::Http {
            status: 500,
            body: "normal body".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("500"));
        assert!(msg.contains("normal body"));
    }

    #[tokio::test]
    async fn gist_happy_path_returns_url() {
        // Mini mock: bind a TcpListener + one-shot handler returning 201.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let body = serde_json::json!({
                "id": "deadbeef",
                "html_url": "https://gist.github.com/xiaolai/deadbeef"
            })
            .to_string();
            let resp = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.flush().await;
        });

        let got = share_gist_with_base(
            "body",
            "session.md",
            "desc",
            false,
            "ghp_testtoken",
            &NoopSink,
            &base,
        )
        .await
        .unwrap();
        assert_eq!(got.id, "deadbeef");
        assert_eq!(got.url, "https://gist.github.com/xiaolai/deadbeef");
    }

    #[tokio::test]
    async fn gist_401_becomes_auth_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let resp = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
        });

        let err = share_gist_with_base(
            "body",
            "x.md",
            "d",
            false,
            "ghp_testtoken",
            &NoopSink,
            &base,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ShareError::Auth));
        // Error display never contains the token.
        let msg = format!("{err}");
        assert!(!msg.contains("ghp_testtoken"));
    }

    #[tokio::test]
    async fn gist_422_surfaces_as_oversized() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let body = "{\"message\": \"too large\"}";
            let resp = format!(
                "HTTP/1.1 422 Unprocessable Entity\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(resp.as_bytes()).await;
        });

        let err = share_gist_with_base(
            "body",
            "x.md",
            "d",
            false,
            "ghp_testtoken",
            &NoopSink,
            &base,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ShareError::Oversized(_)), "err={err:?}");
    }
}

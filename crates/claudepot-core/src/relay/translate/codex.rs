//! The Codex subscription credential (plan §9, P-D Q5 option (a)).
//!
//! The relay never performs Codex's token exchange. It asks Codex's own
//! `app-server` for the token — `getAuthStatus` over stdio, the call
//! clai's `clcd` makes — in a `CODEX_HOME` that **only the relay uses**
//! (`~/.claudepot/relay-codex/`), so there is no second owner of the
//! refresh token: the P-D experiment double-spent a shared refresh token
//! 10 times in 10 on overlap. One home, one relay, and every call here is
//! serialised by one mutex, so two requests never start two refreshes.
//!
//! Sign the home in once with `mkdir -p -m 700 ~/.claudepot/relay-codex &&
//! CODEX_HOME=~/.claudepot/relay-codex codex login` — codex refuses a
//! `CODEX_HOME` that does not exist yet (measured, codex-cli 0.166.0).
//! Until then every request is `credential_unavailable`.
//!
//! A token within ten minutes of its JWT `exp` is refreshed
//! (`refreshToken: true`). A refresh whose answer does not validate is
//! read once more without refreshing — a half-written `auth.json` reads
//! as "no account" for an instant. A token the provider rejects (401) is
//! refreshed once, and only if no other request already replaced it.
//!
//! **One `CodexAuth` per home, process-wide** ([`CodexAuth::shared`]).
//! The mutex only serialises callers of one instance, so two publications
//! that each built their own over the same home could refresh the same
//! refresh token twice.
//!
//! Nothing read from the app-server is logged: its lines carry the token.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use zeroize::Zeroize;

use super::super::secret::Secret;

/// Env that would make the app-server use another credential or another
/// token service than the home's ChatGPT login.
const SCRUBBED_ENV: [&str; 6] = [
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "CODEX_ACCESS_TOKEN",
    "CODEX_APP_SERVER_LOGIN_CLIENT_ID",
    "CODEX_REFRESH_TOKEN_URL_OVERRIDE",
    "CODEX_REVOKE_TOKEN_URL_OVERRIDE",
];

const MAX_RPC_LINE: usize = 1024 * 1024;
const REFRESH_WITHIN: Duration = Duration::from_secs(600);
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(30);

/// Why no token. None of these carries app-server text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodexAuthError {
    #[error("codex app-server could not be started")]
    Spawn,
    #[error("codex app-server did not answer in time")]
    Timeout,
    #[error("codex app-server spoke an unexpected protocol")]
    Protocol,
    #[error("codex app-server returned an error")]
    Rpc,
    #[error("the relay's codex home is not signed in with ChatGPT")]
    NotSignedIn,
    #[error("the codex token carries no valid ChatGPT account id")]
    BadAccount,
    #[error("the relay's codex token is expired and could not be refreshed")]
    Expired,
}

/// A Codex access token and the account it acts for.
#[derive(Clone, Debug)]
pub struct CodexToken {
    pub token: Secret,
    pub account_id: String,
    /// JWT `exp`, seconds since the epoch; `None` when absent.
    pub exp: Option<u64>,
}

impl CodexToken {
    fn fresh_at(&self, now: u64) -> bool {
        self.exp
            .is_none_or(|exp| exp > now + REFRESH_WITHIN.as_secs())
    }

    /// Not yet expired: usable, if perhaps soon due for a refresh.
    fn unexpired_at(&self, now: u64) -> bool {
        self.exp.is_none_or(|exp| exp > now)
    }
}

/// The relay's Codex home: `<data dir>/relay-codex`.
pub fn default_home() -> PathBuf {
    crate::paths::claudepot_data_dir().join("relay-codex")
}

/// Create the home owner-only. It holds Codex's `auth.json`.
///
/// On Windows the directory's DACL is replaced with one ACE for the
/// current user, protected from inheritance
/// ([`crate::secure_perms::harden_user_only`]), so a wide ACL inherited
/// from the parent does not carry over to it. That ACE is itself not
/// inheritable: a file codex creates inside gets the creating token's
/// default DACL, not the parent's.
pub fn ensure_home(home: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    crate::secure_perms::harden_user_only(home)?;
    Ok(())
}

/// How long a rejected token's refresh outcome answers every later
/// rejection of that same token, instead of another refresh.
const REFRESH_RETRY_AFTER: Duration = Duration::from_secs(30);

/// **Every call runs in its own task** ([`CodexAuth::run`]): a caller
/// dropped mid-refresh — a client that disconnected — must not kill the
/// app-server between spending the refresh token and saving its
/// successor, nor release the mutex while that is in flight. The task
/// keeps both until the exchange ends; the dropped caller only stops
/// waiting for it.
pub struct CodexAuth {
    inner: Arc<Inner>,
}

struct Inner {
    bin: PathBuf,
    home: PathBuf,
    deadline: Duration,
    state: tokio::sync::Mutex<State>,
}

#[derive(Default)]
struct State {
    cached: Option<CodexToken>,
    /// The last rejected token and what refreshing it gave, success or
    /// failure, so concurrent rejections of one token share one refresh.
    last_rejection: Option<Rejection>,
}

struct Rejection {
    token: Secret,
    at: std::time::Instant,
    outcome: Result<CodexToken, CodexAuthError>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl CodexAuth {
    pub fn new(bin: impl Into<PathBuf>, home: impl Into<PathBuf>) -> Self {
        Self::with_deadline(bin, home, DEFAULT_DEADLINE)
    }

    pub fn with_deadline(
        bin: impl Into<PathBuf>,
        home: impl Into<PathBuf>,
        deadline: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                bin: bin.into(),
                home: home.into(),
                deadline,
                state: tokio::sync::Mutex::new(State::default()),
            }),
        }
    }

    /// Run `f` as its own task and wait for it. A task that panicked is a
    /// protocol failure, never a token.
    async fn run<F, Fut>(&self, f: F) -> Result<CodexToken, CodexAuthError>
    where
        F: FnOnce(Arc<Inner>) -> Fut,
        Fut: std::future::Future<Output = Result<CodexToken, CodexAuthError>> + Send + 'static,
    {
        tokio::spawn(f(self.inner.clone()))
            .await
            .unwrap_or(Err(CodexAuthError::Protocol))
    }

    /// The one instance for `home` in this process, created on first use.
    /// A later caller's `bin` is ignored: the home, not the binary, is
    /// what owns the refresh token.
    ///
    /// The registry also keeps the [`Inner`], weakly: a task still running
    /// after its caller dropped the last handle holds it, and a new handle
    /// must share that task's mutex, or two refreshes could spend the same
    /// refresh token at once.
    pub fn shared(bin: impl Into<PathBuf>, home: impl Into<PathBuf>) -> Arc<Self> {
        let home = home.into();
        let mut registry = registry().lock();
        if let Some((auth, inner)) = registry.get(&home) {
            if let Some(live) = auth.upgrade() {
                return live;
            }
            if let Some(inner) = inner.upgrade() {
                let auth = Arc::new(Self { inner });
                if let Some(entry) = registry.get_mut(&home) {
                    entry.0 = Arc::downgrade(&auth);
                }
                return auth;
            }
        }
        registry.retain(|_, (a, i)| a.strong_count() > 0 || i.strong_count() > 0);
        let auth = Arc::new(Self::new(bin, home.clone()));
        registry.insert(home, (Arc::downgrade(&auth), Arc::downgrade(&auth.inner)));
        auth
    }

    /// A token valid for at least the next ten minutes.
    pub async fn token(&self) -> Result<CodexToken, CodexAuthError> {
        self.run(|i| async move { i.token().await }).await
    }

    /// [`Self::token`] without the cache: ask the app-server again, so a
    /// home signed in afresh is seen. Health's background probe.
    pub async fn reread(&self) -> Result<CodexToken, CodexAuthError> {
        self.run(|i| async move { i.reread().await }).await
    }

    /// The provider rejected `rejected`. Refresh once — unless another
    /// request already replaced it, in which case use that, or already
    /// tried to within [`REFRESH_RETRY_AFTER`], in which case share what
    /// that attempt gave (a failure, or an unchanged token).
    pub async fn after_rejection(&self, rejected: &Secret) -> Result<CodexToken, CodexAuthError> {
        let rejected = rejected.clone();
        self.run(|i| async move { i.after_rejection(rejected).await })
            .await
    }
}

/// Each home's handle and the state behind it, both weak.
type Registry = parking_lot::Mutex<HashMap<PathBuf, (Weak<CodexAuth>, Weak<Inner>)>>;

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

impl Inner {
    async fn token(&self) -> Result<CodexToken, CodexAuthError> {
        let mut state = self.state.lock().await;
        if let Some(t) = state.cached.as_ref() {
            if t.fresh_at(now_secs()) {
                return Ok(t.clone());
            }
        }
        let t = self.read_fresh().await?;
        state.cached = Some(t.clone());
        Ok(t)
    }

    async fn reread(&self) -> Result<CodexToken, CodexAuthError> {
        let mut state = self.state.lock().await;
        let t = self.read_fresh().await?;
        state.cached = Some(t.clone());
        Ok(t)
    }

    async fn read_fresh(&self) -> Result<CodexToken, CodexAuthError> {
        let t = self.fetch(false).await?;
        if t.fresh_at(now_secs()) {
            return Ok(t);
        }
        self.refresh().await
    }

    async fn after_rejection(&self, rejected: Secret) -> Result<CodexToken, CodexAuthError> {
        let mut state = self.state.lock().await;
        if let Some(t) = state.cached.as_ref() {
            if t.token != rejected {
                return Ok(t.clone());
            }
        }
        if let Some(r) = state.last_rejection.as_ref() {
            if r.token == rejected && r.at.elapsed() < REFRESH_RETRY_AFTER {
                return r.outcome.clone();
            }
        }
        state.cached = None;
        let outcome = self.refresh().await;
        if let Ok(t) = &outcome {
            state.cached = Some(t.clone());
        }
        state.last_rejection = Some(Rejection {
            token: rejected,
            at: std::time::Instant::now(),
            outcome: outcome.clone(),
        });
        outcome
    }

    /// Refresh, falling back to a plain read; either way an expired
    /// token is refused, never handed out as a credential.
    async fn refresh(&self) -> Result<CodexToken, CodexAuthError> {
        let t = match self.fetch(true).await {
            Ok(t) => t,
            Err(CodexAuthError::NotSignedIn | CodexAuthError::BadAccount) => {
                self.fetch(false).await?
            }
            Err(e) => return Err(e),
        };
        if !t.unexpired_at(now_secs()) {
            return Err(CodexAuthError::Expired);
        }
        Ok(t)
    }

    async fn fetch(&self, refresh: bool) -> Result<CodexToken, CodexAuthError> {
        // Before codex can create it with its own (wider) mode.
        ensure_home(&self.home).map_err(|_| CodexAuthError::Spawn)?;
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.args([
            "-c",
            "model_provider=\"openai\"",
            "app-server",
            "--listen",
            "stdio://",
        ])
        .env("CODEX_HOME", &self.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
        for k in SCRUBBED_ENV {
            cmd.env_remove(k);
        }
        let mut child = cmd.spawn().map_err(|_| CodexAuthError::Spawn)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(CodexAuthError::Spawn);
        };
        let result = tokio::time::timeout(self.deadline, converse(stdin, stdout, refresh))
            .await
            .unwrap_or(Err(CodexAuthError::Timeout));
        let _ = child.start_kill();
        let _ = child.wait().await;
        result
    }
}

async fn converse(
    mut stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    refresh: bool,
) -> Result<CodexToken, CodexAuthError> {
    let mut out = BufReader::new(stdout);
    call(
        &mut stdin,
        &mut out,
        "1",
        "initialize",
        json!({"clientInfo": {"name": "claudepot-relay", "version": "1"}, "capabilities": {}}),
    )
    .await?;
    send(&mut stdin, &json!({"method": "initialized"})).await?;
    let status = call(
        &mut stdin,
        &mut out,
        "2",
        "getAuthStatus",
        json!({"includeToken": true, "refreshToken": refresh}),
    )
    .await?;
    token_from_status(status)
}

async fn send(stdin: &mut tokio::process::ChildStdin, v: &Value) -> Result<(), CodexAuthError> {
    let mut line = serde_json::to_vec(v).map_err(|_| CodexAuthError::Protocol)?;
    line.push(b'\n');
    stdin
        .write_all(&line)
        .await
        .map_err(|_| CodexAuthError::Protocol)?;
    stdin.flush().await.map_err(|_| CodexAuthError::Protocol)
}

/// One request; lines until its answer. A request *from* the server is
/// declined, as clcd does; notifications are skipped.
async fn call(
    stdin: &mut tokio::process::ChildStdin,
    out: &mut BufReader<tokio::process::ChildStdout>,
    id: &str,
    method: &str,
    params: Value,
) -> Result<Value, CodexAuthError> {
    send(
        stdin,
        &json!({"id": id, "method": method, "params": params}),
    )
    .await?;
    let mut buf = Vec::new();
    loop {
        buf.zeroize();
        buf.clear();
        let n = (&mut *out)
            .take(MAX_RPC_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
            .await
            .map_err(|_| CodexAuthError::Protocol)?;
        if n == 0 || buf.len() > MAX_RPC_LINE {
            buf.zeroize();
            return Err(CodexAuthError::Protocol);
        }
        let parsed: Option<Value> = serde_json::from_slice(&buf).ok();
        buf.zeroize();
        let Some(Value::Object(mut msg)) = parsed else {
            continue;
        };
        let is_ours = msg.get("id").and_then(Value::as_str) == Some(id);
        if is_ours {
            if msg.contains_key("error") {
                return Err(CodexAuthError::Rpc);
            }
            if let Some(result) = msg.remove("result") {
                return Ok(result);
            }
            continue;
        }
        if let (Some(other), Some(Value::String(_))) = (msg.get("id"), msg.get("method")) {
            let decline =
                json!({"id": other, "error": {"code": -32601, "message": "Not supported"}});
            send(stdin, &decline).await?;
        }
    }
}

fn token_from_status(mut status: Value) -> Result<CodexToken, CodexAuthError> {
    let chatgpt = status.get("authMethod").and_then(Value::as_str) == Some("chatgpt");
    let requires = status.get("requiresOpenaiAuth").and_then(Value::as_bool) != Some(false);
    let token = match status.get_mut("authToken").map(Value::take) {
        Some(Value::String(t)) if !t.is_empty() => Secret::new(t),
        _ => return Err(CodexAuthError::NotSignedIn),
    };
    if !chatgpt || !requires {
        return Err(CodexAuthError::NotSignedIn);
    }
    let claims = jwt_claims(token.expose()).ok_or(CodexAuthError::BadAccount)?;
    let account_id = claims
        .get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|a| valid_account_id(a))
        .ok_or(CodexAuthError::BadAccount)?
        .to_owned();
    let exp = claims.get("exp").and_then(Value::as_u64).filter(|e| *e > 0);
    Ok(CodexToken {
        token,
        account_id,
        exp,
    })
}

fn jwt_claims(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?.trim_end_matches('=');
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// `^[A-Za-z0-9_.-]{1,256}$` — the account id becomes a header value.
fn valid_account_id(a: &str) -> bool {
    (1..=256).contains(&a.len())
        && a.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn jwt(account: &str, exp: u64) -> String {
        let enc = |v: Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string().as_bytes())
        };
        format!(
            "{}.{}.sig",
            enc(json!({"alg": "none"})),
            enc(
                json!({"exp": exp, "https://api.openai.com/auth": {"chatgpt_account_id": account}})
            )
        )
    }

    #[test]
    fn a_chatgpt_status_yields_the_token_and_account() {
        let t = token_from_status(json!({
            "authMethod": "chatgpt", "authToken": jwt("acct_1", 4_000_000_000),
        }))
        .unwrap();
        assert_eq!(t.account_id, "acct_1");
        assert_eq!(t.exp, Some(4_000_000_000));
        assert!(t.fresh_at(now_secs()));
    }

    #[test]
    fn an_api_key_login_or_a_bad_account_is_refused() {
        let api = token_from_status(json!({"authMethod": "apikey", "authToken": "sk-x"}));
        assert_eq!(api.unwrap_err(), CodexAuthError::NotSignedIn);
        let none = token_from_status(json!({"authMethod": null}));
        assert_eq!(none.unwrap_err(), CodexAuthError::NotSignedIn);
        let bad = token_from_status(json!({
            "authMethod": "chatgpt", "authToken": jwt("acct\r\nX-Evil: 1", 4_000_000_000),
        }));
        assert_eq!(bad.unwrap_err(), CodexAuthError::BadAccount);
        let opaque = token_from_status(json!({"authMethod": "chatgpt", "authToken": "opaque"}));
        assert_eq!(opaque.unwrap_err(), CodexAuthError::BadAccount);
    }

    #[test]
    fn a_token_near_expiry_is_not_fresh() {
        let now = now_secs();
        let t = token_from_status(json!({
            "authMethod": "chatgpt", "authToken": jwt("a", now + 60),
        }))
        .unwrap();
        assert!(!t.fresh_at(now));
    }

    #[test]
    fn errors_never_carry_app_server_text() {
        for e in [
            CodexAuthError::Spawn,
            CodexAuthError::Timeout,
            CodexAuthError::Protocol,
            CodexAuthError::Rpc,
            CodexAuthError::NotSignedIn,
            CodexAuthError::BadAccount,
            CodexAuthError::Expired,
        ] {
            assert!(!e.to_string().contains("eyJ"));
        }
    }

    /// A fake `codex app-server`: answers initialize, declines nothing,
    /// sends one server request first, and returns `$STALE` for a plain
    /// read and `$FRESH` for a refresh. Each getAuthStatus appends a line
    /// to `calls`.
    #[cfg(unix)]
    pub(crate) fn fake_codex(dir: &Path, stale: &str, fresh: &str) -> PathBuf {
        fake_codex_with(dir, stale, fresh, 0)
    }

    /// [`fake_codex`] whose refresh first sleeps `refresh_delay` seconds,
    /// and records the refresh only after it. A `fresh` of `ERR` answers
    /// the refresh with an RPC error.
    #[cfg(unix)]
    fn fake_codex_with(dir: &Path, stale: &str, fresh: &str, refresh_delay: u32) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = format!(
            r#"#!/bin/sh
[ -n "$OPENAI_API_KEY" ] && exit 9
[ -z "$CODEX_HOME" ] && exit 8
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      echo '{{"method":"note"}}'
      echo "{{\"id\":\"$id\",\"result\":{{}}}}" ;;
    *getAuthStatus*)
      echo '{{"id":"srv-1","method":"askUser","params":{{}}}}'
      read -r reply
      case "$reply" in *-32601*) ;; *) exit 7 ;; esac
      case "$line" in
        *'"refreshToken":true'*) sleep {refresh_delay}; echo refresh >> "$CODEX_HOME/calls"; tok='{fresh}' ;;
        *) echo read >> "$CODEX_HOME/calls"; tok='{stale}' ;;
      esac
      if [ "$tok" = ERR ]; then
        echo "{{\"id\":\"$id\",\"error\":{{\"code\":1,\"message\":\"no\"}}}}"
      else
        echo "{{\"id\":\"$id\",\"result\":{{\"authMethod\":\"chatgpt\",\"authToken\":\"$tok\"}}}}"
      fi ;;
  esac
done
"#
        );
        let path = dir.join("codex");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    pub(crate) fn calls(home: &Path) -> Vec<String> {
        std::fs::read_to_string(home.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_fresh_token_is_read_once_and_cached() {
        let dir = tempfile::tempdir().unwrap();
        let good = jwt("acct", now_secs() + 3600);
        let bin = fake_codex(dir.path(), &good, "unused");
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::new(bin, &home);
        let t = auth.token().await.unwrap();
        assert_eq!(t.token.expose(), good);
        auth.token().await.unwrap();
        assert_eq!(calls(&home), ["read"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_token_near_expiry_is_refreshed() {
        let dir = tempfile::tempdir().unwrap();
        let stale = jwt("acct", now_secs() + 60);
        let fresh = jwt("acct", now_secs() + 3600);
        let bin = fake_codex(dir.path(), &stale, &fresh);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::new(bin, &home);
        assert_eq!(auth.token().await.unwrap().token.expose(), fresh);
        assert_eq!(calls(&home), ["read", "refresh"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_rejections_of_one_token_refresh_once() {
        let dir = tempfile::tempdir().unwrap();
        let first = jwt("acct", now_secs() + 3600);
        let second = jwt("acct", now_secs() + 7200);
        let bin = fake_codex(dir.path(), &first, &second);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = std::sync::Arc::new(CodexAuth::new(bin, &home));
        let rejected = auth.token().await.unwrap().token;
        let (a, b) = tokio::join!(
            auth.after_rejection(&rejected),
            auth.after_rejection(&rejected)
        );
        assert_eq!(a.unwrap().token.expose(), second);
        assert_eq!(b.unwrap().token.expose(), second);
        assert_eq!(calls(&home), ["read", "refresh"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_silent_app_server_times_out() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("codex");
        std::fs::write(&bin, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let auth = CodexAuth::with_deadline(bin, dir.path(), Duration::from_millis(300));
        assert_eq!(auth.token().await.unwrap_err(), CodexAuthError::Timeout);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn two_shared_handles_for_one_home_refresh_a_rejected_token_once() {
        let dir = tempfile::tempdir().unwrap();
        let first = jwt("acct", now_secs() + 3600);
        let second = jwt("acct", now_secs() + 7200);
        let bin = fake_codex(dir.path(), &first, &second);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        // Two publications of the same Codex home, built independently.
        let a = CodexAuth::shared(&bin, &home);
        let b = CodexAuth::shared(&bin, &home);
        assert!(Arc::ptr_eq(&a, &b), "one instance per home");
        let rejected = a.token().await.unwrap().token;
        let (ra, rb) = tokio::join!(a.after_rejection(&rejected), b.after_rejection(&rejected));
        assert_eq!(ra.unwrap().token.expose(), second);
        assert_eq!(rb.unwrap().token.expose(), second);
        assert_eq!(calls(&home), ["read", "refresh"]);
        let other = CodexAuth::shared(&bin, dir.path().join("other"));
        assert!(!Arc::ptr_eq(&a, &other), "another home is another owner");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_reread_asks_the_app_server_even_with_a_cached_token() {
        let dir = tempfile::tempdir().unwrap();
        let good = jwt("acct", now_secs() + 3600);
        let bin = fake_codex(dir.path(), &good, "unused");
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::new(bin, &home);
        auth.token().await.unwrap();
        auth.reread().await.unwrap();
        auth.token().await.unwrap();
        assert_eq!(calls(&home), ["read", "read"]);
    }

    /// The last handle dropped while its refresh still runs: a handle
    /// taken at once must share that refresh's mutex, not start another
    /// that spends the same refresh token.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_handle_reacquired_while_a_dropped_refresh_runs_shares_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = jwt("acct", now_secs() + 3600);
        let second = jwt("acct", now_secs() + 7200);
        let bin = fake_codex_with(dir.path(), &first, &second, 1);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::shared(&bin, &home);
        let rejected = auth.token().await.unwrap().token;
        let gave_up =
            tokio::time::timeout(Duration::from_millis(300), auth.after_rejection(&rejected)).await;
        assert!(gave_up.is_err(), "the caller was dropped mid-refresh");
        drop(auth);
        let again = CodexAuth::shared(&bin, &home);
        // Waits for the running refresh, then sees its token.
        let t = again.after_rejection(&rejected).await.unwrap();
        assert_eq!(t.token.expose(), second);
        assert_eq!(calls(&home), ["read", "refresh"]);
    }

    /// A refresh that gives back a token already past its `exp` — the
    /// refresh failed to move it on — is refused, not handed out.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_expired_token_after_refresh_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let expired = jwt("acct", now_secs().saturating_sub(60));
        let bin = fake_codex(dir.path(), &expired, &expired);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::new(bin, &home);
        assert_eq!(auth.token().await.unwrap_err(), CodexAuthError::Expired);
        assert_eq!(calls(&home), ["read", "refresh"]);
        // A rejection's refresh is held to the same rule.
        let rejected = Secret::new(expired.clone());
        assert_eq!(
            auth.after_rejection(&rejected).await.unwrap_err(),
            CodexAuthError::Expired
        );
    }

    /// Near expiry but not past it is still a usable token.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unexpired_token_after_refresh_is_kept_even_if_soon_due() {
        let dir = tempfile::tempdir().unwrap();
        let soon = jwt("acct", now_secs() + 60);
        let bin = fake_codex(dir.path(), &soon, &soon);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::new(bin, &home);
        assert_eq!(auth.token().await.unwrap().token.expose(), soon);
    }

    #[cfg(windows)]
    #[test]
    fn the_home_is_created_and_hardened_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        assert!(home.is_dir());
        // Idempotent: a second call re-applies the same DACL.
        ensure_home(&home).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_caller_dropped_mid_refresh_does_not_abort_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = jwt("acct", now_secs() + 3600);
        let second = jwt("acct", now_secs() + 7200);
        let bin = fake_codex_with(dir.path(), &first, &second, 1);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::new(bin, &home);
        let rejected = auth.token().await.unwrap().token;
        let gave_up =
            tokio::time::timeout(Duration::from_millis(300), auth.after_rejection(&rejected)).await;
        assert!(gave_up.is_err(), "the caller was dropped mid-refresh");
        // The refresh finished without its caller, and its token is kept.
        assert_eq!(auth.token().await.unwrap().token.expose(), second);
        assert_eq!(calls(&home), ["read", "refresh"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_rejections_share_one_failed_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let first = jwt("acct", now_secs() + 3600);
        let bin = fake_codex(dir.path(), &first, "ERR");
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::new(bin, &home);
        let rejected = auth.token().await.unwrap().token;
        let (a, b) = tokio::join!(
            auth.after_rejection(&rejected),
            auth.after_rejection(&rejected)
        );
        assert_eq!(a.unwrap_err(), CodexAuthError::Rpc);
        assert_eq!(b.unwrap_err(), CodexAuthError::Rpc);
        assert_eq!(calls(&home), ["read", "refresh"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_rejections_share_one_unchanged_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let first = jwt("acct", now_secs() + 3600);
        let bin = fake_codex(dir.path(), &first, &first);
        let home = dir.path().join("home");
        ensure_home(&home).unwrap();
        let auth = CodexAuth::new(bin, &home);
        let rejected = auth.token().await.unwrap().token;
        let (a, b) = tokio::join!(
            auth.after_rejection(&rejected),
            auth.after_rejection(&rejected)
        );
        assert_eq!(a.unwrap().token.expose(), first);
        assert_eq!(b.unwrap().token.expose(), first);
        assert_eq!(calls(&home), ["read", "refresh"]);
    }

    #[tokio::test]
    async fn a_missing_binary_is_a_spawn_error() {
        let dir = tempfile::tempdir().unwrap();
        let auth = CodexAuth::new(dir.path().join("no-such-codex"), dir.path());
        assert_eq!(auth.token().await.unwrap_err(), CodexAuthError::Spawn);
    }
}

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SwapError {
    #[error("no stored credentials for account {0}")]
    NoStoredCredentials(uuid::Uuid),

    #[error("no default credentials found in CC storage")]
    NoDefaultCredentials,

    #[error("failed to write credentials: {0}")]
    WriteFailed(String),

    #[error("keychain operation failed: {0}")]
    KeychainError(String),

    #[error("file operation failed: {0}")]
    FileError(#[from] std::io::Error),

    #[error("corrupt credential blob: {0}")]
    CorruptBlob(String),

    #[error("token refresh failed: {0}")]
    RefreshFailed(String),

    #[error(
        "identity mismatch: account {stored_email} holds credentials for {actual_email}. \
         Run `claudepot account remove {stored_email}` and re-add to re-seed."
    )]
    IdentityMismatch {
        stored_email: String,
        actual_email: String,
    },

    #[error(
        "a Claude Code process is running — its token refresh will overwrite this swap. \
         Quit Claude Code first, or pass --force to proceed anyway."
    )]
    LiveSessionConflict,

    #[error("identity verification failed: {0}")]
    IdentityVerificationFailed(String),
}

#[derive(Error, Debug)]
pub enum DesktopSwapError {
    #[error("Claude Desktop is still running after quit timeout")]
    DesktopStillRunning,

    #[error("no desktop profile stored for account {0}")]
    NoStoredProfile(uuid::Uuid),

    #[error("file copy failed: {0}")]
    FileCopyFailed(String),

    #[error("desktop not installed on this platform")]
    NotInstalled,

    /// Windows-only. Detected at pre-restore by
    /// `desktop_service::check_profile_dpapi_valid`. Means the
    /// stored profile's ciphertext was encrypted under a different
    /// DPAPI master key than the one this Windows session currently
    /// holds, so Chromium on next launch would reject the cookies /
    /// tokens as corrupt. Surfaced to the user as "re-sign in to
    /// Claude Desktop on this machine; Claudepot will re-bind the
    /// fresh session." Never fires on macOS.
    #[error(
        "Desktop profile encrypted under different Windows credentials \
         (different machine, different user, or password reset) — \
         sign in to Claude Desktop fresh, then re-bind."
    )]
    DpapiInvalidated,

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum OAuthError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("token expired and refresh failed: {0}")]
    RefreshFailed(String),

    #[error("rate limited — retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    /// Non-401, non-429 non-2xx response from an OAuth endpoint. Separate
    /// from AuthFailed so callers (identity verification in particular)
    /// can distinguish "token is genuinely bad" from "the server had a
    /// bad minute" — the former is a `Rejected` outcome requiring
    /// re-login; the latter is `NetworkError` and should NOT wipe the
    /// verified_email history.
    #[error("OAuth server error: {0}")]
    ServerError(String),
}

#[derive(Error, Debug)]
pub enum ProjectError {
    #[error("project not found: {0}")]
    NotFound(String),

    #[error("old and new paths are the same")]
    SamePath,

    #[error("ambiguous: {0}")]
    Ambiguous(String),

    #[error("a claude process is running in {0} — use --force to proceed")]
    ClaudeRunning(String),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum LauncherError {
    #[error("no stored credentials for account {0}")]
    NoStoredCredentials(uuid::Uuid),

    #[error("corrupt credential blob: {0}")]
    CorruptBlob(String),

    #[error("token refresh failed: {0}")]
    RefreshFailed(String),

    #[error("failed to save refreshed credentials: {0}")]
    SaveFailed(String),

    #[error("no command specified")]
    NoCommand,

    #[error("spawn failed: {0}")]
    SpawnFailed(String),
}

#[derive(Error, Debug)]
pub enum OnboardError {
    #[error("claude CLI not found at {0}")]
    CliBinaryNotFound(String),

    #[error("{}", match *.0 {
        -2 => "login timed out — close the Claudepot window and try again, or complete the browser flow faster".to_string(),
        code => format!("`claude auth login` exited with code {code}"),
    })]
    AuthLoginFailed(i32),

    #[error("login cancelled")]
    AuthLoginCancelled,

    #[error("import failed: no credentials at hashed service name for {0}")]
    ImportFailed(String),

    #[error("{0}")]
    Swap(#[from] SwapError),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

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

    #[error("auth login failed with exit code {0}")]
    AuthLoginFailed(i32),

    #[error("import failed: no credentials at hashed service name for {0}")]
    ImportFailed(String),

    #[error("{0}")]
    Swap(#[from] SwapError),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

//! Machine identity for boundary errors — the half of an error the GUI
//! can translate.
//!
//! Core reports data; presentation layers own words. A `#[error("…")]`
//! string stays the canonical English text — the CLI prints it verbatim
//! and it travels to the GUI as `ErrorDto.message`, the fallback for a
//! code with no catalog entry. What this trait *adds* is a stable
//! identity (`code`) plus the values the message interpolates
//! (`params`), so a localized sentence can be re-composed instead of
//! translated string-by-string.
//!
//! Rules, all of them load-bearing:
//!
//! - **Hand-write both methods.** No derive. `code()` is an exhaustive
//!   `match` with no wildcard arm, so a new variant fails to compile
//!   until someone decides what it is called — a derive would mint a
//!   code from the variant name and let a rename silently orphan a
//!   translation.
//! - **Code format is `<module>.<variant_snake_case>`** —
//!   `keys.not_found`, `oauth.rate_limited`, `project.claude_running`.
//!   The module segment is the owning module, not the file.
//! - **`params` carries what the message interpolates**, structured:
//!   a path as `path`, a uuid as `uuid`, a duration as
//!   `retry_after_secs`. A translator writing "还有 {{retry_after_secs}}
//!   秒" must not have to parse an English sentence to find the number.
//! - **Never put a secret in `params`.** `params` crosses the IPC
//!   bridge and lands in the JS heap; `rules/rust-conventions.md`'s
//!   token rule applies here exactly as it does to logs.
//! - **Never reword an existing `#[error("…")]`** to suit a code. The
//!   CLI's output is frozen English; `ProjectError::ClaudeRunning`
//!   keeps saying `use --force` for that reason, and exposes the path
//!   in `params` so a GUI sentence can drop the flag it has no use for.
//!
//! Every code is registered in
//! `crates/claudepot-core/testdata/error-code-vectors.json`; the tests
//! there lock the code set and the param names in both directions.

use serde_json::Value;

/// Translatable identity for one boundary error.
///
/// Implemented on error enums that reach a user surface. `Display`
/// (via `thiserror`) remains the English text; this is the machine
/// half beside it.
pub trait ErrorCode {
    /// Stable `<module>.<variant_snake_case>` identity. Never derived,
    /// never computed from the variant name at runtime.
    fn code(&self) -> &'static str;

    /// JSON **object** of the values the English message interpolates.
    /// Variants that interpolate nothing return an empty object, never
    /// `Value::Null` — the DTO field is typed as a map on the JS side.
    fn params(&self) -> Value;
}

#[cfg(test)]
mod tests {

    //! The golden-vectors lock. `error-code-vectors.json` is the
    //! cross-language contract (this test owns the Rust half; the
    //! catalog-completeness check in Vitest owns the TS half), and the
    //! sample lists below are what make it exhaustive: a new variant
    //! breaks `code()`'s wildcard-free match first, then the
    //! destructuring match in its `all_*` sampler, then this test.

    use super::*;
    use crate::keys::KeyError;
    use crate::oauth::error::OAuthError;
    use crate::project::ProjectError;
    use std::collections::BTreeSet;

    /// Scan a serialized `params()` blob with the same detector bank
    /// the redaction path uses, rather than a hand-written `sk-ant-`
    /// substring. A `params` leak is not limited to Anthropic tokens —
    /// a provider key in a route or env error is the same failure, and
    /// the narrow check could not see it.
    ///
    /// `aws_secret_key` is excluded on purpose: it is the bank's broad
    /// unprefixed 40-char rule and its character class includes `/`, so
    /// it matches ordinary filesystem paths — and paths are exactly what
    /// these params legitimately carry.
    fn secret_hits(text: &str) -> Vec<&'static str> {
        crate::secret_patterns::provider_rules()
            .iter()
            .filter(|r| r.name != "aws_secret_key")
            .filter(|r| r.re.is_match(text))
            .map(|r| r.name)
            .collect()
    }

    #[derive(serde::Deserialize)]
    struct VectorFile {
        codes: Vec<CodeVector>,
    }

    #[derive(serde::Deserialize)]
    struct CodeVector {
        code: String,
        /// `"core"` (an enum variant in this crate) or `"command"` (an
        /// ad-hoc failure raised at a Tauri command site). This test
        /// owns the `core` half; `src-tauri/src/dto_error.rs` owns the
        /// `command` half.
        owner: String,
        params: Vec<String>,
        message: String,
    }

    fn vectors_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata")
            .join("error-codes")
    }

    /// Every `*.json` under `testdata/error-codes/`, merged.
    ///
    /// One file per domain rather than one registry file: the registry
    /// is appended to by whoever converts a module, and a single file
    /// serializes that work behind one editor. Splitting it costs this
    /// directory read and buys conflict-free parallel conversion. A
    /// duplicate code across two files is a hard error — two domains
    /// claiming one identity would make the translation ambiguous.
    fn load() -> Vec<CodeVector> {
        let dir = vectors_dir();
        let mut out: Vec<CodeVector> = Vec::new();
        let mut seen: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        // Sorted so a failure names the same file on every machine.
        entries.sort();
        assert!(!entries.is_empty(), "{} holds no vectors", dir.display());
        for path in entries {
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let file: VectorFile =
                serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            for v in file.codes {
                if let Some(prev) = seen.insert(v.code.clone(), name.clone()) {
                    panic!("duplicate code {} in {name} and {prev}", v.code);
                }
                out.push(v);
            }
        }
        out
    }

    /// One instance of every `KeyError` variant. The trailing match is
    /// wildcard-free on purpose: adding a variant stops compilation
    /// here, which is what forces it into the vectors file.
    fn all_key_errors() -> Vec<KeyError> {
        let samples = vec![
            KeyError::Sql(rusqlite::Error::QueryReturnedNoRows),
            KeyError::Io(std::io::Error::other("permission denied")),
            KeyError::Keychain("keychain is locked".to_string()),
            KeyError::NotFound("6f9a1c2e-0000-4000-8000-000000000000".to_string()),
        ];
        for s in &samples {
            match s {
                KeyError::Sql(_)
                | KeyError::Io(_)
                | KeyError::Keychain(_)
                | KeyError::NotFound(_) => {}
            }
        }
        samples
    }

    /// See `all_key_errors` for why the trailing match exists.
    fn all_oauth_errors() -> Vec<OAuthError> {
        let samples = vec![
            OAuthError::RefreshFailed("token endpoint rejected refresh_token with 401".to_string()),
            OAuthError::RateLimited {
                retry_after_secs: 60,
            },
            OAuthError::AuthFailed("api key rejected by /v1/models".to_string()),
            OAuthError::ServerError("/v1/models returned 503".to_string()),
        ];
        for s in &samples {
            match s {
                // `HttpError` is sampled separately — `reqwest::Error`
                // has no public constructor, so it is covered by the
                // code-set assertion below rather than by an instance.
                OAuthError::HttpError(_)
                | OAuthError::RefreshFailed(_)
                | OAuthError::RateLimited { .. }
                | OAuthError::AuthFailed(_)
                | OAuthError::ServerError(_) => {}
            }
        }
        samples
    }

    /// See `all_key_errors` for why the trailing match exists.
    fn all_project_errors() -> Vec<ProjectError> {
        let samples = vec![
            ProjectError::NotFound("/Users/me/proj".to_string()),
            ProjectError::SamePath,
            ProjectError::Ambiguous("old path contains invalid UTF-8".to_string()),
            ProjectError::ClaudeRunning("/Users/me/proj".to_string()),
            ProjectError::Io(std::io::Error::other("no such file or directory")),
        ];
        for s in &samples {
            match s {
                ProjectError::NotFound(_)
                | ProjectError::SamePath
                | ProjectError::Ambiguous(_)
                | ProjectError::ClaudeRunning(_)
                | ProjectError::Io(_) => {}
            }
        }
        samples
    }

    /// Samplers for the agent / board / env-vault / routes / MCP / PR
    /// domain (`testdata/error-codes/40-agent.json`). Kept in one
    /// module so the domain's slice of `core_samples` is a single line
    /// and parallel conversions do not collide inside this file.
    mod agent_domain {
        use super::*;

        /// A `serde_json::Error` — the crate has no public
        /// constructor, so parse something that cannot succeed.
        pub fn json_err() -> serde_json::Error {
            serde_json::from_str::<u32>("nope").expect_err("not a u32")
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_agent_errors() -> Vec<crate::agent::AgentError> {
            use crate::agent::AgentError as E;
            let samples = vec![
                E::Io(std::io::Error::other("permission denied")),
                E::Json(json_err()),
                E::NotFound("6f9a1c2e-0000-4000-8000-000000000000".to_string()),
                E::RouteNotFound("9f2c1d80-0000-4000-8000-000000000000".to_string()),
                E::DuplicateName("nightly-usage".to_string()),
                E::InvalidName("Nightly Usage".to_string(), "must be lowercase"),
                E::InvalidCron("* * *".to_string(), "expected 5 fields".to_string()),
                E::CronTooDense("* * * * *".to_string(), 1440, 96),
                E::InvalidEnv("key 'PATH' is set by Claudepot".to_string()),
                E::MissingField("prompt"),
                E::InvalidPath("relative/dir".to_string(), "must be absolute"),
                E::NoHomeDir,
                E::UnsupportedPlatform("scheduler register"),
                E::NotManaged("/Users/me/Library/LaunchAgents/x.plist".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Io(_)
                    | E::Json(_)
                    | E::NotFound(_)
                    | E::RouteNotFound(_)
                    | E::DuplicateName(_)
                    | E::InvalidName(_, _)
                    | E::InvalidCron(_, _)
                    | E::CronTooDense(_, _, _)
                    | E::InvalidEnv(_)
                    | E::MissingField(_)
                    | E::InvalidPath(_, _)
                    | E::NoHomeDir
                    | E::UnsupportedPlatform(_)
                    | E::NotManaged(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_agent_events_errors() -> Vec<crate::agent::events::store::AgentEventsError> {
            use crate::agent::events::store::AgentEventsError as E;
            let samples = vec![
                E::Io(std::io::Error::other("permission denied")),
                E::Serde(json_err()),
                E::UnsupportedSchemaVersion {
                    found: 9,
                    expected: 1,
                },
            ];
            for s in &samples {
                match s {
                    E::Io(_) | E::Serde(_) | E::UnsupportedSchemaVersion { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_board_errors() -> Vec<crate::board::BoardError> {
            use crate::board::BoardError as E;
            let samples = vec![
                E::Sql(rusqlite::Error::QueryReturnedNoRows),
                E::Io(std::io::Error::other("permission denied")),
                E::Json(json_err()),
                E::BoardNotFound("nightly-usage".to_string()),
                E::SeriesNotFound {
                    board: "nightly-usage".to_string(),
                    series: "tokens".to_string(),
                },
                E::DuplicateSeries {
                    board: "nightly-usage".to_string(),
                    series: "tokens".to_string(),
                },
                E::RevisionConflict {
                    board: "nightly-usage".to_string(),
                    base: 5,
                    current: 7,
                },
                E::ColumnTypeMismatch {
                    series: "tokens".to_string(),
                    column: "count".to_string(),
                    expected: "number",
                    actual: "text",
                },
                E::ColumnCountMismatch {
                    series: "tokens".to_string(),
                    expected: 3,
                    actual: 2,
                },
                E::InvalidName("bad name!".to_string()),
                E::InvalidTimestamp,
                E::SequenceReplay {
                    writer: "nightly-agent".to_string(),
                    series: "tokens".to_string(),
                    seq: 4,
                },
                E::CapExceeded {
                    what: "rows per push",
                    limit: 1000,
                    actual: 1500,
                },
                E::InvalidSpec("grid has no cells".to_string()),
                E::UnsupportedExportVersion {
                    found: 2,
                    expected: 1,
                },
                E::SchemaTooNew {
                    found: 9,
                    supported: 3,
                },
                E::SequenceOutOfRange { seq: 0, rows: 5 },
                E::InvalidEnvelope("unknown column type"),
                E::CorruptRow {
                    series: "tokens".to_string(),
                    what: "cell count does not match the series",
                },
            ];
            for s in &samples {
                match s {
                    E::Sql(_)
                    | E::Io(_)
                    | E::Json(_)
                    | E::BoardNotFound(_)
                    | E::SeriesNotFound { .. }
                    | E::DuplicateSeries { .. }
                    | E::RevisionConflict { .. }
                    | E::ColumnTypeMismatch { .. }
                    | E::ColumnCountMismatch { .. }
                    | E::InvalidName(_)
                    | E::InvalidTimestamp
                    | E::SequenceReplay { .. }
                    | E::CapExceeded { .. }
                    | E::InvalidSpec(_)
                    | E::UnsupportedExportVersion { .. }
                    | E::SchemaTooNew { .. }
                    | E::SequenceOutOfRange { .. }
                    | E::InvalidEnvelope(_)
                    | E::CorruptRow { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_vault_errors() -> Vec<crate::env_vault::VaultError> {
            use crate::env_vault::VaultError as E;
            let samples = vec![
                E::Sql(rusqlite::Error::QueryReturnedNoRows),
                E::Io(std::io::Error::other("permission denied")),
                E::NotFound("OPENAI_API_KEY".to_string()),
                E::DuplicateName("OPENAI_API_KEY".to_string()),
                E::InvalidName("9lives".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Sql(_)
                    | E::Io(_)
                    | E::NotFound(_)
                    | E::DuplicateName(_)
                    | E::InvalidName(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_env_file_errors() -> Vec<crate::env_vault::env_file::EnvEditError> {
            use crate::env_vault::env_file::EnvEditError as E;
            let samples = vec![
                E::KeyNotFound("OPENAI_API_KEY".to_string()),
                E::AlreadyActive("OPENAI_API_KEY".to_string()),
                E::AlreadyCommented("OPENAI_API_KEY".to_string()),
                E::InvalidKeyName("9lives".to_string()),
                E::ValueHasNewline("OPENAI_API_KEY".to_string()),
            ];
            for s in &samples {
                match s {
                    E::KeyNotFound(_)
                    | E::AlreadyActive(_)
                    | E::AlreadyCommented(_)
                    | E::InvalidKeyName(_)
                    | E::ValueHasNewline(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_route_errors() -> Vec<crate::routes::RouteError> {
            use crate::routes::RouteError as E;
            let samples = vec![
                E::Io(std::io::Error::other("permission denied")),
                E::Json(json_err()),
                E::NotFound("6f9a1c2e-0000-4000-8000-000000000000".to_string()),
                E::DuplicateName("kimi".to_string()),
                E::DuplicateWrapperName("claude-kimi".to_string()),
                E::WrapperShadowsClaude("claude".to_string()),
                E::WrapperFileNotManaged("/Users/me/.local/bin/claude-kimi".to_string()),
                E::InvalidWrapperName("claude kimi".to_string(), "not shell-safe".to_string()),
                E::MissingField("base_url"),
                E::NoHomeDir,
                E::UnsupportedPlatform("Desktop profile install"),
            ];
            for s in &samples {
                match s {
                    E::Io(_)
                    | E::Json(_)
                    | E::NotFound(_)
                    | E::DuplicateName(_)
                    | E::DuplicateWrapperName(_)
                    | E::WrapperShadowsClaude(_)
                    | E::WrapperFileNotManaged(_)
                    | E::InvalidWrapperName(_, _)
                    | E::MissingField(_)
                    | E::NoHomeDir
                    | E::UnsupportedPlatform(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_wrapper_name_errors() -> Vec<crate::routes::WrapperNameError> {
            use crate::routes::WrapperNameError as E;
            let samples = vec![
                E::Empty,
                E::InvalidChars("claude kimi".to_string()),
                E::Reserved("claude".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Empty | E::InvalidChars(_) | E::Reserved(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_base_url_errors() -> Vec<crate::routes::BaseUrlError> {
            use crate::routes::BaseUrlError as E;
            let samples = vec![
                E::Empty,
                E::BadScheme("ftp://example.com".to_string()),
                E::NoHost,
                E::InvalidChars,
                E::Malformed("userinfo (`user:pass@host`) is not allowed".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Empty | E::BadScheme(_) | E::NoHost | E::InvalidChars | E::Malformed(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_save_route_errors() -> Vec<crate::routes::SaveRouteError> {
            use crate::routes::SaveRouteError as E;
            let samples = vec![
                E::NotFound("6f9a1c2e-0000-4000-8000-000000000000".to_string()),
                E::Secrets(crate::routes::RouteError::Io(std::io::Error::other(
                    "keychain write failed",
                ))),
                E::Store(crate::routes::RouteError::DuplicateName("kimi".to_string())),
            ];
            for s in &samples {
                match s {
                    E::NotFound(_) | E::Secrets(_) | E::Store(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_mcp_probe_errors() -> Vec<crate::mcp_probe::McpProbeError> {
            use crate::mcp_probe::McpProbeError as E;
            let samples = vec![
                E::CliNotFound {
                    exe_dir: "/Applications/Claudepot.app/Contents/MacOS".to_string(),
                    tried: "claudepot-cli, claudepot".to_string(),
                },
                E::Spawn {
                    bin: "claudepot-cli".to_string(),
                    source: std::io::Error::other("permission denied"),
                },
            ];
            for s in &samples {
                match s {
                    E::CliNotFound { .. } | E::Spawn { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_mcp_snippet_errors() -> Vec<crate::mcp_snippet::InstallError> {
            use crate::mcp_snippet::InstallError as E;
            use std::path::PathBuf;
            let target = PathBuf::from("/Users/me/proj/.claude/claudepot-mcp-instructions.md");
            let samples = vec![
                E::NoHomeDir,
                E::MissingProjectPath,
                E::ProjectPathNotAbsolute(PathBuf::from("relative/dir")),
                E::ProjectPathNotDir(PathBuf::from("/Users/me/gone")),
                E::CreateParent {
                    path: target.clone(),
                    source: std::io::Error::other("permission denied"),
                },
                E::Write {
                    path: target,
                    source: std::io::Error::other("permission denied"),
                },
            ];
            for s in &samples {
                match s {
                    E::NoHomeDir
                    | E::MissingProjectPath
                    | E::ProjectPathNotAbsolute(_)
                    | E::ProjectPathNotDir(_)
                    | E::CreateParent { .. }
                    | E::Write { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_gh_errors() -> Vec<crate::github_pr::GhError> {
            use crate::github_pr::GhError as E;
            let samples = vec![
                E::MissingCli("gh"),
                E::Subprocess {
                    tool: "gh",
                    code: 1,
                    stderr: "no pull requests found for branch feat/x".to_string(),
                },
                E::BadOutput("gh", json_err()),
                E::Io("gh", std::io::Error::other("No such file or directory")),
                E::Timeout("gh"),
            ];
            for s in &samples {
                match s {
                    E::MissingCli(_)
                    | E::Subprocess { .. }
                    | E::BadOutput(_, _)
                    | E::Io(_, _)
                    | E::Timeout(_) => {}
                }
            }
            samples
        }

        /// `(code, param names)` for every variant this domain owns.
        pub fn samples() -> Vec<(&'static str, BTreeSet<String>)> {
            let mut out = Vec::new();
            out.extend(all_agent_errors().iter().map(entry));
            out.extend(all_agent_events_errors().iter().map(entry));
            out.extend(all_board_errors().iter().map(entry));
            out.extend(all_vault_errors().iter().map(entry));
            out.extend(all_env_file_errors().iter().map(entry));
            out.extend(all_route_errors().iter().map(entry));
            out.extend(all_wrapper_name_errors().iter().map(entry));
            out.extend(all_base_url_errors().iter().map(entry));
            out.extend(all_save_route_errors().iter().map(entry));
            out.extend(all_mcp_probe_errors().iter().map(entry));
            out.extend(all_mcp_snippet_errors().iter().map(entry));
            out.extend(all_gh_errors().iter().map(entry));
            out
        }

        /// Rendered `params()` for every sample, for the token scan.
        pub fn rendered_params() -> Vec<(&'static str, String)> {
            fn render<E: ErrorCode>(e: &E) -> (&'static str, String) {
                (e.code(), e.params().to_string())
            }
            let mut out: Vec<(&'static str, String)> = Vec::new();
            out.extend(all_agent_errors().iter().map(render));
            out.extend(all_agent_events_errors().iter().map(render));
            out.extend(all_board_errors().iter().map(render));
            out.extend(all_vault_errors().iter().map(render));
            out.extend(all_env_file_errors().iter().map(render));
            out.extend(all_route_errors().iter().map(render));
            out.extend(all_wrapper_name_errors().iter().map(render));
            out.extend(all_base_url_errors().iter().map(render));
            out.extend(all_save_route_errors().iter().map(render));
            out.extend(all_mcp_probe_errors().iter().map(render));
            out.extend(all_mcp_snippet_errors().iter().map(render));
            out.extend(all_gh_errors().iter().map(render));
            out
        }
    }

    /// Samplers for the accounts / auth / desktop domain
    /// (`testdata/error-codes/10-account.json`). Same one-module-per-
    /// domain shape as `agent_domain`, for the same reason: the
    /// domain's slice of `core_samples` stays one line.
    mod account_domain {
        use super::*;

        /// See `all_key_errors` for why the trailing match exists.
        fn all_swap_errors() -> Vec<crate::cli_backend::SwapError> {
            use crate::cli_backend::SwapError as E;
            let samples = vec![
                E::NoStoredCredentials(uuid::Uuid::nil()),
                E::NoDefaultCredentials,
                E::WriteFailed("keychain item did not read back".to_string()),
                E::KeychainError("keychain is locked".to_string()),
                E::FileError(std::io::Error::other("no such file or directory")),
                E::CorruptBlob("missing field accessToken".to_string()),
                E::RefreshFailed("token endpoint returned 401".to_string()),
                E::IdentityMismatch {
                    stored_email: "a@example.com".to_string(),
                    actual_email: "b@example.com".to_string(),
                },
                E::LiveSessionConflict,
                E::IdentityVerificationFailed("/profile returned 503".to_string()),
            ];
            for s in &samples {
                match s {
                    E::NoStoredCredentials(_)
                    | E::NoDefaultCredentials
                    | E::WriteFailed(_)
                    | E::KeychainError(_)
                    | E::FileError(_)
                    | E::CorruptBlob(_)
                    | E::RefreshFailed(_)
                    | E::IdentityMismatch { .. }
                    | E::LiveSessionConflict
                    | E::IdentityVerificationFailed(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_desktop_swap_errors() -> Vec<crate::desktop_backend::DesktopSwapError> {
            use crate::desktop_backend::DesktopSwapError as E;
            let samples = vec![
                E::DesktopStillRunning,
                E::NoStoredProfile(uuid::Uuid::nil()),
                E::FileCopyFailed("destination is read-only".to_string()),
                E::NotInstalled,
                E::DpapiInvalidated,
                E::Lock(crate::desktop_lock::DesktopLockError::Held),
                E::Io(std::io::Error::other("no such file or directory")),
            ];
            for s in &samples {
                match s {
                    E::DesktopStillRunning
                    | E::NoStoredProfile(_)
                    | E::FileCopyFailed(_)
                    | E::NotInstalled
                    | E::DpapiInvalidated
                    | E::Lock(_)
                    | E::Io(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_desktop_key_errors() -> Vec<crate::desktop_backend::DesktopKeyError> {
            use crate::desktop_backend::DesktopKeyError as E;
            let samples = vec![
                E::KeychainRead("user canceled the operation".to_string()),
                E::DpapiFailed("key not valid for use in specified state".to_string()),
                E::LocalState("no such file or directory".to_string()),
                E::Unsupported,
            ];
            for s in &samples {
                match s {
                    E::KeychainRead(_) | E::DpapiFailed(_) | E::LocalState(_) | E::Unsupported => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_decrypt_errors() -> Vec<crate::desktop_backend::crypto::DecryptError> {
            use crate::desktop_backend::crypto::DecryptError as E;
            let samples = vec![
                E::BadFormat("ciphertext shorter than 3 bytes".to_string()),
                E::UnknownVersion([118, 57, 57]),
                E::Base64("Invalid symbol 33, offset 2".to_string()),
                E::Aes,
                E::Kdf("invalid iteration count".to_string()),
            ];
            for s in &samples {
                match s {
                    E::BadFormat(_) | E::UnknownVersion(_) | E::Base64(_) | E::Aes | E::Kdf(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_token_parse_errors() -> Vec<crate::desktop_backend::token_cache::TokenParseError> {
            use crate::desktop_backend::token_cache::TokenParseError as E;
            let samples = vec![
                E::Json("expected value at line 1 column 2".to_string()),
                E::Empty,
            ];
            for s in &samples {
                match s {
                    E::Json(_) | E::Empty => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_desktop_identity_errors() -> Vec<crate::desktop_identity::DesktopIdentityError> {
            use crate::desktop_identity::DesktopIdentityError as E;
            let samples = vec![
                E::DataDirUnreadable("permission denied".to_string()),
                E::NoDataDir,
                E::NoConfig,
                E::ConfigParse("expected value at line 1 column 2".to_string()),
                E::NotSignedIn,
                E::Key(crate::desktop_backend::DesktopKeyError::Unsupported),
                E::Decrypt(crate::desktop_backend::crypto::DecryptError::Aes),
                E::TokenParse("no access token found in decrypted bundles".to_string()),
                E::ProfileFetch("OAuth server error: /profile returned 503".to_string()),
                E::Unsupported,
            ];
            for s in &samples {
                match s {
                    E::DataDirUnreadable(_)
                    | E::NoDataDir
                    | E::NoConfig
                    | E::ConfigParse(_)
                    | E::NotSignedIn
                    | E::Key(_)
                    | E::Decrypt(_)
                    | E::TokenParse(_)
                    | E::ProfileFetch(_)
                    | E::Unsupported => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_desktop_lock_errors() -> Vec<crate::desktop_lock::DesktopLockError> {
            use crate::desktop_lock::DesktopLockError as E;
            let samples = vec![
                E::Open("permission denied".to_string()),
                E::Held,
                E::Timeout(std::time::Duration::from_secs(5)),
            ];
            for s in &samples {
                match s {
                    E::Open(_) | E::Held | E::Timeout(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_resolve_errors() -> Vec<crate::resolve::ResolveError> {
            use crate::resolve::ResolveError as E;
            let samples = vec![
                E::NoMatch("nobody".to_string()),
                E::Ambiguous {
                    input: "x".to_string(),
                    candidates: vec!["xa@example.com".to_string(), "xb@example.com".to_string()],
                },
                E::StoreError("no such table: accounts".to_string()),
            ];
            for s in &samples {
                match s {
                    E::NoMatch(_) | E::Ambiguous { .. } | E::StoreError(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_onboard_errors() -> Vec<crate::onboard::OnboardError> {
            use crate::onboard::OnboardError as E;
            let samples = vec![
                E::CliBinaryNotFound("/usr/local/bin/claude".to_string()),
                E::AuthLoginFailed(1, None),
                E::AuthLoginCancelled,
                E::ImportFailed("/tmp/claudepot-onboard-xyz".to_string()),
                E::Swap(crate::cli_backend::SwapError::NoDefaultCredentials),
                E::Io(std::io::Error::other("no such file or directory")),
            ];
            for s in &samples {
                match s {
                    E::CliBinaryNotFound(_)
                    | E::AuthLoginFailed(_, _)
                    | E::AuthLoginCancelled
                    | E::ImportFailed(_)
                    | E::Swap(_)
                    | E::Io(_) => {}
                }
            }
            samples
        }

        /// `(code, param names)` for every variant this domain owns.
        pub fn samples() -> Vec<(&'static str, BTreeSet<String>)> {
            let mut out = Vec::new();
            out.extend(all_swap_errors().iter().map(entry));
            out.extend(all_desktop_swap_errors().iter().map(entry));
            out.extend(all_desktop_key_errors().iter().map(entry));
            out.extend(all_decrypt_errors().iter().map(entry));
            out.extend(all_token_parse_errors().iter().map(entry));
            out.extend(all_desktop_identity_errors().iter().map(entry));
            out.extend(all_desktop_lock_errors().iter().map(entry));
            out.extend(all_resolve_errors().iter().map(entry));
            out.extend(all_onboard_errors().iter().map(entry));
            out
        }

        /// Rendered `params()` for every sample, for the token scan.
        pub fn rendered_params() -> Vec<(&'static str, String)> {
            fn render<E: ErrorCode>(e: &E) -> (&'static str, String) {
                (e.code(), e.params().to_string())
            }
            let mut out: Vec<(&'static str, String)> = Vec::new();
            out.extend(all_swap_errors().iter().map(render));
            out.extend(all_desktop_swap_errors().iter().map(render));
            out.extend(all_desktop_key_errors().iter().map(render));
            out.extend(all_decrypt_errors().iter().map(render));
            out.extend(all_token_parse_errors().iter().map(render));
            out.extend(all_desktop_identity_errors().iter().map(render));
            out.extend(all_desktop_lock_errors().iter().map(render));
            out.extend(all_resolve_errors().iter().map(render));
            out.extend(all_onboard_errors().iter().map(render));
            out
        }
    }

    #[test]
    fn account_domain_params_never_carry_a_token() {
        // This domain sits closest to credential material: the swap
        // slot, the decrypted Desktop token cache, and the `claude auth
        // login` stderr tail all pass through these enums.
        for (code, params_json) in account_domain::rendered_params() {
            assert!(
                secret_hits(&params_json).is_empty(),
                "{code}: params must never carry a token (matched: {:?})",
                secret_hits(&params_json),
            );
        }
    }

    /// Samplers for the projects / sessions / memory / corpus domain
    /// (`testdata/error-codes/20-project.json`). Same one-module-per-
    /// domain shape as `agent_domain`, for the same reason: the
    /// domain's slice of `core_samples` stays one line.
    mod project_domain {
        use super::*;
        use std::path::PathBuf;

        /// A `serde_json::Error` — the crate has no public constructor,
        /// so parse something that cannot succeed. Deliberately a local
        /// copy rather than a reach into a sibling domain's module:
        /// these modules are written by separate conversion passes and
        /// must not depend on each other's internals.
        fn json_err() -> serde_json::Error {
            serde_json::from_str::<u32>("nope").expect_err("not a u32")
        }

        fn sql_err() -> rusqlite::Error {
            rusqlite::Error::QueryReturnedNoRows
        }

        fn io_err() -> std::io::Error {
            std::io::Error::other("permission denied")
        }

        /// A transcript path. The samples use the Unix shape; the
        /// Windows shapes are exercised by `params_carry_the_path_raw`
        /// below, which is where the no-normalization rule is locked.
        fn transcript() -> PathBuf {
            PathBuf::from("/Users/me/.claude/projects/-Users-me-proj/6f9a.jsonl")
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_project_trash_errors() -> Vec<crate::project::trash::ProjectTrashError> {
            use crate::project::trash::ProjectTrashError as E;
            let manifest = PathBuf::from("/Users/me/.claudepot/trash/projects/b1/manifest.json");
            let samples = vec![
                E::Io {
                    path: PathBuf::from("/Users/me/.claudepot/trash/projects"),
                    source: io_err(),
                },
                E::SourceMissing(PathBuf::from("/Users/me/.claude/projects/-Users-me-proj")),
                E::EntryNotFound("20260426T153045Z-6f9a1c2e".to_string()),
                E::ManifestParse {
                    path: manifest.clone(),
                    source: json_err(),
                },
                E::Serialize {
                    path: manifest,
                    source: json_err(),
                },
                E::RestoreCollision(PathBuf::from("/Users/me/.claude/projects/-Users-me-proj")),
                E::InvalidSlug("../escape".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Io { .. }
                    | E::SourceMissing(_)
                    | E::EntryNotFound(_)
                    | E::ManifestParse { .. }
                    | E::Serialize { .. }
                    | E::RestoreCollision(_)
                    | E::InvalidSlug(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_trash_errors() -> Vec<crate::trash::TrashError> {
            use crate::trash::TrashError as E;
            let samples = vec![
                E::Io {
                    path: PathBuf::from("/Users/me/.claudepot/trash"),
                    source: io_err(),
                },
                E::SourceMissing(transcript()),
                E::EntryNotFound("20260426T153045Z-6f9a1c2e".to_string()),
                E::ManifestParse {
                    path: PathBuf::from("/Users/me/.claudepot/trash/manifest.json"),
                    source: json_err(),
                },
                E::ManifestSerialize(json_err()),
                E::RestoreCollision(transcript()),
            ];
            for s in &samples {
                match s {
                    E::Io { .. }
                    | E::SourceMissing(_)
                    | E::EntryNotFound(_)
                    | E::ManifestParse { .. }
                    | E::ManifestSerialize(_)
                    | E::RestoreCollision(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_activity_index_errors() -> Vec<crate::activity::ActivityIndexError> {
            use crate::activity::ActivityIndexError as E;
            let samples = vec![E::Sql(sql_err()), E::Io(io_err()), E::Serde(json_err())];
            for s in &samples {
                match s {
                    E::Sql(_) | E::Io(_) | E::Serde(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_corpus_errors() -> Vec<crate::corpus::CorpusError> {
            use crate::corpus::CorpusError as E;
            let samples = vec![E::Db(sql_err()), E::Io(io_err())];
            for s in &samples {
                match s {
                    E::Db(_) | E::Io(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_memory_log_errors() -> Vec<crate::memory_log::MemoryLogError> {
            use crate::memory_log::MemoryLogError as E;
            let samples = vec![E::Io(io_err()), E::Sql(sql_err())];
            for s in &samples {
                match s {
                    E::Io(_) | E::Sql(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_read_memory_errors() -> Vec<crate::memory_view::ReadMemoryError> {
            use crate::memory_view::ReadMemoryError as E;
            let samples = vec![
                E::PathOutsideScope(PathBuf::from("/etc/passwd")),
                E::Io(io_err()),
            ];
            for s in &samples {
                match s {
                    E::PathOutsideScope(_) | E::Io(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        ///
        /// `Project` is sampled too even though it mints no code of its
        /// own — it delegates to the wrapped `ProjectError`, and the
        /// sample is what proves the delegation still resolves.
        fn all_migrate_errors() -> Vec<crate::migrate::MigrateError> {
            use crate::migrate::MigrateError as E;
            let samples = vec![
                E::Io(std::io::Error::other("no such file or directory")),
                E::IntegrityViolation("manifest digest does not match payload".to_string()),
                E::Serialize("expected value at line 1 column 1".to_string()),
                E::UnsupportedSchemaVersion {
                    found: 3,
                    expected: 1,
                },
                E::Configuration("--to requires --from".to_string()),
                E::ProjectNotInBundle("no on-disk slug for cwd /Users/me/proj".to_string()),
                E::Conflict("target already holds a different session with this id".to_string()),
                E::TrustGate {
                    gate: "signature".to_string(),
                    reason: "bundle is unsigned".to_string(),
                },
                E::LiveSession("/Users/me/proj".to_string()),
                E::Project(ProjectError::NotFound("/Users/me/proj".to_string())),
                E::NotImplemented("worktree remap".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Io(_)
                    | E::IntegrityViolation(_)
                    | E::Serialize(_)
                    | E::UnsupportedSchemaVersion { .. }
                    | E::Configuration(_)
                    | E::ProjectNotInBundle(_)
                    | E::Conflict(_)
                    | E::TrustGate { .. }
                    | E::LiveSession(_)
                    | E::Project(_)
                    | E::NotImplemented(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_codex_errors() -> Vec<crate::codex_session::error::CodexError> {
            use crate::codex_session::error::CodexError as E;
            let rollout = PathBuf::from("/Users/me/.codex/sessions/rollout.jsonl");
            let samples = vec![
                E::Io {
                    path: rollout.clone(),
                    source: io_err(),
                },
                E::MissingSessionMeta {
                    path: rollout.clone(),
                },
                E::MissingSessionId { path: rollout },
            ];
            for s in &samples {
                match s {
                    E::Io { .. } | E::MissingSessionMeta { .. } | E::MissingSessionId { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_session_errors() -> Vec<crate::session::SessionError> {
            use crate::session::SessionError as E;
            let samples = vec![
                E::Io(std::io::Error::other("no such file or directory")),
                E::NotFound("6f9a1c2e-0000-4000-8000-000000000000".to_string()),
                E::InvalidPath("/Users/me/elsewhere/notes.jsonl".to_string()),
                E::Index("sqlite: Query returned no rows".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Io(_) | E::NotFound(_) | E::InvalidPath(_) | E::Index(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_session_index_errors() -> Vec<crate::session_index::SessionIndexError> {
            use crate::session_index::SessionIndexError as E;
            let samples = vec![
                E::Sql(sql_err()),
                E::Io(io_err()),
                E::Session(crate::session::SessionError::NotFound(
                    "6f9a1c2e-0000-4000-8000-000000000000".to_string(),
                )),
                E::Json(json_err()),
                E::MigrationValidationFailed {
                    target_version: "4".to_string(),
                    expected: 12,
                    found: 10,
                    missing: vec!["exchanges".to_string(), "tool_calls".to_string()],
                },
            ];
            for s in &samples {
                match s {
                    E::Sql(_)
                    | E::Io(_)
                    | E::Session(_)
                    | E::Json(_)
                    | E::MigrationValidationFailed { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_prune_errors() -> Vec<crate::session::prune::PruneError> {
            use crate::session::prune::PruneError as E;
            let samples = vec![
                E::EmptyFilter,
                E::List(crate::session::SessionError::NotFound(
                    "6f9a1c2e-0000-4000-8000-000000000000".to_string(),
                )),
                E::Trash(crate::trash::TrashError::SourceMissing(transcript())),
            ];
            for s in &samples {
                match s {
                    E::EmptyFilter | E::List(_) | E::Trash(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_redact_errors() -> Vec<crate::session::redact::RedactError> {
            use crate::session::redact::RedactError as E;
            let samples = vec![
                E::Io {
                    path: transcript(),
                    source: io_err(),
                },
                E::NotFound(transcript()),
                E::LiveWriteDetected,
                E::Trash(crate::trash::TrashError::SourceMissing(transcript())),
                E::Json {
                    line: 42,
                    source: json_err(),
                },
                E::NoPatterns,
            ];
            for s in &samples {
                match s {
                    E::Io { .. }
                    | E::NotFound(_)
                    | E::LiveWriteDetected
                    | E::Trash(_)
                    | E::Json { .. }
                    | E::NoPatterns => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_slim_errors() -> Vec<crate::session::slim::SlimError> {
            use crate::session::slim::SlimError as E;
            let samples = vec![
                E::Io {
                    path: transcript(),
                    source: io_err(),
                },
                E::NotFound(transcript()),
                E::LiveWriteDetected,
                E::Trash(crate::trash::TrashError::SourceMissing(transcript())),
                E::Json {
                    line: 42,
                    source: json_err(),
                },
                E::EmptyFilter,
                E::Session(crate::session::SessionError::NotFound(
                    "6f9a1c2e-0000-4000-8000-000000000000".to_string(),
                )),
            ];
            for s in &samples {
                match s {
                    E::Io { .. }
                    | E::NotFound(_)
                    | E::LiveWriteDetected
                    | E::Trash(_)
                    | E::Json { .. }
                    | E::EmptyFilter
                    | E::Session(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_share_errors() -> Vec<crate::session::share::ShareError> {
            use crate::session::share::ShareError as E;
            let samples = vec![
                E::Auth,
                E::Oversized("Validation Failed".to_string()),
                E::Http {
                    status: 503,
                    body: "service unavailable".to_string(),
                },
                E::Network("connection refused".to_string()),
                E::Parse("expected value at line 1 column 1".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Auth | E::Oversized(_) | E::Http { .. } | E::Network(_) | E::Parse(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_deliver_errors() -> Vec<crate::session::export_delivery::DeliverError> {
            use crate::session::export_delivery::DeliverError as E;
            let samples = vec![
                E::File(io_err()),
                E::Clipboard("no display server".to_string()),
                E::Gist(crate::session::share::ShareError::Network(
                    "connection refused".to_string(),
                )),
                E::NoToken,
                E::Keychain("keychain is locked".to_string()),
                E::Harden("permission denied".to_string()),
            ];
            for s in &samples {
                match s {
                    E::File(_)
                    | E::Clipboard(_)
                    | E::Gist(_)
                    | E::NoToken
                    | E::Keychain(_)
                    | E::Harden(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_move_session_errors() -> Vec<crate::session::move_::MoveSessionError> {
            use crate::session::move_::MoveSessionError as E;
            let sid = uuid::Uuid::nil();
            let slug_dir = PathBuf::from("/Users/me/.claude/projects/-Users-me-proj");
            let samples = vec![
                E::SessionNotFound(sid, slug_dir.clone()),
                E::InvalidSlug(String::new(), "empty after sanitization"),
                E::SyncConflictPresent(sid),
                E::LiveSession(sid),
                E::TargetCollision(sid),
                E::SidecarCollision(PathBuf::from(
                    "/Users/me/.claude/projects/-Users-me-new/6f9a.summary.json",
                )),
                E::SameCwd,
                E::WorktreeSiblingStillLive(PathBuf::from("/Users/me/proj")),
                E::InvalidConfigDir(PathBuf::from("/Users/me/.claude")),
                E::TrashFailed("permission denied".to_string()),
                E::Io(std::io::Error::other("no such file or directory")),
            ];
            for s in &samples {
                match s {
                    E::SessionNotFound(_, _)
                    | E::InvalidSlug(_, _)
                    | E::SyncConflictPresent(_)
                    | E::LiveSession(_)
                    | E::TargetCollision(_)
                    | E::SidecarCollision(_)
                    | E::SameCwd
                    | E::WorktreeSiblingStillLive(_)
                    | E::InvalidConfigDir(_)
                    | E::TrashFailed(_)
                    | E::Io(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_bus_errors() -> Vec<crate::session_live::bus::BusError> {
            use crate::session_live::bus::BusError as E;
            let samples = vec![E::SubscriberGone, E::AlreadySubscribed];
            for s in &samples {
                match s {
                    E::SubscriberGone | E::AlreadySubscribed => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_metrics_errors() -> Vec<crate::session_live::metrics_store::MetricsError> {
            use crate::session_live::metrics_store::MetricsError as E;
            let samples = vec![E::Sqlite(sql_err()), E::Io(io_err())];
            for s in &samples {
                match s {
                    E::Sqlite(_) | E::Io(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_live_activity_errors(
        ) -> Vec<crate::services::live_activity_service::LiveActivityError> {
            use crate::services::live_activity_service::LiveActivityError as E;
            let samples = vec![
                E::AlreadySubscribed("6f9a1c2e-0000-4000-8000-000000000000".to_string()),
                E::NotStarted,
            ];
            for s in &samples {
                match s {
                    E::AlreadySubscribed(_) | E::NotStarted => {}
                }
            }
            samples
        }

        /// `(code, param names)` for every variant this domain owns.
        pub fn samples() -> Vec<(&'static str, BTreeSet<String>)> {
            let mut out = Vec::new();
            out.extend(all_live_activity_errors().iter().map(entry));
            out.extend(all_project_trash_errors().iter().map(entry));
            out.extend(all_trash_errors().iter().map(entry));
            out.extend(all_activity_index_errors().iter().map(entry));
            out.extend(all_corpus_errors().iter().map(entry));
            out.extend(all_memory_log_errors().iter().map(entry));
            out.extend(all_read_memory_errors().iter().map(entry));
            out.extend(all_migrate_errors().iter().map(entry));
            out.extend(all_codex_errors().iter().map(entry));
            out.extend(all_session_errors().iter().map(entry));
            out.extend(all_session_index_errors().iter().map(entry));
            out.extend(all_prune_errors().iter().map(entry));
            out.extend(all_redact_errors().iter().map(entry));
            out.extend(all_slim_errors().iter().map(entry));
            out.extend(all_share_errors().iter().map(entry));
            out.extend(all_deliver_errors().iter().map(entry));
            out.extend(all_move_session_errors().iter().map(entry));
            out.extend(all_bus_errors().iter().map(entry));
            out.extend(all_metrics_errors().iter().map(entry));
            out
        }

        /// Rendered `params()` for every sample, for the token scan.
        pub fn rendered_params() -> Vec<(&'static str, String)> {
            fn render<E: ErrorCode>(e: &E) -> (&'static str, String) {
                (e.code(), e.params().to_string())
            }
            let mut out: Vec<(&'static str, String)> = Vec::new();
            out.extend(all_live_activity_errors().iter().map(render));
            out.extend(all_project_trash_errors().iter().map(render));
            out.extend(all_trash_errors().iter().map(render));
            out.extend(all_activity_index_errors().iter().map(render));
            out.extend(all_corpus_errors().iter().map(render));
            out.extend(all_memory_log_errors().iter().map(render));
            out.extend(all_read_memory_errors().iter().map(render));
            out.extend(all_migrate_errors().iter().map(render));
            out.extend(all_codex_errors().iter().map(render));
            out.extend(all_session_errors().iter().map(render));
            out.extend(all_session_index_errors().iter().map(render));
            out.extend(all_prune_errors().iter().map(render));
            out.extend(all_redact_errors().iter().map(render));
            out.extend(all_slim_errors().iter().map(render));
            out.extend(all_share_errors().iter().map(render));
            out.extend(all_deliver_errors().iter().map(render));
            out.extend(all_move_session_errors().iter().map(render));
            out.extend(all_bus_errors().iter().map(render));
            out.extend(all_metrics_errors().iter().map(render));
            out
        }
    }

    #[test]
    fn project_domain_params_never_carry_a_token() {
        // Transcripts are the leakiest substrate in the product: a
        // `.jsonl` holds whatever the user pasted into a session, and
        // redact / slim / share all read one.
        for (code, params_json) in project_domain::rendered_params() {
            assert!(
                secret_hits(&params_json).is_empty(),
                "{code}: params must never carry a token (matched: {:?})",
                secret_hits(&params_json),
            );
        }
    }

    #[test]
    fn params_carry_the_path_raw() {
        // `rules/paths.md`: a path reaching `params` is never
        // normalized or re-separatored. All four shapes survive
        // byte-for-byte, so a localized sentence places the same string
        // the user's filesystem uses.
        use crate::session::redact::RedactError;
        for shape in [
            r"/Users/me/.claude/projects/-Users-me-proj/6f9a.jsonl",
            r"C:\Users\me\.claude\projects\C--Users-me-proj\6f9a.jsonl",
            r"\\server\share\.claude\projects\6f9a.jsonl",
            r"\\?\C:\Users\me\.claude\projects\6f9a.jsonl",
        ] {
            let e = RedactError::NotFound(std::path::PathBuf::from(shape));
            assert_eq!(e.params()["path"], shape, "{shape} was rewritten");
        }
    }

    #[test]
    fn migrate_project_delegates_instead_of_minting_a_code() {
        // The variant is `#[error("{0}")]` — its English text *is* the
        // inner error's, so a `migrate.project` code carrying `detail`
        // would freeze an English clause inside a localized sentence.
        let inner = ProjectError::ClaudeRunning("/Users/me/proj".to_string());
        let english = inner.to_string();
        let e = crate::migrate::MigrateError::Project(inner);
        assert_eq!(e.code(), "project.claude_running");
        assert_eq!(e.params()["path"], "/Users/me/proj");
        assert_eq!(e.to_string(), english);
    }

    /// Config, settings, retention, permissions, rotation, pricing.
    mod config_domain {
        use super::*;

        /// A `serde_json::Error` — the crate has no public constructor,
        /// so parse something that cannot succeed.
        fn json_err() -> serde_json::Error {
            serde_json::from_str::<u32>("nope").expect_err("not a u32")
        }

        fn io_err() -> std::io::Error {
            std::io::Error::other("permission denied")
        }

        fn path() -> std::path::PathBuf {
            std::path::PathBuf::from("/Users/me/.claude/settings.json")
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_cc_env_errors() -> Vec<crate::cc_env::CcEnvError> {
            use crate::cc_env::spec::Blocked;
            use crate::cc_env::CcEnvError as E;
            let samples = vec![
                E::Io(io_err()),
                E::JsonParse(json_err()),
                E::NotAJsonObject(path()),
                E::EnvNotAnObject {
                    path: path(),
                    found: "array",
                },
                E::UnknownVariable("NOT_A_REAL_VAR".to_string()),
                E::NotEditable {
                    name: "CLAUDE_CONFIG_DIR".to_string(),
                    reason: Blocked::BootstrapSplitBrain,
                },
                E::InvalidEnumValue {
                    name: "DISABLE_TELEMETRY".to_string(),
                    value: "yes".to_string(),
                    allowed: vec!["0".to_string(), "1".to_string()],
                },
                E::InvalidNumber {
                    name: "MAX_THINKING_TOKENS".to_string(),
                    value: "12x".to_string(),
                },
                // The leak case the second gate exists for: a token
                // reaching `params` from a constructor that skipped
                // `redacted`. `params_never_carry_a_token` below is what
                // this sample is here to feed.
                E::InvalidNumber {
                    name: "MAX_THINKING_TOKENS".to_string(),
                    value: "sk-ant-oat01-pasted-here".to_string(),
                },
                E::NotSet("MAX_THINKING_TOKENS".to_string()),
                E::Contended { path: path() },
                E::MalformedSpec("duplicate variable name".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Io(_)
                    | E::JsonParse(_)
                    | E::NotAJsonObject(_)
                    | E::EnvNotAnObject { .. }
                    | E::UnknownVariable(_)
                    | E::NotEditable { .. }
                    | E::InvalidEnumValue { .. }
                    | E::InvalidNumber { .. }
                    | E::NotSet(_)
                    | E::Contended { .. }
                    | E::MalformedSpec(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_retention_errors() -> Vec<crate::cc_retention::RetentionError> {
            use crate::cc_retention::RetentionError as E;
            let samples = vec![
                E::Write(crate::settings_writer::SettingsWriteError::Io(io_err())),
                // `0` — the most destructive value on this key, and the
                // reason this variant may never be folded into a generic
                // out-of-range code.
                E::NonPositive(0),
            ];
            for s in &samples {
                match s {
                    E::Write(_) | E::NonPositive(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_tips_errors() -> Vec<crate::cc_tips::TipsError> {
            use crate::cc_tips::TipsError as E;
            let samples = vec![
                E::BinaryNotFound {
                    path: "/usr/local/bin/claude".to_string(),
                },
                E::BinaryRead {
                    path: "/usr/local/bin/claude".to_string(),
                    source: io_err(),
                },
                E::ConfigRead {
                    path: "/Users/me/.claude.json".to_string(),
                    source: io_err(),
                },
                E::ConfigParse {
                    path: "/Users/me/.claude.json".to_string(),
                    source: json_err(),
                },
                E::SnapshotRead {
                    path: "/Users/me/.claudepot/tips.json".to_string(),
                    source: io_err(),
                },
                E::SnapshotWrite {
                    path: "/Users/me/.claudepot/tips.json".to_string(),
                    source: io_err(),
                },
                E::CatalogIo {
                    path: "/Users/me/.claudepot/tips-cache.json".to_string(),
                    source: io_err(),
                },
                E::CatalogParse { source: json_err() },
                E::NoHome,
            ];
            for s in &samples {
                match s {
                    E::BinaryNotFound { .. }
                    | E::BinaryRead { .. }
                    | E::ConfigRead { .. }
                    | E::ConfigParse { .. }
                    | E::SnapshotRead { .. }
                    | E::SnapshotWrite { .. }
                    | E::CatalogIo { .. }
                    | E::CatalogParse { .. }
                    | E::NoHome => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_config_view_errors() -> Vec<crate::config_view::ConfigViewError> {
            use crate::config_view::ConfigViewError as E;
            let samples = vec![
                E::Io {
                    path: path(),
                    source: io_err(),
                },
                E::Parse {
                    path: path(),
                    source: json_err(),
                },
                // The pattern is assembled at runtime on purpose:
                // clippy's `invalid_regex` lint reads a literal here
                // and denies the build, and the whole point of this
                // sample is a pattern that does not compile.
                E::BadQuery(regex::Regex::new(&String::from("[")).expect_err("not a regex")),
                E::ManifestNotFound(std::path::PathBuf::from("/Users/me/.claude/plugins/demo")),
            ];
            for s in &samples {
                match s {
                    E::Io { .. } | E::Parse { .. } | E::BadQuery(_) | E::ManifestNotFound(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_launch_errors() -> Vec<crate::config_view::launcher::LaunchError> {
            use crate::config_view::launcher::LaunchError as E;
            let samples = vec![
                E::Spawn("no such file or directory".to_string()),
                E::NoEnvEditor,
                E::NoBinary,
                E::EmptyPath,
                E::UnknownEditor("nano9000".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Spawn(_)
                    | E::NoEnvEditor
                    | E::NoBinary
                    | E::EmptyPath
                    | E::UnknownEditor(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_launcher_errors() -> Vec<crate::launcher::LauncherError> {
            use crate::launcher::LauncherError as E;
            let samples = vec![
                E::NoStoredCredentials(uuid::Uuid::nil()),
                E::CorruptBlob("missing field `accessToken`".to_string()),
                E::CredentialRead("keychain is locked".to_string()),
                E::RefreshFailed("token endpoint rejected refresh_token with 401".to_string()),
                E::SaveFailed("keychain is locked".to_string()),
                E::NoCommand,
                E::SpawnFailed("no such file or directory".to_string()),
            ];
            for s in &samples {
                match s {
                    E::NoStoredCredentials(_)
                    | E::CorruptBlob(_)
                    | E::CredentialRead(_)
                    | E::RefreshFailed(_)
                    | E::SaveFailed(_)
                    | E::NoCommand
                    | E::SpawnFailed(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_protected_paths_errors() -> Vec<crate::protected_paths::ProtectedPathsError> {
            use crate::protected_paths::ProtectedPathsError as E;
            let samples = vec![
                E::Io(io_err()),
                E::InvalidJson {
                    path: std::path::PathBuf::from("/Users/me/.claudepot/protected-paths.json"),
                    err: json_err(),
                },
                E::Empty,
                E::NotAbsolute("relative/dir".to_string()),
                E::Duplicate("/Users/me/proj".to_string()),
                E::NotFound("/Users/me/proj".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Io(_)
                    | E::InvalidJson { .. }
                    | E::Empty
                    | E::NotAbsolute(_)
                    | E::Duplicate(_)
                    | E::NotFound(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_allowlist_errors() -> Vec<crate::available_models::AllowlistError> {
            use crate::available_models::AllowlistError as E;
            let samples = vec![
                E::EmptyEntry(2),
                E::ContainsWhitespace(1, "claude opus 4".to_string()),
            ];
            for s in &samples {
                match s {
                    E::EmptyEntry(_) | E::ContainsWhitespace(..) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        /// `Allowlist` delegates, so its codes are claimed by
        /// `all_allowlist_errors` above; only `Write` is new here.
        fn all_set_allowlist_errors() -> Vec<crate::available_models::SetAllowlistError> {
            use crate::available_models::SetAllowlistError as E;
            let samples = vec![
                E::Allowlist(crate::available_models::AllowlistError::EmptyEntry(2)),
                E::Write(crate::settings_writer::SettingsWriteError::Io(io_err())),
            ];
            for s in &samples {
                match s {
                    E::Allowlist(_) | E::Write(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_grant_validation_errors() -> Vec<crate::permission::grants::ValidationError> {
            use crate::permission::grants::ValidationError as E;
            let samples = vec![
                E::UnsupportedSchemaVersion {
                    found: 9,
                    expected: 1,
                },
                E::EmptyProjectPath,
                E::NonPositiveDuration("/Users/me/proj".to_string()),
                E::DuplicateProject("/Users/me/proj".to_string()),
                E::ProjectLayerNotAllowed("/Users/me/proj".to_string()),
            ];
            for s in &samples {
                match s {
                    E::UnsupportedSchemaVersion { .. }
                    | E::EmptyProjectPath
                    | E::NonPositiveDuration(_)
                    | E::DuplicateProject(_)
                    | E::ProjectLayerNotAllowed(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_permission_settings_errors(
        ) -> Vec<crate::permission::settings::PermissionSettingsError> {
            use crate::permission::settings::PermissionSettingsError as E;
            let samples = vec![
                E::Io(io_err()),
                E::JsonParse(json_err()),
                E::NotAJsonObject(path()),
                E::PermissionsNotAnObject(path()),
                E::UnsupportedLayer {
                    layer: crate::settings_writer::SettingsLayer::Project,
                },
                E::Contended { path: path() },
            ];
            for s in &samples {
                match s {
                    E::Io(_)
                    | E::JsonParse(_)
                    | E::NotAJsonObject(_)
                    | E::PermissionsNotAnObject(_)
                    | E::UnsupportedLayer { .. }
                    | E::Contended { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_permission_store_errors() -> Vec<crate::permission::store::PermissionStoreError> {
            use crate::permission::store::PermissionStoreError as E;
            let samples = vec![
                E::Io(io_err()),
                E::Serde(json_err()),
                E::Validation(crate::permission::grants::ValidationError::EmptyProjectPath),
            ];
            for s in &samples {
                match s {
                    E::Io(_) | E::Serde(_) | E::Validation(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_rotation_validation_errors() -> Vec<crate::rotation::rules::ValidationError> {
            use crate::rotation::rules::ValidationError as E;
            let samples = vec![
                E::EmptyId,
                E::DuplicateId("nightly".to_string()),
                E::BadThreshold(120),
                E::EmptyCandidates,
                E::DuplicateCandidate("me@example.com".to_string()),
                E::EmptyExplicitEmail,
                E::ZeroMaxSwaps,
                E::LeastUsedWindowMismatch {
                    selector: "seven_day",
                    trigger: "five_hour",
                },
                E::UnsupportedSchemaVersion {
                    found: 9,
                    expected: 1,
                },
            ];
            for s in &samples {
                match s {
                    E::EmptyId
                    | E::DuplicateId(_)
                    | E::BadThreshold(_)
                    | E::EmptyCandidates
                    | E::DuplicateCandidate(_)
                    | E::EmptyExplicitEmail
                    | E::ZeroMaxSwaps
                    | E::LeastUsedWindowMismatch { .. }
                    | E::UnsupportedSchemaVersion { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_rotation_store_errors() -> Vec<crate::rotation::store::RotationStoreError> {
            use crate::rotation::store::RotationStoreError as E;
            let samples = vec![
                E::Io(io_err()),
                E::Serde(json_err()),
                E::Validation(crate::rotation::rules::ValidationError::EmptyId),
            ];
            for s in &samples {
                match s {
                    E::Io(_) | E::Serde(_) | E::Validation(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_rotation_breaker_errors() -> Vec<crate::rotation::breaker_store::RotationBreakerError>
        {
            use crate::rotation::breaker_store::RotationBreakerError as E;
            let samples = vec![
                E::Io(io_err()),
                E::Serde(json_err()),
                E::UnsupportedSchemaVersion {
                    found: 9,
                    expected: 1,
                },
            ];
            for s in &samples {
                match s {
                    E::Io(_) | E::Serde(_) | E::UnsupportedSchemaVersion { .. } => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_pricing_validation_errors() -> Vec<crate::pricing::history::ValidationError> {
            use crate::pricing::history::ValidationError as E;
            let samples = vec![
                E::UnsupportedSchemaVersion(9),
                E::BadDate(3),
                E::EmptyModelId(3),
                E::NegativeRate(3),
            ];
            for s in &samples {
                match s {
                    E::UnsupportedSchemaVersion(_)
                    | E::BadDate(_)
                    | E::EmptyModelId(_)
                    | E::NegativeRate(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_pricing_store_errors() -> Vec<crate::pricing::history::HistoryStoreError> {
            use crate::pricing::history::HistoryStoreError as E;
            let samples = vec![
                E::Io(io_err()),
                E::Validation(crate::pricing::history::ValidationError::NegativeRate(3)),
            ];
            for s in &samples {
                match s {
                    E::Io(_) | E::Validation(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        /// `#[non_exhaustive]` does not relax the match inside the
        /// defining crate, which is what keeps this gate real.
        fn all_lifecycle_errors() -> Vec<crate::artifact_lifecycle::LifecycleError> {
            use crate::artifact_lifecycle::LifecycleError as E;
            let samples = vec![
                E::Refused(crate::artifact_lifecycle::RefuseReason::OutOfScope {
                    path: std::path::PathBuf::from("/tmp/elsewhere"),
                }),
                E::SourceMissing(std::path::PathBuf::from("/Users/me/.claude/skills/demo")),
                E::Conflict(std::path::PathBuf::from("/Users/me/.claude/skills/demo")),
                E::ScopeRootMissing(std::path::PathBuf::from("/Users/me/proj")),
                E::WrongTrashState {
                    state: "MissingManifest",
                    action: "restore",
                },
                E::TrashEntryNotFound("6f9a1c2e-0000-4000-8000-000000000000".to_string()),
                E::Io {
                    op: "disable",
                    source: io_err(),
                },
                E::ManifestParse(json_err()),
                E::RecoveryAmbiguous("orphan payload has multiple children".to_string()),
                E::InvalidTrashId("../../etc".to_string()),
            ];
            for s in &samples {
                match s {
                    E::Refused(_)
                    | E::SourceMissing(_)
                    | E::Conflict(_)
                    | E::ScopeRootMissing(_)
                    | E::WrongTrashState { .. }
                    | E::TrashEntryNotFound(_)
                    | E::Io { .. }
                    | E::ManifestParse(_)
                    | E::RecoveryAmbiguous(_)
                    | E::InvalidTrashId(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_refuse_reasons() -> Vec<crate::artifact_lifecycle::RefuseReason> {
            use crate::artifact_lifecycle::RefuseReason as E;
            let demo = || std::path::PathBuf::from("/Users/me/.claude/skills/demo");
            let samples = vec![
                E::Plugin {
                    plugin_id: "cc-suite".to_string(),
                    path: demo(),
                },
                E::ManagedPolicy {
                    root: std::path::PathBuf::from("/Library/Application Support/ClaudeCode"),
                    path: demo(),
                },
                E::OutOfScope {
                    path: std::path::PathBuf::from("/tmp/elsewhere"),
                },
                E::SymlinkLoop { path: demo() },
                E::WrongKind {
                    path: std::path::PathBuf::from("/Users/me/.claude/notes.md"),
                },
            ];
            for s in &samples {
                match s {
                    E::Plugin { .. }
                    | E::ManagedPolicy { .. }
                    | E::OutOfScope { .. }
                    | E::SymlinkLoop { .. }
                    | E::WrongKind { .. } => {}
                }
            }
            samples
        }

        pub fn samples() -> Vec<(&'static str, BTreeSet<String>)> {
            let mut out = Vec::new();
            out.extend(all_cc_env_errors().iter().map(entry));
            out.extend(all_retention_errors().iter().map(entry));
            out.extend(all_tips_errors().iter().map(entry));
            out.extend(all_config_view_errors().iter().map(entry));
            out.extend(all_launch_errors().iter().map(entry));
            out.extend(all_launcher_errors().iter().map(entry));
            out.extend(all_protected_paths_errors().iter().map(entry));
            out.extend(all_allowlist_errors().iter().map(entry));
            out.extend(all_set_allowlist_errors().iter().map(entry));
            out.extend(all_grant_validation_errors().iter().map(entry));
            out.extend(all_permission_settings_errors().iter().map(entry));
            out.extend(all_permission_store_errors().iter().map(entry));
            out.extend(all_rotation_validation_errors().iter().map(entry));
            out.extend(all_rotation_store_errors().iter().map(entry));
            out.extend(all_rotation_breaker_errors().iter().map(entry));
            out.extend(all_pricing_validation_errors().iter().map(entry));
            out.extend(all_pricing_store_errors().iter().map(entry));
            out.extend(all_lifecycle_errors().iter().map(entry));
            out.extend(all_refuse_reasons().iter().map(entry));
            out
        }

        /// Rendered `params()` for every sample, for the token scan.
        pub fn rendered_params() -> Vec<(&'static str, String)> {
            fn render<E: ErrorCode>(e: &E) -> (&'static str, String) {
                (e.code(), e.params().to_string())
            }
            let mut out: Vec<(&'static str, String)> = Vec::new();
            out.extend(all_cc_env_errors().iter().map(render));
            out.extend(all_retention_errors().iter().map(render));
            out.extend(all_tips_errors().iter().map(render));
            out.extend(all_config_view_errors().iter().map(render));
            out.extend(all_launch_errors().iter().map(render));
            out.extend(all_launcher_errors().iter().map(render));
            out.extend(all_protected_paths_errors().iter().map(render));
            out.extend(all_allowlist_errors().iter().map(render));
            out.extend(all_set_allowlist_errors().iter().map(render));
            out.extend(all_grant_validation_errors().iter().map(render));
            out.extend(all_permission_settings_errors().iter().map(render));
            out.extend(all_permission_store_errors().iter().map(render));
            out.extend(all_rotation_validation_errors().iter().map(render));
            out.extend(all_rotation_store_errors().iter().map(render));
            out.extend(all_rotation_breaker_errors().iter().map(render));
            out.extend(all_pricing_validation_errors().iter().map(render));
            out.extend(all_pricing_store_errors().iter().map(render));
            out.extend(all_lifecycle_errors().iter().map(render));
            out.extend(all_refuse_reasons().iter().map(render));
            out
        }
    }

    #[test]
    fn config_domain_params_never_carry_a_token() {
        // `cc_env` is the reason this test exists as its own case: the
        // env pane is the one surface where a user can paste a token
        // into a field that is not credential-bearing, and `params`
        // outlives the toast in the JS heap. The sampler includes that
        // exact shape.
        for (code, params_json) in config_domain::rendered_params() {
            assert!(
                secret_hits(&params_json).is_empty(),
                "{code}: params must never carry a token (matched: {:?})",
                secret_hits(&params_json),
            );
        }
    }

    #[test]
    fn zero_is_not_an_ordinary_out_of_range_retention_value() {
        // AGENTS.md, "Transcript retention": `0` means *write no
        // transcripts and delete the existing ones at startup*. It is
        // not on the duration scale, and `set_retention_days` refuses
        // it so that `disable_persistence()` stays the only way there.
        // Its code must therefore stay distinct from any generic
        // invalid-value code, and `days` must cross so a localized
        // sentence can name what the user typed without re-parsing an
        // English message that points at an internal API.
        let e = crate::cc_retention::RetentionError::NonPositive(0);
        assert_eq!(e.code(), "cc_retention.non_positive");
        assert_eq!(e.params()["days"], 0);
        assert!(e.to_string().contains("disable_persistence()"), "{e}");
    }

    #[test]
    fn a_blocked_env_var_crosses_its_reason_as_a_token_not_a_sentence() {
        // The English message renders `describe_blocked(reason)`, a
        // whole paragraph. A translator needs the discriminant.
        let e = crate::cc_env::CcEnvError::NotEditable {
            name: "CLAUDE_CONFIG_DIR".to_string(),
            reason: crate::cc_env::spec::Blocked::BootstrapSplitBrain,
        };
        assert_eq!(e.code(), "cc_env.not_editable");
        assert_eq!(e.params()["name"], "CLAUDE_CONFIG_DIR");
        assert_eq!(e.params()["reason"], "bootstrap_split_brain");
    }

    /// `(code, param names)` for one sampled variant. Free function so
    /// the per-domain sampler modules can share it.
    fn entry<E: ErrorCode>(e: &E) -> (&'static str, BTreeSet<String>) {
        let params = e.params();
        let obj = params
            .as_object()
            .unwrap_or_else(|| panic!("{}: params() must be a JSON object", e.code()));
        (e.code(), obj.keys().cloned().collect())
    }

    #[test]
    fn agent_domain_params_never_carry_a_token() {
        // The strictest slice: boards hold arbitrary agent output and
        // the env vault holds named secrets, so `params` here is the
        // easiest place in the codebase to leak one.
        for (code, params_json) in agent_domain::rendered_params() {
            assert!(
                secret_hits(&params_json).is_empty(),
                "{code}: params must never carry a token (matched: {:?})",
                secret_hits(&params_json),
            );
        }
    }

    /// Samplers for the `services::*` orchestration enums plus
    /// `settings_writer` (`testdata/error-codes/50-shared-memory.json`).
    ///
    /// These seven were the last core enums still riding generic
    /// `{{detail}}`-only *command* codes — `accounts.register_failed`,
    /// `desktop.switch_failed`, and friends. A command code can say only
    /// "that failed"; these enums distinguish "nothing was spent" from
    /// "a request went out", and "retry" from "sign in again", so each
    /// one earned a real identity.
    mod services_domain {
        use super::*;

        fn io_err() -> std::io::Error {
            std::io::Error::other("permission denied")
        }

        /// A `serde_json::Error` — the crate has no public constructor,
        /// so parse something that cannot succeed. A local copy on
        /// purpose: these domain modules are written by separate
        /// conversion passes and must not depend on each other.
        fn json_err() -> serde_json::Error {
            serde_json::from_str::<u32>("nope").expect_err("not a u32")
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_register_errors() -> Vec<crate::services::account_service::RegisterError> {
            use crate::services::account_service::RegisterError as E;
            let samples = vec![
                E::NoCredentials,
                E::CredentialRead("keychain item did not decode".to_string()),
                E::CredentialWrite("keychain item did not read back".to_string()),
                E::ProfileFetch("/profile returned 503".to_string()),
                E::AlreadyRegistered("a@example.com".to_string(), uuid::Uuid::nil()),
                E::Store("no such table: accounts".to_string()),
                E::NotFound,
                E::AuthRejected,
                E::CcChangedDuringRefresh,
                E::CcLiveRefreshSkipped,
            ];
            for s in &samples {
                match s {
                    E::NoCredentials
                    | E::CredentialRead(_)
                    | E::CredentialWrite(_)
                    | E::ProfileFetch(_)
                    | E::AlreadyRegistered(_, _)
                    | E::Store(_)
                    | E::NotFound
                    | E::AuthRejected
                    | E::CcChangedDuringRefresh
                    | E::CcLiveRefreshSkipped => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_verify_errors() -> Vec<crate::services::identity::VerifyError> {
            use crate::services::identity::VerifyError as E;
            let samples = vec![
                E::AccountNotFound(uuid::Uuid::nil()),
                E::NoBlob,
                E::CorruptBlob("expected value at line 1 column 2".to_string()),
                E::Store("database is locked".to_string()),
            ];
            for s in &samples {
                match s {
                    E::AccountNotFound(_) | E::NoBlob | E::CorruptBlob(_) | E::Store(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_wake_errors() -> Vec<crate::services::wake_service::WakeError> {
            use crate::services::wake_service::WakeError as E;
            let samples = vec![
                E::NotFound,
                E::NoCredentials("a@example.com".to_string()),
                E::IdentityUnverified("a@example.com".to_string(), "drift".to_string()),
                E::Store("database is locked".to_string()),
                E::Token("no stored credentials for this account".to_string()),
                E::Request("/v1/messages returned 429".to_string()),
            ];
            for s in &samples {
                match s {
                    E::NotFound
                    | E::NoCredentials(_)
                    | E::IdentityUnverified(_, _)
                    | E::Store(_)
                    | E::Token(_)
                    | E::Request(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_switch_errors() -> Vec<crate::services::desktop_service::SwitchError> {
            use crate::services::desktop_service::SwitchError as E;
            let samples = vec![
                E::NoSnapshot {
                    email: "a@example.com".to_string(),
                },
                E::NotFound(uuid::Uuid::nil()),
                E::Unsupported,
                // Delegates — `#[error(transparent)]`. The sample proves
                // the delegation still resolves to a registered code.
                E::Swap(crate::desktop_backend::DesktopSwapError::NotInstalled),
                E::Store("no such table: accounts".to_string()),
            ];
            for s in &samples {
                match s {
                    E::NoSnapshot { .. }
                    | E::NotFound(_)
                    | E::Unsupported
                    | E::Swap(_)
                    | E::Store(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_adopt_errors() -> Vec<crate::services::desktop_service::AdoptError> {
            use crate::services::desktop_service::AdoptError as E;
            let samples = vec![
                E::IdentityMismatch {
                    expected: "a@example.com".to_string(),
                    actual: "b@example.com".to_string(),
                },
                E::NotFound(uuid::Uuid::nil()),
                E::ProfileExists,
                E::Unsupported,
                E::DataDirUnreadable,
                E::Swap(crate::desktop_backend::DesktopSwapError::DesktopStillRunning),
                E::Store("no such table: accounts".to_string()),
                E::Lock(crate::desktop_lock::DesktopLockError::Held),
                E::Sidecar("permission denied".to_string()),
            ];
            for s in &samples {
                match s {
                    E::IdentityMismatch { .. }
                    | E::NotFound(_)
                    | E::ProfileExists
                    | E::Unsupported
                    | E::DataDirUnreadable
                    | E::Swap(_)
                    | E::Store(_)
                    | E::Lock(_)
                    | E::Sidecar(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_desktop_clear_errors() -> Vec<crate::services::desktop_service::ClearError> {
            use crate::services::desktop_service::ClearError as E;
            let samples = vec![
                E::Unsupported,
                E::DataDirMissing,
                E::Swap(crate::desktop_backend::DesktopSwapError::DesktopStillRunning),
                E::Fs("permission denied".to_string()),
                E::Store("no such table: accounts".to_string()),
                E::Lock(crate::desktop_lock::DesktopLockError::Held),
            ];
            for s in &samples {
                match s {
                    E::Unsupported
                    | E::DataDirMissing
                    | E::Swap(_)
                    | E::Fs(_)
                    | E::Store(_)
                    | E::Lock(_) => {}
                }
            }
            samples
        }

        /// See `all_key_errors` for why the trailing match exists.
        fn all_settings_write_errors() -> Vec<crate::settings_writer::SettingsWriteError> {
            use crate::settings_writer::SettingsWriteError as E;
            let settings = std::path::PathBuf::from("/Users/me/.claude/settings.json");
            let samples = vec![
                E::Io(io_err()),
                E::JsonParse(json_err()),
                E::NotAJsonObject(settings.clone()),
                E::UnsupportedLayer {
                    layer: crate::settings_writer::SettingsLayer::Project,
                },
                E::Contended { path: settings },
            ];
            for s in &samples {
                match s {
                    E::Io(_)
                    | E::JsonParse(_)
                    | E::NotAJsonObject(_)
                    | E::UnsupportedLayer { .. }
                    | E::Contended { .. } => {}
                }
            }
            samples
        }

        /// `(code, param names)` for every variant this domain owns.
        pub fn samples() -> Vec<(&'static str, BTreeSet<String>)> {
            let mut out = Vec::new();
            out.extend(all_register_errors().iter().map(entry));
            out.extend(all_verify_errors().iter().map(entry));
            out.extend(all_wake_errors().iter().map(entry));
            out.extend(all_switch_errors().iter().map(entry));
            out.extend(all_adopt_errors().iter().map(entry));
            out.extend(all_desktop_clear_errors().iter().map(entry));
            out.extend(all_settings_write_errors().iter().map(entry));
            out
        }

        /// Rendered `params()` for every sample, for the token scan.
        pub fn rendered_params() -> Vec<(&'static str, String)> {
            fn render<E: ErrorCode>(e: &E) -> (&'static str, String) {
                (e.code(), e.params().to_string())
            }
            let mut out: Vec<(&'static str, String)> = Vec::new();
            out.extend(all_register_errors().iter().map(render));
            out.extend(all_verify_errors().iter().map(render));
            out.extend(all_wake_errors().iter().map(render));
            out.extend(all_switch_errors().iter().map(render));
            out.extend(all_adopt_errors().iter().map(render));
            out.extend(all_desktop_clear_errors().iter().map(render));
            out.extend(all_settings_write_errors().iter().map(render));
            out
        }
    }

    #[test]
    fn services_domain_params_never_carry_a_token() {
        // These enums sit directly on the credential paths: register
        // reads CC's keychain blob, verify holds a decoded one, and wake
        // mints an access token to spend quota with.
        for (code, params_json) in services_domain::rendered_params() {
            assert!(
                secret_hits(&params_json).is_empty(),
                "{code}: params must never carry a token (matched: {:?})",
                secret_hits(&params_json),
            );
        }
    }

    #[test]
    fn a_refused_wake_is_distinguishable_from_a_failed_one() {
        // `wake_service`'s own doc draws this line: the eligibility
        // variants mean nothing was billed, the failure variants mean a
        // request may have gone out. One shared code would let a
        // translator answer "did that cost me anything" once, for two
        // opposite cases.
        use crate::services::wake_service::WakeError;
        let refused = WakeError::IdentityUnverified("a@example.com".to_string(), "drift".into());
        let failed = WakeError::Request("/v1/messages returned 429".to_string());
        assert_ne!(refused.code(), failed.code());
        assert_eq!(refused.params()["status"], "drift");
    }

    #[test]
    fn a_transparent_wrapper_delegates_and_a_prefixing_one_does_not() {
        // `SwitchError::Swap` is `#[error(transparent)]` — its English
        // *is* the inner error's, so a wrapper code would freeze that
        // text inside a sentence someone already translated.
        // `ClearError::Swap` prepends `"swap error: "`, which is framing
        // a translator must carry, so it keeps its own identity.
        use crate::services::desktop_service::{ClearError, SwitchError};
        let inner = crate::desktop_backend::DesktopSwapError::DesktopStillRunning;
        let english = inner.to_string();
        let transparent = SwitchError::Swap(inner);
        assert_eq!(transparent.code(), "desktop_backend.desktop_still_running");
        assert_eq!(transparent.to_string(), english);

        let prefixed =
            ClearError::Swap(crate::desktop_backend::DesktopSwapError::DesktopStillRunning);
        assert_eq!(prefixed.code(), "desktop_clear.swap");
        assert!(prefixed.to_string().starts_with("swap error: "));
    }

    /// `(code, param names)` for every sampled variant, plus the codes
    /// that carry no constructible sample.
    fn core_samples() -> Vec<(&'static str, BTreeSet<String>)> {
        fn entry<E: ErrorCode>(e: &E) -> (&'static str, BTreeSet<String>) {
            let params = e.params();
            let obj = params
                .as_object()
                .unwrap_or_else(|| panic!("{}: params() must be a JSON object", e.code()));
            (e.code(), obj.keys().cloned().collect())
        }
        let mut out: Vec<(&'static str, BTreeSet<String>)> = Vec::new();
        out.extend(all_key_errors().iter().map(entry));
        out.extend(all_oauth_errors().iter().map(entry));
        out.extend(all_project_errors().iter().map(entry));
        out.extend(agent_domain::samples());
        out.extend(account_domain::samples());
        out.extend(project_domain::samples());
        out.extend(config_domain::samples());
        out.extend(services_domain::samples());
        // `OAuthError::HttpError` wraps a `reqwest::Error`, which the
        // crate does not let us construct. Its code and param name are
        // asserted from the impl's own shape instead.
        out.push((
            "oauth.http_error",
            ["detail".to_string()].into_iter().collect(),
        ));
        out
    }

    #[test]
    fn every_core_variant_has_a_vector() {
        let file = load();
        for (code, _) in core_samples() {
            assert!(
                file.iter().any(|v| v.code == code),
                "{code} has no entry in error-code-vectors.json",
            );
        }
    }

    #[test]
    fn no_core_vector_is_unclaimed() {
        let file = load();
        let claimed: BTreeSet<&str> = core_samples().iter().map(|(c, _)| *c).collect();
        for v in file.iter().filter(|v| v.owner == "core") {
            assert!(
                claimed.contains(v.code.as_str()),
                "{} is registered as a core code but no enum variant produces it",
                v.code,
            );
        }
    }

    #[test]
    fn vector_param_names_match_params_impl() {
        let file = load();
        for (code, names) in core_samples() {
            let v = file
                .iter()
                .find(|v| v.code == code)
                .unwrap_or_else(|| panic!("{code} missing from vectors"));
            let declared: BTreeSet<String> = v.params.iter().cloned().collect();
            assert_eq!(
                declared, names,
                "{code}: params() keys and the vectors file disagree",
            );
        }
    }

    #[test]
    fn every_vector_carries_a_sample_message() {
        for v in load() {
            assert!(
                !v.message.trim().is_empty(),
                "{}: a vector needs a sample English message — it is what a \
                 translator reads to write the localized sentence",
                v.code,
            );
            assert!(
                matches!(v.owner.as_str(), "core" | "command"),
                "{}: owner must be \"core\" or \"command\", got {:?}",
                v.code,
                v.owner,
            );
        }
    }

    #[test]
    fn codes_are_module_dot_snake_case() {
        for v in load() {
            let (module, variant) = v
                .code
                .split_once('.')
                .unwrap_or_else(|| panic!("{}: code must be <module>.<variant>", v.code));
            assert!(!module.is_empty() && !variant.is_empty(), "{}", v.code);
            // Digits are snake_case: `DecryptError::Base64` snake-cases
            // to `base64`, and the documented rule
            // (`<module>.<variant_snake_case>`) outranks a predicate
            // that happened to be written letters-only. Uppercase is
            // still rejected, which is what the check is actually for.
            assert!(
                v.code
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'),
                "{}: codes are lowercase snake_case",
                v.code,
            );
        }
    }

    #[test]
    fn params_never_carry_a_token() {
        // `rules/rust-conventions.md`: no `sk-ant-` string may reach a
        // user surface un-truncated, and `params` lands in the JS heap.
        let rendered: Vec<(&'static str, String)> = all_key_errors()
            .iter()
            .map(|e| (e.code(), e.params().to_string()))
            .chain(
                all_oauth_errors()
                    .iter()
                    .map(|e| (e.code(), e.params().to_string())),
            )
            .chain(
                all_project_errors()
                    .iter()
                    .map(|e| (e.code(), e.params().to_string())),
            )
            .collect();
        for (code, params_json) in rendered {
            assert!(
                secret_hits(&params_json).is_empty(),
                "{code}: params must never carry a token (matched: {:?})",
                secret_hits(&params_json),
            );
        }
    }

    #[test]
    fn message_field_is_the_display_string() {
        // The contract the GUI leans on: `ErrorDto.message` is exactly
        // what the CLI prints, so an untranslated code still renders
        // the English the user would have seen before.
        let e = OAuthError::RateLimited {
            retry_after_secs: 60,
        };
        assert_eq!(e.to_string(), "rate limited — retry after 60s");
        assert_eq!(e.code(), "oauth.rate_limited");
        assert_eq!(e.params()["retry_after_secs"], 60);
    }

    #[test]
    fn claude_running_exposes_the_path_the_cli_flag_hides() {
        // The message names `--force` because the CLI owns that text.
        // A GUI sentence has no such flag, so the path must be
        // reachable without parsing the English.
        let e = ProjectError::ClaudeRunning("/Users/me/proj".to_string());
        assert!(e.to_string().contains("--force"));
        assert_eq!(e.code(), "project.claude_running");
        assert_eq!(e.params()["path"], "/Users/me/proj");
    }
}

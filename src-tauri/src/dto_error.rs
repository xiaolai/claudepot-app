//! The command-error DTO — what a rejected `#[tauri::command]` sends
//! to the webview.
//!
//! `Result<T, String>` is the historical shape: core's `Display` text
//! concatenated with an English prefix written at the call site
//! (`format!("keys store open failed: {e}")`). That shape has no
//! machine identity, so the GUI can only render English, and the prefix
//! is written by the layer with the *least* idea of what the user was
//! doing.
//!
//! `ErrorDto` splits the two halves:
//!
//! - **`code`** — `<module>.<variant_snake_case>`, resolved against the
//!   renderer's `errors.*` catalog. See `claudepot_core::error_code`.
//! - **`params`** — the values the sentence interpolates, structured,
//!   so a translator can re-compose rather than re-parse.
//! - **`message`** — the English `Display` text, verbatim. Never empty.
//!   This is what renders when a code has no catalog entry, and it is
//!   why migrating a command needs no frontend change: the renderer's
//!   `extractMessage` duck-types any object carrying `message`.
//!
//! The operation prefix does **not** live here. It belongs to the
//! caller that knows what it was attempting ("Sign in failed: …"), and
//! that caller is the UI.
//!
//! Two caveats a fan-out has to respect:
//!
//! - **Never leave `message` empty**, and never fill it with anything
//!   other than the English text the command used to return. It is the
//!   whole reason a half-migrated command surface still renders.
//! - **A renderer call site that does `String(e)` or `` `${e}` ``
//!   instead of `renderError` will print `[object Object]`** once its
//!   command migrates. Check the consumers before converting a
//!   command; the ones still on the raw form are listed in
//!   `dev-docs/i18n-error-codes-seed.md`.

use claudepot_core::error_code::ErrorCode;
use claudepot_core::keys::KeyError;
use claudepot_core::oauth::error::OAuthError;
use claudepot_core::project::ProjectError;
use serde::Serialize;
use serde_json::{json, Value};

/// Codes for failures raised at a command site with no core error enum
/// behind them.
///
/// They are consts rather than inline literals so the vectors test can
/// close the loop in both directions: every const appears in
/// `error-code-vectors.json`, and every `owner: "command"` entry there
/// is one of these. A literal typed at a call site would silently mint
/// an unknown code, which degrades to English instead of failing.
///
/// Each of these is a candidate for promotion into a core enum. Until
/// then, this module is the single spelling.
pub mod codes {
    /// `accounts.db` would not open.
    pub const ACCOUNTS_STORE_OPEN_FAILED: &str = "accounts.store_open_failed";
    /// The account list query failed.
    pub const ACCOUNTS_LIST_FAILED: &str = "accounts.list_failed";
    /// A renderer-supplied account uuid did not parse.
    pub const ACCOUNTS_INVALID_UUID: &str = "accounts.invalid_uuid";
    /// A well-formed account uuid that matches no registered account.
    pub const ACCOUNTS_UNKNOWN_UUID: &str = "accounts.unknown_uuid";
    /// A `spawn_blocking` worker panicked or was cancelled.
    pub const APP_TASK_JOIN_FAILED: &str = "app.task_join_failed";
    /// A renderer-supplied key/token uuid did not parse.
    pub const KEYS_BAD_UUID: &str = "keys.bad_uuid";
    /// Add / rename submitted with an empty label.
    pub const KEYS_LABEL_REQUIRED: &str = "keys.label_required";
    /// Copy targeted an API key row that is no longer there.
    pub const KEYS_API_KEY_NOT_FOUND: &str = "keys.api_key_not_found";
    /// Copy targeted an OAuth token row that is no longer there.
    pub const KEYS_OAUTH_TOKEN_NOT_FOUND: &str = "keys.oauth_token_not_found";
    /// The OS clipboard write failed.
    pub const KEYS_CLIPBOARD_WRITE_FAILED: &str = "keys.clipboard_write_failed";
    /// `/v1/models` refused the key.
    pub const KEYS_PROBE_REJECTED: &str = "keys.probe_rejected";
    /// `/v1/models` rate-limited the probe.
    pub const KEYS_PROBE_RATE_LIMITED: &str = "keys.probe_rate_limited";
    /// The preferences mutex was poisoned — a previous holder panicked.
    /// Domain-scoped rather than `app.*` because every section reads
    /// preferences and a shared spelling would be claimed twice.
    pub const ACTIVITY_PREFS_LOCK_POISONED: &str = "activity.prefs_lock_poisoned";
    /// Save submitted with an empty (or whitespace-only) GitHub token.
    pub const SESSION_SHARE_TOKEN_EMPTY: &str = "session_share.token_empty";

    // --- accounts / auth / desktop (commands/{account,desktop,preferences}.rs)
    /// `LoginState`'s mutex was poisoned — a previous holder panicked.
    pub const ACCOUNTS_LOGIN_STATE_LOCK_POISONED: &str = "accounts.login_state_lock_poisoned";
    /// A second login was started while one is still running. One at a
    /// time because they share the temp `CLAUDE_CONFIG_DIR`.
    pub const ACCOUNTS_LOGIN_IN_PROGRESS: &str = "accounts.login_in_progress";
    /// A second `verify_all` was started while one is still running.
    pub const ACCOUNTS_VERIFY_ALL_IN_PROGRESS: &str = "accounts.verify_all_in_progress";
    /// `remove_account` failed.
    pub const ACCOUNTS_REMOVE_FAILED: &str = "accounts.remove_failed";
    /// The post-verify re-read of the account row failed.
    pub const ACCOUNTS_LOOKUP_FAILED: &str = "accounts.lookup_failed";
    /// `desktop_backend::create_platform()` returned `None` — no
    /// Desktop implementation for this OS. Distinct from
    /// `desktop_backend.not_installed`, which means the platform is
    /// supported but Claude Desktop is absent.
    pub const DESKTOP_NOT_SUPPORTED_ON_PLATFORM: &str = "desktop.not_supported_on_platform";
    /// Adopt found no signed-in Desktop session to adopt.
    pub const DESKTOP_NO_LIVE_IDENTITY: &str = "desktop.no_live_identity";
    /// `resolve_target` rejected the email. The helper lives in
    /// `commands/mod.rs` and still returns a prefixed `String`, so its
    /// English is carried through unchanged rather than re-framed here.
    pub const DESKTOP_RESOLVE_TARGET_FAILED: &str = "desktop.resolve_target_failed";
    /// The `PreferencesState` mutex was poisoned. Own segment rather
    /// than sharing `activity.prefs_lock_poisoned` for the reason given
    /// there — one spelling per owning surface.
    pub const PREFERENCES_LOCK_POISONED: &str = "preferences.lock_poisoned";
    /// Writing `preferences.json` failed. `Preferences::save` returns a
    /// pre-composed English string; it is carried verbatim.
    pub const PREFERENCES_SAVE_FAILED: &str = "preferences.save_failed";
    /// The renderer asked for a locale outside `SUPPORTED`.
    pub const PREFERENCES_UNSUPPORTED_LOCALE: &str = "preferences.unsupported_locale";
    /// macOS refused the Dock activation-policy change.
    pub const PREFERENCES_SET_ACTIVATION_POLICY_FAILED: &str =
        "preferences.set_activation_policy_failed";

    // --- config / permissions / rotation / retention -----------------
    /// A renderer-supplied grant duration outside the guard-rail range.
    pub const PERMISSION_DURATION_OUT_OF_RANGE: &str = "permission.duration_out_of_range";
    /// A permission mode string Claudepot will not grant.
    pub const PERMISSION_UNKNOWN_MODE: &str = "permission.unknown_mode";
    /// Revert / extend targeted a project with no Claudepot grant. A
    /// project elevated by hand-editing settings lands here, and that
    /// is the point — Claudepot does not revert someone's own choice.
    pub const PERMISSION_NO_ACTIVE_GRANT: &str = "permission.no_active_grant";
    /// The renderer sent a project path that is not an existing
    /// absolute directory.
    pub const PERMISSION_INVALID_PROJECT_PATH: &str = "permission.invalid_project_path";
    /// Reading one project's permission settings failed during the
    /// full-tree scan. Distinct from the core code because the failing
    /// project's path is the whole diagnostic value.
    pub const PERMISSION_READ_SETTINGS_FOR_PROJECT: &str =
        "permission.read_settings_for_project_failed";
    /// `permission-grants.json` would not load. The store returns a
    /// bare `io::Error` here — there is no core enum behind it, so the
    /// identity is minted at this boundary.
    pub const PERMISSION_GRANTS_LOAD_FAILED: &str = "permission.grants_load_failed";
    /// `rotation-rules.json` would not load. Same bare-`io::Error`
    /// shape as the grants store above.
    pub const ROTATION_RULES_LOAD_FAILED: &str = "rotation.rules_load_failed";
    /// A rotation rule / rules-file the renderer sent did not convert
    /// into its core shape (unknown mode, unknown window, …). The DTO
    /// boundary is the only place that can say which field.
    pub const ROTATION_BAD_RULE_DTO: &str = "rotation.bad_rule_dto";
    /// A confirmed swap failed to apply. The audit log records the same
    /// text under `RotationOutcome::Failed`.
    pub const ROTATION_APPLY_FAILED: &str = "rotation.apply_failed";
    /// The on-disk usage snapshot exists but did not deserialize, so a
    /// dry run cannot answer "would this rule fire". A *missing*
    /// snapshot is not this — that is a normal `would_fire: false`
    /// result with a reason, never an error.
    pub const ROTATION_SNAPSHOT_PARSE_FAILED: &str = "rotation.snapshot_parse_failed";
    /// An `on_conflict` string outside `refuse` / `suffix`.
    pub const ARTIFACT_UNKNOWN_ON_CONFLICT: &str = "artifact.unknown_on_conflict";
    /// An artifact-kind string outside Skill / Agent / Command.
    pub const ARTIFACT_UNKNOWN_KIND: &str = "artifact.unknown_kind";
    /// The renderer-supplied `relative_path` failed the traversal
    /// guard: empty, backslash-separated, or carrying `.` / `..` /
    /// a root. One code rather than five — every case has the same
    /// stake (a caller that is not our renderer) and the same remedy,
    /// and the specific reason rides as `detail`.
    pub const ARTIFACT_INVALID_RELATIVE_PATH: &str = "artifact.invalid_relative_path";
    /// The claimed `scope_root` is not one of the backend's active
    /// roots. Without this the renderer could name any directory.
    pub const ARTIFACT_SCOPE_ROOT_NOT_ACTIVE: &str = "artifact.scope_root_not_active";
    /// A disabled-artifact preview was asked for a path that is not in
    /// the `.disabled` tree.
    pub const ARTIFACT_NOT_DISABLED: &str = "artifact.not_disabled";
    /// Opening the disabled artifact for preview failed.
    pub const ARTIFACT_PREVIEW_OPEN_FAILED: &str = "artifact.preview_open_failed";
    /// Reading the disabled artifact's bytes failed after a successful
    /// open.
    pub const ARTIFACT_PREVIEW_READ_FAILED: &str = "artifact.preview_read_failed";
    /// `config_preview` was handed a node id the committed tree has no
    /// file for — usually a stale renderer holding ids from a previous
    /// scan generation.
    pub const CONFIG_NODE_NOT_FOUND: &str = "config.node_not_found";
    /// A command that needs the scanned tree ran before any scan
    /// committed one.
    pub const CONFIG_TREE_NOT_SCANNED: &str = "config.tree_not_scanned";
    /// Opening a config file for preview failed.
    pub const CONFIG_PREVIEW_OPEN_FAILED: &str = "config.preview_open_failed";
    /// `stat`ing a config file for preview failed.
    pub const CONFIG_PREVIEW_STAT_FAILED: &str = "config.preview_stat_failed";
    /// Set-default submitted with an empty editor id.
    pub const CONFIG_EDITOR_ID_EMPTY: &str = "config.editor_id_empty";
    /// A file-kind string outside `config_dto::kind_from_str`'s set.
    pub const CONFIG_UNKNOWN_KIND: &str = "config.unknown_kind";
    /// Open-in-editor called with an empty path.
    pub const CONFIG_PATH_EMPTY: &str = "config.path_empty";
    /// The requested editor id is not among the detected editors.
    pub const CONFIG_EDITOR_NOT_DETECTED: &str = "config.editor_not_detected";
    /// No editor could be resolved — neither the per-kind default, the
    /// fallback, nor the `system` opener.
    pub const CONFIG_NO_EDITOR_AVAILABLE: &str = "config.no_editor_available";
    /// Search started with an empty `search_id`.
    pub const CONFIG_SEARCH_ID_EMPTY: &str = "config.search_id_empty";
    /// The search-cancel-token registry's mutex was poisoned.
    pub const CONFIG_SEARCH_REGISTRY_LOCK_POISONED: &str = "config.search_registry_lock_poisoned";
    /// Effective *settings* asked for with no project anchor. Kept
    /// separate from the MCP case below because the two surfaces gate
    /// differently in the UI and each needs its own remediation.
    pub const CONFIG_NO_ANCHOR_EFFECTIVE_SETTINGS: &str = "config.no_anchor_effective_settings";
    /// Effective *MCP* asked for with no project anchor.
    pub const CONFIG_NO_ANCHOR_EFFECTIVE_MCP: &str = "config.no_anchor_effective_mcp";

    // ── env-secret vault + per-project `.env` ──────────────────────
    //
    // These share the `env_file.*` prefix with `EnvEditError`'s core
    // codes. That is deliberate — the user sees one surface — and the
    // names cannot collide because `EnvEditError`'s five variants are
    // all `key_*` / `already_*` / `invalid_key_name` / `value_*`.
    //
    // **None of these carries a `.env` value.** Every param below is a
    // path, a file name, or a key name. `params` lands in the JS heap
    // (rules/architecture.md, "IPC trust + secret direction"), and this
    // module's whole point is that a secret leaves Rust only through
    // the clipboard.
    /// A renderer-supplied project path failed `validate_project_path`.
    pub const ENV_FILE_INVALID_PROJECT_PATH: &str = "env_file.invalid_project_path";
    /// A `.env*` file exists but could not be read.
    pub const ENV_FILE_READ_FAILED: &str = "env_file.read_failed";
    /// A `.env*` file is past the editor's size cap.
    pub const ENV_FILE_TOO_LARGE: &str = "env_file.too_large";
    /// A `.env*` file is not valid UTF-8. The code says "encoding"
    /// rather than "utf8" because a code may carry no digits.
    pub const ENV_FILE_INVALID_ENCODING: &str = "env_file.invalid_encoding";
    /// A dotenv path with no usable file-name component.
    pub const ENV_FILE_INVALID_DOTENV_PATH: &str = "env_file.invalid_dotenv_path";
    /// The sidecar lock file could not be opened.
    pub const ENV_FILE_LOCK_OPEN_FAILED: &str = "env_file.lock_open_failed";
    /// The sidecar lock could not be acquired.
    pub const ENV_FILE_LOCK_FAILED: &str = "env_file.lock_failed";
    /// Empty `.env*` file name from the renderer.
    pub const ENV_FILE_NAME_EMPTY: &str = "env_file.name_empty";
    /// A `.env*` file name that could escape the project root.
    pub const ENV_FILE_NAME_UNSAFE: &str = "env_file.name_unsafe";
    /// A file name that is not `.env` or `.env.<suffix>`.
    pub const ENV_FILE_NAME_NOT_DOTENV: &str = "env_file.name_not_dotenv";
    /// The project root could not be listed.
    pub const ENV_FILE_READ_DIR_FAILED: &str = "env_file.read_dir_failed";
    /// A directory entry could not be read while scanning.
    pub const ENV_FILE_DIR_ENTRY_FAILED: &str = "env_file.dir_entry_failed";
    /// A directory entry could not be stat'd while scanning.
    pub const ENV_FILE_STAT_ENTRY_FAILED: &str = "env_file.stat_entry_failed";
    /// The edited `.env*` file could not be written back.
    pub const ENV_FILE_WRITE_FAILED: &str = "env_file.write_failed";
    /// Copy-out targeted a key that is in neither an active nor a
    /// commented line.
    pub const ENV_FILE_KEY_MISSING_IN_FILE: &str = "env_file.key_missing_in_file";

    // ── notification routing + log (commands/notification.rs) ───────
    //
    // `NotificationLog` returns `std::io::Result`, not an enum, so
    // these stay command-owned until it grows one.
    /// `open -b <bundle id>` failed to bring the host app forward.
    pub const NOTIFICATION_HOST_ACTIVATE_FAILED: &str = "notification.host_activate_failed";
    /// Appending an entry to `notifications.json` failed.
    pub const NOTIFICATION_LOG_APPEND_FAILED: &str = "notification.log_append_failed";
    /// Appending a *routed* entry failed.
    pub const NOTIFICATION_LOG_APPEND_ROUTED_FAILED: &str = "notification.log_append_routed_failed";
    /// Marking every entry read failed.
    pub const NOTIFICATION_LOG_MARK_ALL_READ_FAILED: &str = "notification.log_mark_all_read_failed";
    /// Marking one entry delivered on a surface failed.
    pub const NOTIFICATION_LOG_MARK_DELIVERED_FAILED: &str =
        "notification.log_mark_delivered_failed";
    /// Wiping the log failed.
    pub const NOTIFICATION_LOG_CLEAR_FAILED: &str = "notification.log_clear_failed";

    // ── boards (commands/board.rs) ─────────────────────────────────
    /// Export was handed a relative path. The renderer must pick a
    /// destination; a bare filename resolves against whatever working
    /// directory the OS launched the app from.
    pub const BOARD_EXPORT_PATH_NOT_ABSOLUTE: &str = "board.export_path_not_absolute";
    /// Export refused to overwrite an existing file.
    pub const BOARD_EXPORT_PATH_EXISTS: &str = "board.export_path_exists";
    /// Export could not create the destination file.
    pub const BOARD_EXPORT_CREATE_FAILED: &str = "board.export_create_failed";

    // ── routes / Providers (commands/routes.rs) ────────────────────
    //
    // Form-validation rejections, one per rule, so the Providers form
    // can highlight the field rather than pattern-match English. None
    // of them carries a provider secret: the DTO's secret fields are
    // zeroized on every error exit before the rejection is built.
    /// `provider_kind` was not one of the four known kinds.
    pub const ROUTES_UNKNOWN_PROVIDER_KIND: &str = "routes.unknown_provider_kind";
    /// A renderer-supplied route id did not parse as a uuid.
    pub const ROUTES_INVALID_ROUTE_ID: &str = "routes.invalid_route_id";
    /// `provider_kind` said gateway but no gateway block arrived.
    pub const ROUTES_GATEWAY_CONFIG_MISSING: &str = "routes.gateway_config_missing";
    /// `provider_kind` said bedrock but no bedrock block arrived.
    pub const ROUTES_BEDROCK_CONFIG_MISSING: &str = "routes.bedrock_config_missing";
    /// `provider_kind` said vertex but no vertex block arrived.
    pub const ROUTES_VERTEX_CONFIG_MISSING: &str = "routes.vertex_config_missing";
    /// `provider_kind` said foundry but no foundry block arrived.
    pub const ROUTES_FOUNDRY_CONFIG_MISSING: &str = "routes.foundry_config_missing";
    /// The gateway base URL failed `normalize_gateway_base_url`.
    pub const ROUTES_INVALID_GATEWAY_BASE_URL: &str = "routes.invalid_gateway_base_url";
    /// Bedrock submitted with a blank region.
    pub const ROUTES_BEDROCK_REGION_REQUIRED: &str = "routes.bedrock_region_required";
    /// Bedrock submitted with no bearer token, no AWS profile, and
    /// `skip_aws_auth` unset.
    pub const ROUTES_BEDROCK_AUTH_REQUIRED: &str = "routes.bedrock_auth_required";
    /// The optional Bedrock override URL failed validation.
    pub const ROUTES_INVALID_BEDROCK_BASE_URL: &str = "routes.invalid_bedrock_base_url";
    /// Vertex submitted with a blank GCP project id.
    pub const ROUTES_VERTEX_PROJECT_ID_REQUIRED: &str = "routes.vertex_project_id_required";
    /// The optional Vertex override URL failed validation.
    pub const ROUTES_INVALID_VERTEX_BASE_URL: &str = "routes.invalid_vertex_base_url";
    /// Foundry submitted with neither a base URL nor a resource name.
    pub const ROUTES_FOUNDRY_TARGET_REQUIRED: &str = "routes.foundry_target_required";
    /// Foundry submitted with both a base URL and a resource name.
    pub const ROUTES_FOUNDRY_TARGET_AMBIGUOUS: &str = "routes.foundry_target_ambiguous";
    /// The optional Foundry override URL failed validation.
    pub const ROUTES_INVALID_FOUNDRY_BASE_URL: &str = "routes.invalid_foundry_base_url";
    /// A wrapper name (typed or derived) failed `sanitize_wrapper_name`.
    /// Distinct from core's `routes.invalid_wrapper_name`, which the
    /// store raises; this one is the form gate.
    pub const ROUTES_WRAPPER_NAME_REJECTED: &str = "routes.wrapper_name_rejected";
    /// The route was not in the store on the re-read that follows a
    /// successful persist — a torn write or a concurrent removal.
    pub const ROUTES_GONE_AFTER_PERSIST: &str = "routes.gone_after_persist";
    /// `edit_route` failed in its store phase. Carries the guidance the
    /// phase distinction exists to give (the previously-saved route is
    /// still active).
    pub const ROUTES_EDIT_STORE_FAILED: &str = "routes.edit_store_failed";
    /// The route persisted but a follow-up write (wrapper, Desktop
    /// profile) reported a warning, so on-disk artifacts may be stale.
    pub const ROUTES_SAVE_WARNINGS: &str = "routes.save_warnings";
    /// The route was removed but one or more teardown steps failed,
    /// leaving a wrapper, profile, keychain entry, or helper behind.
    pub const ROUTES_REMOVE_CLEANUP_FAILED: &str = "routes.remove_cleanup_failed";
    /// Restart was asked for on a platform with no Desktop backend.
    pub const ROUTES_DESKTOP_NOT_SUPPORTED: &str = "routes.desktop_not_supported";

    // ── projects / sessions / memory (commands/{project,repair,
    //    session_index,session_prune,memory}.rs) ────────────────────
    //
    // Every param below is a path, a count, a scope name or an English
    // `detail`. None of them can carry transcript bytes: the commands
    // that read a `.jsonl` fail with a core `SessionError` / `SlimError`
    // / `RedactError`, all of which already own their identity.
    /// `project_move_dry_run` noticed a newer request superseded this
    /// one and bailed. **Not a failure** — the renderer discards it.
    ///
    /// It travels in the error channel because that is the shape the
    /// command has, and `RenameProjectModal` matches the sentinel by
    /// substring off `extractMessage(e)`. `message` therefore carries
    /// the sentinel string *verbatim* and nothing else; changing that
    /// text makes the preview pane flash an error every time the user
    /// types another character. The cleaner shape is an
    /// `Option<DryRunPlan>` return, which is a frontend change and a
    /// different slice.
    pub const PROJECT_DRY_RUN_SUPERSEDED: &str = "project.dry_run_superseded";
    /// Clean refused because rename journals are still pending. The
    /// remedy is the Repair view, so the count is the whole payload.
    pub const PROJECT_CLEAN_BLOCKED_BY_JOURNALS: &str = "project.clean_blocked_by_journals";
    /// Remove refused for the same reason as the clean gate above. Two
    /// codes rather than one because the sentence names the verb the
    /// user pressed, and a shared code could only say "this action".
    pub const PROJECT_REMOVE_BLOCKED_BY_JOURNALS: &str = "project.remove_blocked_by_journals";
    /// A repair action named a journal id that is no longer pending —
    /// usually a stale Repair view after another window resolved it.
    pub const REPAIR_JOURNAL_NOT_FOUND: &str = "repair.journal_not_found";
    /// Break-lock found no lock file for the named project.
    pub const REPAIR_LOCK_FILE_NOT_FOUND: &str = "repair.lock_file_not_found";
    /// An export-format string outside `md` / `markdown` / `json`.
    pub const SESSION_EXPORT_UNKNOWN_FORMAT: &str = "session_export.unknown_format";
    /// The renderer-supplied destination is not a usable output path:
    /// relative, carrying `..`, or with no parent / file name. One code
    /// rather than four — every case has the same stake (a destination
    /// that did not come from the native save dialog) and the same
    /// remedy (pick another one); the specific reason rides as
    /// `detail`. Same reasoning as `ARTIFACT_INVALID_RELATIVE_PATH`.
    pub const SESSION_EXPORT_INVALID_OUTPUT_PATH: &str = "session_export.invalid_output_path";
    /// The destination is a symlink. Its own code, not folded into the
    /// one above: the path is well-formed and the remedy is different
    /// (delete the link, or export beside it).
    pub const SESSION_EXPORT_OUTPUT_IS_SYMLINK: &str = "session_export.output_is_symlink";
    /// A step of the atomic write pipeline failed — open, write, fsync,
    /// stat, chmod or rename. One code because the user-facing sentence
    /// and the remedy are identical for all of them; which step it was
    /// rides as `detail`.
    pub const SESSION_EXPORT_WRITE_FAILED: &str = "session_export.write_failed";
    /// The destination filesystem would not hold 0600, so the export
    /// was refused rather than written world-readable. A refusal, not
    /// an I/O failure: the remedy is a different filesystem.
    pub const SESSION_EXPORT_PERMISSIONS_NOT_ENFORCED: &str =
        "session_export.permissions_not_enforced";
    /// A memory command arrived with an empty `project_root`.
    pub const MEMORY_PROJECT_ROOT_EMPTY: &str = "memory.project_root_empty";
    /// Enumerating a project's memory files failed. `enumerate_project_memory`
    /// returns a bare `std::io::Result`, so the identity is minted here.
    pub const MEMORY_ENUMERATE_FAILED: &str = "memory.enumerate_failed";
    /// An auto-memory scope string outside `user` / `local_project`.
    pub const MEMORY_UNKNOWN_SCOPE: &str = "memory.unknown_scope";
    /// Writing `autoMemoryEnabled` failed. Wraps `SettingsWriteError`,
    /// which has no `ErrorCode` yet — promote both of these to
    /// `ErrorDto::from_core` when the settings slice converts it.
    pub const MEMORY_AUTO_MEMORY_WRITE_FAILED: &str = "memory.auto_memory_write_failed";
    /// Clearing `autoMemoryEnabled` from a layer failed. Distinct from
    /// the write above because clearing is a separate user action with
    /// a separate undo.
    pub const MEMORY_AUTO_MEMORY_CLEAR_FAILED: &str = "memory.auto_memory_clear_failed";

    // ── agents (commands/agents.rs) ────────────────────────────────
    //
    // **Plural `agents.*` on purpose.** Core's `AgentError` owns the
    // singular `agent.*` namespace; these are raised by the Agents
    // command surface before (or beside) a core call, and the two sets
    // must not collide. Where a command site restates a condition core
    // also names — "not found", "name already taken" — the code stays
    // separate because the English here is the GUI form's and is frozen
    // at this boundary, while core's is the CLI's and must not move.
    //
    // **A draft is inert.** `agents.draft_not_runnable` is the only
    // code on this surface that mentions a draft, and its whole job is
    // to say the run did *not* happen: a draft schedules nothing and
    // executes nothing until a human arms it through the install
    // review. No message or param here may read as "already running" or
    // "already armed".
    //
    // Cron strings, permission modes, model ids, lifecycle wire values
    // and artifact paths are data: they may ride in `params`, and they
    // are never translated.
    /// A renderer-supplied agent uuid did not parse.
    pub const AGENTS_INVALID_ID: &str = "agents.invalid_id";
    /// A well-formed agent uuid the store has no record for.
    pub const AGENTS_NOT_FOUND: &str = "agents.not_found";
    /// Add rejected because the name is already in the store.
    pub const AGENTS_NAME_TAKEN: &str = "agents.name_taken";
    /// Template instantiation rejected for the same reason. Its own
    /// code because the remedy it states is template-specific (rename
    /// or remove before instantiating the template again).
    pub const AGENTS_TEMPLATE_NAME_TAKEN: &str = "agents.template_name_taken";
    /// A `permission_mode` wire string outside `parse_permission_mode`.
    pub const AGENTS_INVALID_PERMISSION_MODE: &str = "agents.invalid_permission_mode";
    /// An `output_format` wire string outside `parse_output_format`.
    pub const AGENTS_INVALID_OUTPUT_FORMAT: &str = "agents.invalid_output_format";
    /// `bypassPermissions` submitted with an empty allowed-tools list.
    /// The whitelist is what bounds an unattended run, so there is no
    /// "allow everything" shape of this agent.
    pub const AGENTS_BYPASS_REQUIRES_ALLOWED_TOOLS: &str = "agents.bypass_requires_allowed_tools";
    /// `binary_kind = "route"` with no `binary_route_id` beside it.
    pub const AGENTS_ROUTE_BINARY_ID_REQUIRED: &str = "agents.route_binary_id_required";
    /// A `binary_kind` outside first_party / route.
    pub const AGENTS_UNKNOWN_BINARY_KIND: &str = "agents.unknown_binary_kind";
    /// An `event_kind` outside the one kind v1 ships. Rejected loudly
    /// rather than defaulted — an unknown value means the payload was
    /// hand-crafted.
    pub const AGENTS_UNKNOWN_EVENT_KIND: &str = "agents.unknown_event_kind";
    /// A `max_budget_usd` that is negative or not finite.
    pub const AGENTS_INVALID_MAX_BUDGET: &str = "agents.invalid_max_budget";
    /// Run-now refused because the agent is still a draft. **Not** a
    /// report that something ran: nothing was spawned, and arming it
    /// through the install review is the gate.
    pub const AGENTS_DRAFT_NOT_RUNNABLE: &str = "agents.draft_not_runnable";
    /// Artifact preview asked for on a platform with no scheduler
    /// adapter compiled in.
    ///
    /// `allow(dead_code)` because its one call site is the
    /// `cfg(not(any(macos, linux, windows)))` arm of
    /// `agents_dry_run_artifact`, which compiles out on all three
    /// shipped targets. The const still has to exist unconditionally:
    /// `codes::ALL` and the vectors file lock the code set for *every*
    /// build, and cfg-gating it here would fail that check on macOS.
    #[allow(dead_code)]
    pub const AGENTS_NO_SCHEDULER_ADAPTER: &str = "agents.no_scheduler_adapter";
    /// The active scheduler reports no artifact directory to reveal.
    pub const AGENTS_NO_ARTIFACT_DIR: &str = "agents.no_artifact_dir";
    /// The OS file manager would not open the artifact directory —
    /// either the opener failed to start or it exited non-zero.
    pub const AGENTS_OPEN_ARTIFACT_DIR_FAILED: &str = "agents.open_artifact_dir_failed";
    /// systemd linger was asked for off Linux.
    pub const AGENTS_LINGER_LINUX_ONLY: &str = "agents.linger_linux_only";

    // ── templates (commands/templates.rs) ──────────────────────────
    //
    // A template install ends in `agents_add`, so these move with the
    // agents block above. Blueprint ids, placeholder names and
    // host-platform names are data and are never translated.
    /// The bundled template registry would not load.
    pub const TEMPLATES_REGISTRY_LOAD_FAILED: &str = "templates.registry_load_failed";
    /// A blueprint id the bundled registry has no entry for. Shared
    /// with `agent_add_from_template`, which dispatches on the same id
    /// space.
    pub const TEMPLATES_UNKNOWN_ID: &str = "templates.unknown_id";
    /// The template exists but ships no sample report.
    pub const TEMPLATES_NO_SAMPLE_REPORT: &str = "templates.no_sample_report";
    /// A report path that would not canonicalize.
    pub const TEMPLATES_REPORT_PATH_UNRESOLVABLE: &str = "templates.report_path_unresolvable";
    /// A report path outside `~/.claudepot/`. The reader is scoped to
    /// that tree; anything else is refused rather than read.
    pub const TEMPLATES_REPORT_PATH_OUTSIDE_ROOT: &str = "templates.report_path_outside_root";
    /// `stat`ing the report failed.
    pub const TEMPLATES_REPORT_STAT_FAILED: &str = "templates.report_stat_failed";
    /// The report path resolves to something that is not a file.
    pub const TEMPLATES_REPORT_NOT_A_FILE: &str = "templates.report_not_a_file";
    /// The report is past the reader's size cap.
    pub const TEMPLATES_REPORT_TOO_LARGE: &str = "templates.report_too_large";
    /// Reading the report's bytes failed after a successful stat.
    pub const TEMPLATES_REPORT_READ_FAILED: &str = "templates.report_read_failed";
    /// The blueprint does not declare support for this host platform.
    /// Enforced here as well as in the gallery filter so a direct IPC
    /// call cannot install a macOS-only template on Linux.
    pub const TEMPLATES_PLATFORM_UNSUPPORTED: &str = "templates.platform_unsupported";
    /// A placeholder value arrived with the wrong JSON type.
    pub const TEMPLATES_PLACEHOLDER_TYPE_MISMATCH: &str = "templates.placeholder_type_mismatch";
    /// The wire schedule did not convert into its core shape.
    pub const TEMPLATES_INVALID_SCHEDULE: &str = "templates.invalid_schedule";
    /// `templates::instantiate` rejected the instance. `TemplateError`
    /// has no `ErrorCode` yet — promote this to `ErrorDto::from_core`
    /// when the templates enum converts.
    pub const TEMPLATES_INSTANTIATE_FAILED: &str = "templates.instantiate_failed";
    /// Every numeric suffix the uniquifier tries was already taken.
    pub const TEMPLATES_UNIQUE_NAME_EXHAUSTED: &str = "templates.unique_name_exhausted";

    // ── shared memory / Knowledge (commands/shared_memory.rs) ──────
    //
    // Three module segments for one file because the user sees three
    // surfaces: the memory/decision/evidence store (`shared_memory.*`),
    // the lesson triage queue (`lesson.*`), and recurrence tracking
    // (`recurrence.*`). One shared prefix would make every sentence
    // start by disambiguating which of the three it meant.
    //
    // These are command-owned rather than promoted into
    // `shared_memory::durable::DurableError`: the English each one
    // carries is the *command's* framing (`"list: "`, `"archive: "`),
    // and per `claudepot_core::error_code` a wrapper that adds English
    // owns that framing. The same `DurableError::Sql` reaching six
    // different commands is six different sentences to the user.
    //
    // Nothing here carries transcript content. `detail` is core's
    // `Display` text — a sqlite complaint or a locator — never a body,
    // which is why redaction stays on the success path.
    /// The shared `SessionIndex` failed to open at startup, so every
    /// command in this file is unavailable. The UI renders this as a
    /// banner with a rebuild path, not as a per-action toast.
    pub const SHARED_MEMORY_INDEX_UNAVAILABLE: &str = "shared_memory.index_unavailable";
    /// Full-text search over indexed transcripts failed.
    pub const SHARED_MEMORY_SEARCH_FAILED: &str = "shared_memory.search_failed";
    /// Reading a conversation excerpt failed for a reason that is not
    /// "it's gone" — a locked database, a permissions problem.
    pub const SHARED_MEMORY_READ_FAILED: &str = "shared_memory.read_failed";
    /// The excerpt's transcript is unknown to the index or missing on
    /// disk. Split from `SHARED_MEMORY_READ_FAILED` because this is the
    /// one the user can act on (the transcript moved or was pruned) and
    /// it is the code that inherited the deleted `toExcerptError`
    /// regex's remediation copy. Classified from the error *variant*,
    /// never from matching English — a permission failure is not this.
    pub const SHARED_MEMORY_EXCERPT_UNAVAILABLE: &str = "shared_memory.excerpt_unavailable";
    /// Listing durable memories failed.
    pub const SHARED_MEMORY_LIST_MEMORIES_FAILED: &str = "shared_memory.list_memories_failed";
    /// A memory-scope string outside `global` / `project`.
    pub const SHARED_MEMORY_INVALID_SCOPE: &str = "shared_memory.invalid_scope";
    /// A memory-kind string outside the five `durable::MemoryKind`s.
    pub const SHARED_MEMORY_INVALID_KIND: &str = "shared_memory.invalid_kind";
    /// Writing a new durable memory failed.
    pub const SHARED_MEMORY_CREATE_MEMORY_FAILED: &str = "shared_memory.create_memory_failed";
    /// Archiving a memory failed.
    pub const SHARED_MEMORY_ARCHIVE_MEMORY_FAILED: &str = "shared_memory.archive_memory_failed";
    /// Listing decisions failed.
    pub const SHARED_MEMORY_LIST_DECISIONS_FAILED: &str = "shared_memory.list_decisions_failed";
    /// Logging (or superseding) a decision failed.
    pub const SHARED_MEMORY_LOG_DECISION_FAILED: &str = "shared_memory.log_decision_failed";
    /// Archiving a decision failed.
    pub const SHARED_MEMORY_ARCHIVE_DECISION_FAILED: &str = "shared_memory.archive_decision_failed";
    /// Listing audit-fix evidence failed.
    pub const SHARED_MEMORY_LIST_EVIDENCE_FAILED: &str = "shared_memory.list_evidence_failed";
    /// `memory_links` called with none of the three parent ids.
    pub const SHARED_MEMORY_LINK_TARGET_REQUIRED: &str = "shared_memory.link_target_required";
    /// Reading a durable row's provenance links failed.
    pub const SHARED_MEMORY_LINKS_FAILED: &str = "shared_memory.links_failed";
    /// Enumerating indexed sessions failed.
    pub const SHARED_MEMORY_LIST_SESSIONS_FAILED: &str = "shared_memory.list_sessions_failed";
    /// Enumerating indexed projects failed.
    pub const SHARED_MEMORY_LIST_PROJECTS_FAILED: &str = "shared_memory.list_projects_failed";
    /// An MCP-snippet install scope outside `user` / `project`. Distinct
    /// from `SHARED_MEMORY_INVALID_SCOPE` — different vocabulary, and
    /// the installer's remedy names a directory rather than a memory.
    pub const SHARED_MEMORY_INVALID_INSTALL_SCOPE: &str = "shared_memory.invalid_install_scope";
    /// The renderer's `out` override was not an absolute path. Core's
    /// `out` arm is the trusted CLI escape hatch and validates nothing,
    /// so this boundary is the gate.
    pub const SHARED_MEMORY_OUT_PATH_NOT_ABSOLUTE: &str = "shared_memory.out_path_not_absolute";
    /// `std::env::current_exe()` failed, so the sibling CLI cannot be
    /// located for the MCP probe.
    pub const SHARED_MEMORY_CURRENT_EXE_FAILED: &str = "shared_memory.current_exe_failed";
    /// `current_exe()` returned a path with no parent directory.
    pub const SHARED_MEMORY_CURRENT_EXE_NO_PARENT: &str = "shared_memory.current_exe_no_parent";
    /// A review-state string outside proposed / accepted / rejected /
    /// suspect / all.
    pub const LESSON_BAD_STATE: &str = "lesson.bad_state";
    /// Listing the lesson triage queue failed.
    pub const LESSON_LIST_FAILED: &str = "lesson.list_failed";
    /// Counting lessons by review state failed.
    pub const LESSON_COUNTS_FAILED: &str = "lesson.counts_failed";
    /// The dashboard's per-project coverage grid failed to load.
    pub const LESSON_COUNTS_BY_PROJECT_FAILED: &str = "lesson.counts_by_project_failed";
    /// Accepting a proposed lesson failed.
    pub const LESSON_ACCEPT_FAILED: &str = "lesson.accept_failed";
    /// Rejecting a proposed lesson failed.
    pub const LESSON_REJECT_FAILED: &str = "lesson.reject_failed";
    /// Listing pending recurrence candidates failed.
    pub const RECURRENCE_LIST_FAILED: &str = "recurrence.list_failed";
    /// Confirming a recurrence failed.
    pub const RECURRENCE_CONFIRM_FAILED: &str = "recurrence.confirm_failed";
    /// Dismissing a recurrence candidate failed.
    pub const RECURRENCE_DISMISS_FAILED: &str = "recurrence.dismiss_failed";
    /// The confirmed-in-window count failed. Its own code rather than
    /// sharing the pending one's: the dashboard renders the two numbers
    /// separately and either can fail alone.
    pub const RECURRENCE_CONFIRMED_COUNT_FAILED: &str = "recurrence.confirmed_count_failed";
    /// The pending-candidate count failed.
    pub const RECURRENCE_PENDING_COUNT_FAILED: &str = "recurrence.pending_count_failed";

    // ── core command surface (commands/mod.rs) ─────────────────────
    //
    // `commands/mod.rs` is the shell's own surface: the account read
    // path every section mounts against, plus the three OS-shell verbs
    // (reveal, unlock, open the log directory). The shell verbs take
    // the `app.*` segment they share with `app.task_join_failed` —
    // they belong to no domain noun, and giving each of them a
    // per-file segment would mint three one-entry namespaces.
    //
    // Every param below is a path or an English `detail`. The reveal
    // verbs take a renderer-supplied path and hand it to the OS file
    // manager; nothing on this surface reads a credential.
    /// `reconcile_all` failed. `ReconcileError` has no `ErrorCode` yet
    /// — promote this to `ErrorDto::from_core` when the services slice
    /// converts it.
    pub const ACCOUNTS_RECONCILE_FAILED: &str = "accounts.reconcile_failed";
    /// Keychain unlock asked for off macOS. The native unlock dialog
    /// is a `/usr/bin/security` affordance with no counterpart on
    /// Linux/Windows, where the secret backends are unlocked with the
    /// session.
    ///
    /// `allow(dead_code)` for the same reason
    /// `AGENTS_NO_SCHEDULER_ADAPTER` carries it: the one call site is
    /// the `cfg(not(target_os = "macos"))` arm of `unlock_keychain`, so
    /// on a macOS build nothing reads it. The const still has to exist
    /// unconditionally — `codes::ALL` and the vectors file lock the
    /// code set for *every* build.
    #[allow(dead_code)]
    pub const APP_KEYCHAIN_UNLOCK_UNSUPPORTED: &str = "app.keychain_unlock_unsupported";
    /// Reveal called with an empty path.
    pub const APP_REVEAL_PATH_EMPTY: &str = "app.reveal_path_empty";
    /// Reveal found neither the path nor any existing ancestor. The
    /// walk up the tree is why this is rare: it fires only when the
    /// whole branch is gone, so the path is the entire diagnostic.
    pub const APP_REVEAL_PATH_NOT_FOUND: &str = "app.reveal_path_not_found";
    /// The OS file manager would not start. One code for all three
    /// platforms — `open` / `xdg-open` / `explorer` differ only in the
    /// name inside `detail`, and the remedy is identical.
    pub const APP_REVEAL_SPAWN_FAILED: &str = "app.reveal_spawn_failed";
    /// The OS file manager started and then reported a failure. Split
    /// from the spawn code above because that one means "no file
    /// manager" and this one means "it refused this path".
    pub const APP_REVEAL_FAILED: &str = "app.reveal_failed";
    /// The diagnostic log directory could not be created on first use.
    pub const APP_LOG_DIR_CREATE_FAILED: &str = "app.log_dir_create_failed";

    // ── keys: the two add-form gates (commands/keys.rs) ────────────
    //
    // Both are prefix checks on the pasted value, and **neither may
    // ever carry the value**. `params` is empty on purpose: the only
    // thing worth saying is which prefix was expected, and that is a
    // constant the sentence can spell itself.
    /// Add-API-key rejected a value that is not `sk-ant-api03-…`.
    pub const KEYS_NOT_AN_API_KEY: &str = "keys.not_an_api_key";
    /// Add-OAuth-token rejected a value that is not `sk-ant-oat01-…`.
    pub const KEYS_NOT_AN_OAUTH_TOKEN: &str = "keys.not_an_oauth_token";

    // ── CC / Desktop updates (commands/updates.rs) ─────────────────
    //
    // `claudepot_core::updates::UpdateError` has no `ErrorCode` yet, so
    // the four wrapper codes below ride core's own English in
    // `message` — same interim shape `templates.instantiate_failed`
    // carries. Promote them to `ErrorDto::from_core` when that enum
    // converts; the codes then disappear rather than being renamed.
    //
    // Version strings and channel names are data: they ride in
    // `params` and are never translated.
    /// The `updates.json` state mutex was poisoned.
    pub const UPDATES_STATE_LOCK_POISONED: &str = "updates.state_lock_poisoned";
    /// The `PollerGate` single-flight refused: the background poller or
    /// another click already holds the lease. A refusal, not a failure
    /// — nothing was installed and the retry is immediate.
    pub const UPDATES_OPERATION_IN_PROGRESS: &str = "updates.operation_in_progress";
    /// `claude update` did not complete.
    pub const UPDATES_CLI_INSTALL_FAILED: &str = "updates.cli_install_failed";
    /// The Desktop install did not complete.
    pub const UPDATES_DESKTOP_INSTALL_FAILED: &str = "updates.desktop_install_failed";
    /// A channel string outside `latest` / `stable`. CC's own values,
    /// not Claudepot's — see `release_update.unknown_channel` for
    /// Claudepot's separate stable/beta pair.
    pub const UPDATES_UNKNOWN_CHANNEL: &str = "updates.unknown_channel";
    /// Writing CC's `autoUpdatesChannel` (and its `minimumVersion`
    /// floor) failed.
    pub const UPDATES_CHANNEL_WRITE_FAILED: &str = "updates.channel_write_failed";
    /// Writing CC's `minimumVersion` on its own failed. Its own code
    /// because it is a separate user action with a separate undo.
    pub const UPDATES_MINIMUM_VERSION_WRITE_FAILED: &str = "updates.minimum_version_write_failed";

    // ── Claudepot's own self-updater (commands/release_update.rs) ──
    //
    // Distinct from `updates.*` above, which is about *CC and Claude
    // Desktop*. These are about the running Claudepot bundle, and the
    // two channel vocabularies do not overlap (`stable`/`beta` here,
    // `latest`/`stable` there) — one shared segment would make every
    // sentence start by saying which app it meant.
    /// A channel string outside `stable` / `beta`.
    pub const RELEASE_UPDATE_UNKNOWN_CHANNEL: &str = "release_update.unknown_channel";
    /// The stashed-`Update` mutex was poisoned.
    pub const RELEASE_UPDATE_STATE_LOCK_POISONED: &str = "release_update.state_lock_poisoned";
    /// A compiled-in manifest endpoint did not parse as a URL. Not
    /// user input — this is a build-time constant, so it means the
    /// release-channel table is wrong.
    pub const RELEASE_UPDATE_INVALID_ENDPOINT: &str = "release_update.invalid_endpoint";
    /// The updater builder refused the endpoint list.
    pub const RELEASE_UPDATE_ENDPOINT_CONFIG_FAILED: &str = "release_update.endpoint_config_failed";
    /// The updater would not build — usually a missing or malformed
    /// signing pubkey in `tauri.conf.json`.
    pub const RELEASE_UPDATE_BUILDER_FAILED: &str = "release_update.builder_failed";
    /// The manifest fetch / parse failed. **Not** "no update
    /// available", which is a successful check.
    pub const RELEASE_UPDATE_CHECK_FAILED: &str = "release_update.check_failed";
    /// Install called with nothing stashed — the renderer must run a
    /// check first. Nothing was downloaded and nothing was replaced.
    pub const RELEASE_UPDATE_NOT_STAGED: &str = "release_update.not_staged";
    /// The download / signature verification / swap failed. The stashed
    /// handle is put back, so a retry needs no fresh check.
    pub const RELEASE_UPDATE_INSTALL_FAILED: &str = "release_update.install_failed";

    // ── misc command surfaces (commands/{attribution,auto_dream,
    //    memory_health,cli,migrate,session_move,activity_cards,
    //    artifact_usage,usage_local,cc_doctor}.rs) ────────────────
    //
    // The long tail: small panes whose only ad-hoc rejections are wire
    // strings the renderer sent (a mode, a window kind, a pricing tier)
    // plus a handful of I/O failures with no core enum behind them.
    //
    // Every param below is a wire discriminant, a path, or an English
    // `detail`. None of these surfaces touches a credential: the tiers
    // and modes are enum spellings, and the `detail` values are core
    // `Display` text — a settings-file complaint, a uuid parse error, a
    // sqlite message.
    /// An attribution-mode string outside `default` / `off` / `custom`.
    pub const ATTRIBUTION_UNKNOWN_MODE: &str = "attribution.unknown_mode";
    /// An auto-dream mode string outside `default` / `on` / `off`. Its
    /// own code rather than sharing the attribution one above: the two
    /// sentences have to name different valid sets, and a shared code
    /// could only say "that value is not allowed".
    pub const AUTO_DREAM_UNKNOWN_MODE: &str = "auto_dream.unknown_mode";
    /// Auditing `CLAUDE.md` / `MEMORY.md` failed for a reason other than
    /// "the file is missing" — a missing file is reported inline as
    /// `missing: true`, never as an error. `build_report` returns a bare
    /// `std::io::Result`, so the identity is minted at this boundary.
    pub const MEMORY_HEALTH_AUDIT_FAILED: &str = "memory_health.audit_failed";
    /// `resolve_target` rejected the email on the CLI-swap path. Its own
    /// code rather than reusing `desktop.resolve_target_failed` for the
    /// reason given there — one spelling per owning surface, because the
    /// two panes offer different remediation.
    pub const CLI_RESOLVE_TARGET_FAILED: &str = "cli.resolve_target_failed";
    /// An `.age` bundle was inspected with no passphrase.
    pub const MIGRATE_PASSPHRASE_REQUIRED: &str = "migrate.passphrase_required";
    /// A conflict-mode string outside `skip` / `merge` / `replace`.
    pub const MIGRATE_UNKNOWN_CONFLICT_MODE: &str = "migrate.unknown_conflict_mode";
    /// A merge-preference string outside `imported` / `target`.
    pub const MIGRATE_UNKNOWN_MERGE_PREFERENCE: &str = "migrate.unknown_merge_preference";
    /// A renderer-supplied session uuid did not parse. Distinct from
    /// core's `session_move.session_not_found`, which means a well-formed
    /// id the slug directory has no transcript for.
    pub const SESSION_MOVE_INVALID_SESSION_ID: &str = "session_move.invalid_session_id";
    /// Adopt was pointed at a directory that does not exist. Checked
    /// here rather than in core because the renderer picked the path and
    /// this is the layer that can name it back.
    pub const SESSION_MOVE_TARGET_CWD_MISSING: &str = "session_move.target_cwd_missing";
    /// A cards query carried a card kind or severity outside the known
    /// set. One code rather than two — both are the same stake (a
    /// renderer that sent a value it did not get from us) and the same
    /// remedy; which field it was rides as `detail`.
    pub const ACTIVITY_CARDS_INVALID_QUERY: &str = "activity_cards.invalid_query";
    /// Seeking into a card's source transcript to fetch its body failed.
    /// The read is a bare `std::io` path with no core enum behind it.
    pub const ACTIVITY_CARDS_BODY_READ_FAILED: &str = "activity_cards.body_read_failed";
    /// An artifact-kind string `ArtifactKind::parse` does not know.
    pub const ARTIFACT_USAGE_UNKNOWN_KIND: &str = "artifact_usage.unknown_kind";
    /// A usage window kind outside `all` / `last_days`.
    pub const USAGE_LOCAL_UNKNOWN_WINDOW_KIND: &str = "usage_local.unknown_window_kind";
    /// `last_days` arrived without its `days` field.
    pub const USAGE_LOCAL_WINDOW_DAYS_REQUIRED: &str = "usage_local.window_days_required";
    /// A pricing-tier string outside `PriceTier::parse`'s set. Rejected
    /// loudly rather than defaulted: silently reverting to the default
    /// tier would re-price every figure on the dashboard without saying
    /// so.
    pub const USAGE_LOCAL_UNKNOWN_PRICING_TIER: &str = "usage_local.unknown_pricing_tier";
    /// The doctor snapshot cache's mutex was poisoned. Domain-scoped for
    /// the same reason as `activity.prefs_lock_poisoned` — one spelling
    /// per owning surface.
    pub const CC_DOCTOR_CACHE_LOCK_POISONED: &str = "cc_doctor.cache_lock_poisoned";
    /// The config-watcher handle's mutex was poisoned. `config.*`
    /// rather than a segment of its own: the watcher is the Config
    /// tree's live-refresh half, and the user sees one surface.
    pub const CONFIG_WATCH_STATE_LOCK_POISONED: &str = "config.watch_state_lock_poisoned";
    /// The watcher thread would not spawn, so the Config tree will not
    /// refresh itself. Not fatal to the pane — a manual rescan still
    /// works — which is why it is its own code rather than folded into
    /// a generic scan failure.
    pub const CONFIG_WATCH_START_FAILED: &str = "config.watch_start_failed";
    //
    // There is deliberately no `cc_doctor.reveal_log_failed`.
    // `cc_doctor_open_parse_failures_log` delegates to
    // `reveal_in_finder`, which already owns the `app.reveal_*` codes;
    // a wrapper code here would be a second identity for one failure
    // and a second catalog sentence to keep in step.

    /// Every const above. The vectors test asserts set equality with
    /// the file's `owner: "command"` entries — adding a const without a
    /// vector (or the reverse) fails the build. `cfg(test)` because the
    /// registry exists for that assertion and nothing else; the consts
    /// themselves are used at their call sites.
    #[cfg(test)]
    pub const ALL: &[&str] = &[
        ACCOUNTS_STORE_OPEN_FAILED,
        ACCOUNTS_LIST_FAILED,
        ACCOUNTS_INVALID_UUID,
        ACCOUNTS_UNKNOWN_UUID,
        APP_TASK_JOIN_FAILED,
        KEYS_BAD_UUID,
        KEYS_LABEL_REQUIRED,
        KEYS_API_KEY_NOT_FOUND,
        KEYS_OAUTH_TOKEN_NOT_FOUND,
        KEYS_CLIPBOARD_WRITE_FAILED,
        KEYS_PROBE_REJECTED,
        KEYS_PROBE_RATE_LIMITED,
        ACTIVITY_PREFS_LOCK_POISONED,
        SESSION_SHARE_TOKEN_EMPTY,
        ACCOUNTS_LOGIN_STATE_LOCK_POISONED,
        ACCOUNTS_LOGIN_IN_PROGRESS,
        ACCOUNTS_VERIFY_ALL_IN_PROGRESS,
        ACCOUNTS_REMOVE_FAILED,
        ACCOUNTS_LOOKUP_FAILED,
        DESKTOP_NOT_SUPPORTED_ON_PLATFORM,
        DESKTOP_NO_LIVE_IDENTITY,
        DESKTOP_RESOLVE_TARGET_FAILED,
        PREFERENCES_LOCK_POISONED,
        PREFERENCES_SAVE_FAILED,
        PREFERENCES_UNSUPPORTED_LOCALE,
        PREFERENCES_SET_ACTIVATION_POLICY_FAILED,
        PERMISSION_DURATION_OUT_OF_RANGE,
        PERMISSION_UNKNOWN_MODE,
        PERMISSION_NO_ACTIVE_GRANT,
        PERMISSION_INVALID_PROJECT_PATH,
        PERMISSION_READ_SETTINGS_FOR_PROJECT,
        PERMISSION_GRANTS_LOAD_FAILED,
        ROTATION_RULES_LOAD_FAILED,
        ROTATION_BAD_RULE_DTO,
        ROTATION_APPLY_FAILED,
        ROTATION_SNAPSHOT_PARSE_FAILED,
        ARTIFACT_UNKNOWN_ON_CONFLICT,
        ARTIFACT_UNKNOWN_KIND,
        ARTIFACT_INVALID_RELATIVE_PATH,
        ARTIFACT_SCOPE_ROOT_NOT_ACTIVE,
        ARTIFACT_NOT_DISABLED,
        ARTIFACT_PREVIEW_OPEN_FAILED,
        ARTIFACT_PREVIEW_READ_FAILED,
        CONFIG_NODE_NOT_FOUND,
        CONFIG_TREE_NOT_SCANNED,
        CONFIG_PREVIEW_OPEN_FAILED,
        CONFIG_PREVIEW_STAT_FAILED,
        CONFIG_EDITOR_ID_EMPTY,
        CONFIG_UNKNOWN_KIND,
        CONFIG_PATH_EMPTY,
        CONFIG_EDITOR_NOT_DETECTED,
        CONFIG_NO_EDITOR_AVAILABLE,
        CONFIG_SEARCH_ID_EMPTY,
        CONFIG_SEARCH_REGISTRY_LOCK_POISONED,
        CONFIG_NO_ANCHOR_EFFECTIVE_SETTINGS,
        CONFIG_NO_ANCHOR_EFFECTIVE_MCP,
        ENV_FILE_INVALID_PROJECT_PATH,
        ENV_FILE_READ_FAILED,
        ENV_FILE_TOO_LARGE,
        ENV_FILE_INVALID_ENCODING,
        ENV_FILE_INVALID_DOTENV_PATH,
        ENV_FILE_LOCK_OPEN_FAILED,
        ENV_FILE_LOCK_FAILED,
        ENV_FILE_NAME_EMPTY,
        ENV_FILE_NAME_UNSAFE,
        ENV_FILE_NAME_NOT_DOTENV,
        ENV_FILE_READ_DIR_FAILED,
        ENV_FILE_DIR_ENTRY_FAILED,
        ENV_FILE_STAT_ENTRY_FAILED,
        ENV_FILE_WRITE_FAILED,
        ENV_FILE_KEY_MISSING_IN_FILE,
        NOTIFICATION_HOST_ACTIVATE_FAILED,
        NOTIFICATION_LOG_APPEND_FAILED,
        NOTIFICATION_LOG_APPEND_ROUTED_FAILED,
        NOTIFICATION_LOG_MARK_ALL_READ_FAILED,
        NOTIFICATION_LOG_MARK_DELIVERED_FAILED,
        NOTIFICATION_LOG_CLEAR_FAILED,
        BOARD_EXPORT_PATH_NOT_ABSOLUTE,
        BOARD_EXPORT_PATH_EXISTS,
        BOARD_EXPORT_CREATE_FAILED,
        ROUTES_UNKNOWN_PROVIDER_KIND,
        ROUTES_INVALID_ROUTE_ID,
        ROUTES_GATEWAY_CONFIG_MISSING,
        ROUTES_BEDROCK_CONFIG_MISSING,
        ROUTES_VERTEX_CONFIG_MISSING,
        ROUTES_FOUNDRY_CONFIG_MISSING,
        ROUTES_INVALID_GATEWAY_BASE_URL,
        ROUTES_BEDROCK_REGION_REQUIRED,
        ROUTES_BEDROCK_AUTH_REQUIRED,
        ROUTES_INVALID_BEDROCK_BASE_URL,
        ROUTES_VERTEX_PROJECT_ID_REQUIRED,
        ROUTES_INVALID_VERTEX_BASE_URL,
        ROUTES_FOUNDRY_TARGET_REQUIRED,
        ROUTES_FOUNDRY_TARGET_AMBIGUOUS,
        ROUTES_INVALID_FOUNDRY_BASE_URL,
        ROUTES_WRAPPER_NAME_REJECTED,
        ROUTES_GONE_AFTER_PERSIST,
        ROUTES_EDIT_STORE_FAILED,
        ROUTES_SAVE_WARNINGS,
        ROUTES_REMOVE_CLEANUP_FAILED,
        ROUTES_DESKTOP_NOT_SUPPORTED,
        PROJECT_DRY_RUN_SUPERSEDED,
        PROJECT_CLEAN_BLOCKED_BY_JOURNALS,
        PROJECT_REMOVE_BLOCKED_BY_JOURNALS,
        REPAIR_JOURNAL_NOT_FOUND,
        REPAIR_LOCK_FILE_NOT_FOUND,
        SESSION_EXPORT_UNKNOWN_FORMAT,
        SESSION_EXPORT_INVALID_OUTPUT_PATH,
        SESSION_EXPORT_OUTPUT_IS_SYMLINK,
        SESSION_EXPORT_WRITE_FAILED,
        SESSION_EXPORT_PERMISSIONS_NOT_ENFORCED,
        MEMORY_PROJECT_ROOT_EMPTY,
        MEMORY_ENUMERATE_FAILED,
        MEMORY_UNKNOWN_SCOPE,
        MEMORY_AUTO_MEMORY_WRITE_FAILED,
        MEMORY_AUTO_MEMORY_CLEAR_FAILED,
        AGENTS_INVALID_ID,
        AGENTS_NOT_FOUND,
        AGENTS_NAME_TAKEN,
        AGENTS_TEMPLATE_NAME_TAKEN,
        AGENTS_INVALID_PERMISSION_MODE,
        AGENTS_INVALID_OUTPUT_FORMAT,
        AGENTS_BYPASS_REQUIRES_ALLOWED_TOOLS,
        AGENTS_ROUTE_BINARY_ID_REQUIRED,
        AGENTS_UNKNOWN_BINARY_KIND,
        AGENTS_UNKNOWN_EVENT_KIND,
        AGENTS_INVALID_MAX_BUDGET,
        AGENTS_DRAFT_NOT_RUNNABLE,
        AGENTS_NO_SCHEDULER_ADAPTER,
        AGENTS_NO_ARTIFACT_DIR,
        AGENTS_OPEN_ARTIFACT_DIR_FAILED,
        AGENTS_LINGER_LINUX_ONLY,
        TEMPLATES_REGISTRY_LOAD_FAILED,
        TEMPLATES_UNKNOWN_ID,
        TEMPLATES_NO_SAMPLE_REPORT,
        TEMPLATES_REPORT_PATH_UNRESOLVABLE,
        TEMPLATES_REPORT_PATH_OUTSIDE_ROOT,
        TEMPLATES_REPORT_STAT_FAILED,
        TEMPLATES_REPORT_NOT_A_FILE,
        TEMPLATES_REPORT_TOO_LARGE,
        TEMPLATES_REPORT_READ_FAILED,
        TEMPLATES_PLATFORM_UNSUPPORTED,
        TEMPLATES_PLACEHOLDER_TYPE_MISMATCH,
        TEMPLATES_INVALID_SCHEDULE,
        TEMPLATES_INSTANTIATE_FAILED,
        TEMPLATES_UNIQUE_NAME_EXHAUSTED,
        SHARED_MEMORY_INDEX_UNAVAILABLE,
        SHARED_MEMORY_SEARCH_FAILED,
        SHARED_MEMORY_READ_FAILED,
        SHARED_MEMORY_EXCERPT_UNAVAILABLE,
        SHARED_MEMORY_LIST_MEMORIES_FAILED,
        SHARED_MEMORY_INVALID_SCOPE,
        SHARED_MEMORY_INVALID_KIND,
        SHARED_MEMORY_CREATE_MEMORY_FAILED,
        SHARED_MEMORY_ARCHIVE_MEMORY_FAILED,
        SHARED_MEMORY_LIST_DECISIONS_FAILED,
        SHARED_MEMORY_LOG_DECISION_FAILED,
        SHARED_MEMORY_ARCHIVE_DECISION_FAILED,
        SHARED_MEMORY_LIST_EVIDENCE_FAILED,
        SHARED_MEMORY_LINK_TARGET_REQUIRED,
        SHARED_MEMORY_LINKS_FAILED,
        SHARED_MEMORY_LIST_SESSIONS_FAILED,
        SHARED_MEMORY_LIST_PROJECTS_FAILED,
        SHARED_MEMORY_INVALID_INSTALL_SCOPE,
        SHARED_MEMORY_OUT_PATH_NOT_ABSOLUTE,
        SHARED_MEMORY_CURRENT_EXE_FAILED,
        SHARED_MEMORY_CURRENT_EXE_NO_PARENT,
        LESSON_BAD_STATE,
        LESSON_LIST_FAILED,
        LESSON_COUNTS_FAILED,
        LESSON_COUNTS_BY_PROJECT_FAILED,
        LESSON_ACCEPT_FAILED,
        LESSON_REJECT_FAILED,
        RECURRENCE_LIST_FAILED,
        RECURRENCE_CONFIRM_FAILED,
        RECURRENCE_DISMISS_FAILED,
        RECURRENCE_CONFIRMED_COUNT_FAILED,
        RECURRENCE_PENDING_COUNT_FAILED,
        ACCOUNTS_RECONCILE_FAILED,
        APP_KEYCHAIN_UNLOCK_UNSUPPORTED,
        APP_REVEAL_PATH_EMPTY,
        APP_REVEAL_PATH_NOT_FOUND,
        APP_REVEAL_SPAWN_FAILED,
        APP_REVEAL_FAILED,
        APP_LOG_DIR_CREATE_FAILED,
        KEYS_NOT_AN_API_KEY,
        KEYS_NOT_AN_OAUTH_TOKEN,
        UPDATES_STATE_LOCK_POISONED,
        UPDATES_OPERATION_IN_PROGRESS,
        UPDATES_CLI_INSTALL_FAILED,
        UPDATES_DESKTOP_INSTALL_FAILED,
        UPDATES_UNKNOWN_CHANNEL,
        UPDATES_CHANNEL_WRITE_FAILED,
        UPDATES_MINIMUM_VERSION_WRITE_FAILED,
        RELEASE_UPDATE_UNKNOWN_CHANNEL,
        RELEASE_UPDATE_STATE_LOCK_POISONED,
        RELEASE_UPDATE_INVALID_ENDPOINT,
        RELEASE_UPDATE_ENDPOINT_CONFIG_FAILED,
        RELEASE_UPDATE_BUILDER_FAILED,
        RELEASE_UPDATE_CHECK_FAILED,
        RELEASE_UPDATE_NOT_STAGED,
        RELEASE_UPDATE_INSTALL_FAILED,
        ATTRIBUTION_UNKNOWN_MODE,
        AUTO_DREAM_UNKNOWN_MODE,
        MEMORY_HEALTH_AUDIT_FAILED,
        CLI_RESOLVE_TARGET_FAILED,
        MIGRATE_PASSPHRASE_REQUIRED,
        MIGRATE_UNKNOWN_CONFLICT_MODE,
        MIGRATE_UNKNOWN_MERGE_PREFERENCE,
        SESSION_MOVE_INVALID_SESSION_ID,
        SESSION_MOVE_TARGET_CWD_MISSING,
        ACTIVITY_CARDS_INVALID_QUERY,
        ACTIVITY_CARDS_BODY_READ_FAILED,
        ARTIFACT_USAGE_UNKNOWN_KIND,
        USAGE_LOCAL_UNKNOWN_WINDOW_KIND,
        USAGE_LOCAL_WINDOW_DAYS_REQUIRED,
        USAGE_LOCAL_UNKNOWN_PRICING_TIER,
        CC_DOCTOR_CACHE_LOCK_POISONED,
        CONFIG_WATCH_STATE_LOCK_POISONED,
        CONFIG_WATCH_START_FAILED,
    ];
}

/// A rejected command's payload. Serializes as
/// `{ "code": …, "params": { … }, "message": … }` — snake_case like
/// every other DTO in this family.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorDto {
    pub code: String,
    /// Always a JSON object, `{}` when the message interpolates
    /// nothing. The renderer indexes it by name.
    pub params: Value,
    pub message: String,
}

/// Two invariants the renderer depends on were documented and not
/// enforced. Both are cheap to guarantee at construction, and both fail
/// silently and far from their cause if they are violated:
///
/// - **`params` is always a JSON object.** The renderer indexes it by
///   name; a scalar or `null` yields `undefined` lookups and a sentence
///   rendering literal `{{braces}}`.
/// - **`message` is never empty.** It is the English fallback shown
///   whenever a code has no catalog entry, so an empty one renders a
///   *blank* error — the operation failed and the surface said nothing.
///
/// `debug_assert` fires in tests and dev builds so a mistake surfaces
/// during development; release builds degrade to something coherent
/// rather than panicking in front of a user.
fn enforce_object(params: Value, code: &str) -> Value {
    if params.is_object() {
        return params;
    }
    debug_assert!(false, "{code}: params must be a JSON object, got {params}");
    json!({})
}

fn enforce_message(message: String, code: &str) -> String {
    if !message.trim().is_empty() {
        return message;
    }
    debug_assert!(false, "{code}: message must never be empty");
    code.to_string()
}

impl ErrorDto {
    /// A failure whose message interpolates nothing.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            params: json!({}),
            message: enforce_message(message.into(), code),
        }
    }

    /// A failure with named values. `params` must be a JSON object —
    /// pass `json!({ … })`.
    pub fn with_params(code: &'static str, params: Value, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            params: enforce_object(params, code),
            message: enforce_message(message.into(), code),
        }
    }

    /// A wrapped failure whose only structured value is the underlying
    /// text: `params.detail` and `message` are the same string. The
    /// localized sentence appends `{{detail}}` rather than re-deriving
    /// it from `message`.
    pub fn detail(code: &'static str, e: impl std::fmt::Display) -> Self {
        let text = e.to_string();
        Self::with_params(code, json!({ "detail": text }), text)
    }

    /// The `spawn_blocking` join failure every async command shares.
    /// Named because the fan-out repeats it once per command, and one
    /// spelling keeps the catalog entry singular.
    pub fn task_join(e: impl std::fmt::Display) -> Self {
        Self::with_params(
            codes::APP_TASK_JOIN_FAILED,
            json!({ "detail": e.to_string() }),
            format!("blocking task failed: {e}"),
        )
    }

    /// Lift any core error that carries a code. `message` is the
    /// `Display` string verbatim — the contract the renderer's English
    /// fallback depends on.
    pub fn from_core<E: ErrorCode + std::fmt::Display>(e: &E) -> Self {
        Self {
            code: e.code().to_string(),
            params: e.params(),
            message: e.to_string(),
        }
    }
}

// One `From` per converted core enum rather than a blanket impl over
// `ErrorCode`: a blanket `impl<E: ErrorCode> From<E> for ErrorDto`
// collides with core's `impl<T> From<T> for T`. Add a line here when a
// fan-out slice converts another enum.
impl From<KeyError> for ErrorDto {
    fn from(e: KeyError) -> Self {
        Self::from_core(&e)
    }
}

impl From<OAuthError> for ErrorDto {
    fn from(e: OAuthError) -> Self {
        Self::from_core(&e)
    }
}

impl From<ProjectError> for ErrorDto {
    fn from(e: ProjectError) -> Self {
        Self::from_core(&e)
    }
}

// --- projects / sessions / memory / corpus domain ---

impl From<claudepot_core::session::SessionError> for ErrorDto {
    fn from(e: claudepot_core::session::SessionError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::session_export_delivery::DeliverError> for ErrorDto {
    fn from(e: claudepot_core::session_export_delivery::DeliverError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::services::live_activity_service::LiveActivityError> for ErrorDto {
    fn from(e: claudepot_core::services::live_activity_service::LiveActivityError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::session_live::metrics_store::MetricsError> for ErrorDto {
    fn from(e: claudepot_core::session_live::metrics_store::MetricsError) -> Self {
        Self::from_core(&e)
    }
}

// `crate::trash` (one transcript) and `project::trash` (a whole project
// artifact directory) are separate enums with separate codes on
// purpose — same shapes, different objects. Both are lifted.
impl From<claudepot_core::trash::TrashError> for ErrorDto {
    fn from(e: claudepot_core::trash::TrashError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::project_trash::ProjectTrashError> for ErrorDto {
    fn from(e: claudepot_core::project_trash::ProjectTrashError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::session_index::SessionIndexError> for ErrorDto {
    fn from(e: claudepot_core::session_index::SessionIndexError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::session_prune::PruneError> for ErrorDto {
    fn from(e: claudepot_core::session_prune::PruneError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::session_slim::SlimError> for ErrorDto {
    fn from(e: claudepot_core::session_slim::SlimError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::memory_log::MemoryLogError> for ErrorDto {
    fn from(e: claudepot_core::memory_log::MemoryLogError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::memory_view::ReadMemoryError> for ErrorDto {
    fn from(e: claudepot_core::memory_view::ReadMemoryError) -> Self {
        Self::from_core(&e)
    }
}

// --- accounts / auth / desktop.
impl From<claudepot_core::cli_backend::SwapError> for ErrorDto {
    fn from(e: claudepot_core::cli_backend::SwapError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::desktop_backend::DesktopSwapError> for ErrorDto {
    fn from(e: claudepot_core::desktop_backend::DesktopSwapError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::desktop_identity::DesktopIdentityError> for ErrorDto {
    fn from(e: claudepot_core::desktop_identity::DesktopIdentityError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::resolve::ResolveError> for ErrorDto {
    fn from(e: claudepot_core::resolve::ResolveError) -> Self {
        Self::from_core(&e)
    }
}

// `claudepot_core::onboard` is `pub(crate)`; `error::OnboardError` is
// the public re-export.
impl From<claudepot_core::error::OnboardError> for ErrorDto {
    fn from(e: claudepot_core::error::OnboardError) -> Self {
        Self::from_core(&e)
    }
}

// --- config / permissions / rotation / retention ---------------------

impl From<claudepot_core::permission::store::PermissionStoreError> for ErrorDto {
    fn from(e: claudepot_core::permission::store::PermissionStoreError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::permission::settings::PermissionSettingsError> for ErrorDto {
    fn from(e: claudepot_core::permission::settings::PermissionSettingsError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::rotation::store::RotationStoreError> for ErrorDto {
    fn from(e: claudepot_core::rotation::store::RotationStoreError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::rotation::rules::ValidationError> for ErrorDto {
    fn from(e: claudepot_core::rotation::rules::ValidationError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::cc_retention::RetentionError> for ErrorDto {
    fn from(e: claudepot_core::cc_retention::RetentionError) -> Self {
        Self::from_core(&e)
    }
}

// ── agents / boards / env vault / routes / MCP / PRs ───────────────
impl From<claudepot_core::agent::AgentError> for ErrorDto {
    fn from(e: claudepot_core::agent::AgentError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::agent::events::store::AgentEventsError> for ErrorDto {
    fn from(e: claudepot_core::agent::events::store::AgentEventsError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::board::BoardError> for ErrorDto {
    fn from(e: claudepot_core::board::BoardError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::env_vault::VaultError> for ErrorDto {
    fn from(e: claudepot_core::env_vault::VaultError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::env_vault::env_file::EnvEditError> for ErrorDto {
    fn from(e: claudepot_core::env_vault::env_file::EnvEditError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::routes::RouteError> for ErrorDto {
    fn from(e: claudepot_core::routes::RouteError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::routes::WrapperNameError> for ErrorDto {
    fn from(e: claudepot_core::routes::WrapperNameError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::routes::BaseUrlError> for ErrorDto {
    fn from(e: claudepot_core::routes::BaseUrlError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::routes::SaveRouteError> for ErrorDto {
    fn from(e: claudepot_core::routes::SaveRouteError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::mcp_probe::McpProbeError> for ErrorDto {
    fn from(e: claudepot_core::mcp_probe::McpProbeError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::mcp_snippet::InstallError> for ErrorDto {
    fn from(e: claudepot_core::mcp_snippet::InstallError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::github_pr::GhError> for ErrorDto {
    fn from(e: claudepot_core::github_pr::GhError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::protected_paths::ProtectedPathsError> for ErrorDto {
    fn from(e: claudepot_core::protected_paths::ProtectedPathsError) -> Self {
        Self::from_core(&e)
    }
}

// `CcEnvError`'s `value` field is redacted at its one constructor and
// re-scanned in `params()`, so lifting it here cannot widen the leak
// surface beyond what `Display` already prints. See the `ErrorCode`
// impl in `claudepot_core::cc_env::errors`.
impl From<claudepot_core::cc_env::CcEnvError> for ErrorDto {
    fn from(e: claudepot_core::cc_env::CcEnvError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::artifact_lifecycle::LifecycleError> for ErrorDto {
    fn from(e: claudepot_core::artifact_lifecycle::LifecycleError) -> Self {
        Self::from_core(&e)
    }
}

// `classify_path` rejects with a bare `RefuseReason`, and the GUI hides
// the action entirely for each of them — so the reason needs its own
// identity, not `LifecycleError::Refused`'s wrapper.
impl From<claudepot_core::artifact_lifecycle::RefuseReason> for ErrorDto {
    fn from(e: claudepot_core::artifact_lifecycle::RefuseReason) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::config_view::ConfigViewError> for ErrorDto {
    fn from(e: claudepot_core::config_view::ConfigViewError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::config_view::launcher::LaunchError> for ErrorDto {
    fn from(e: claudepot_core::config_view::launcher::LaunchError) -> Self {
        Self::from_core(&e)
    }
}

// ── services orchestration + settings writes ──────────────────────
//
// These seven replaced the last `{{detail}}`-only command codes in this
// module (`accounts.login_failed`, `desktop.switch_failed`, …). Each
// enum draws a distinction a single wrapper code could not carry —
// "retry" vs "sign in again", "nothing was billed" vs "a request went
// out", "already signed out" vs "sign-out failed". `message` is
// unchanged in every case: `from_core` copies `Display` verbatim, which
// is the same string `ErrorDto::detail` used to produce.
impl From<claudepot_core::services::account_service::RegisterError> for ErrorDto {
    fn from(e: claudepot_core::services::account_service::RegisterError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::services::identity::VerifyError> for ErrorDto {
    fn from(e: claudepot_core::services::identity::VerifyError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::services::wake_service::WakeError> for ErrorDto {
    fn from(e: claudepot_core::services::wake_service::WakeError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::services::desktop_service::SwitchError> for ErrorDto {
    fn from(e: claudepot_core::services::desktop_service::SwitchError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::services::desktop_service::AdoptError> for ErrorDto {
    fn from(e: claudepot_core::services::desktop_service::AdoptError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::services::desktop_service::ClearError> for ErrorDto {
    fn from(e: claudepot_core::services::desktop_service::ClearError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::settings_writer::SettingsWriteError> for ErrorDto {
    fn from(e: claudepot_core::settings_writer::SettingsWriteError) -> Self {
        Self::from_core(&e)
    }
}

// ── the long-tail command surfaces ────────────────────────────────
//
// Five enums the small panes lean on. Each already owned an
// `ErrorCode`; what was missing was the lift, so those commands kept
// stringifying (`format!("query: {e}")`) and threw the identity away.
impl From<claudepot_core::cc_tips::TipsError> for ErrorDto {
    fn from(e: claudepot_core::cc_tips::TipsError) -> Self {
        Self::from_core(&e)
    }
}

// Both allowlist enums are lifted: `SetAllowlistError` is what the
// write path returns, and it delegates `code()` to whichever half
// failed — so a normalization rejection and a settings-write failure
// keep their own identities instead of collapsing into a wrapper.
impl From<claudepot_core::available_models::SetAllowlistError> for ErrorDto {
    fn from(e: claudepot_core::available_models::SetAllowlistError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::migrate::MigrateError> for ErrorDto {
    fn from(e: claudepot_core::migrate::MigrateError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::session_move::MoveSessionError> for ErrorDto {
    fn from(e: claudepot_core::session_move::MoveSessionError) -> Self {
        Self::from_core(&e)
    }
}

impl From<claudepot_core::activity::ActivityIndexError> for ErrorDto {
    fn from(e: claudepot_core::activity::ActivityIndexError) -> Self {
        Self::from_core(&e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[derive(serde::Deserialize)]
    struct VectorFile {
        codes: Vec<CodeVector>,
    }

    #[derive(serde::Deserialize)]
    struct CodeVector {
        code: String,
        owner: String,
    }

    fn load() -> Vec<CodeVector> {
        // The vectors live beside the core crate they mostly describe;
        // this crate reads the same registry so the command-layer codes
        // are locked by it too. One file per domain — see the loader in
        // `claudepot_core::error_code` for why it is a directory.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/claudepot-core/testdata/error-codes");
        let mut paths: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        let mut out = Vec::new();
        for path in paths {
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let file: VectorFile =
                serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            out.extend(file.codes);
        }
        out
    }

    #[test]
    fn every_command_code_has_a_vector() {
        let file = load();
        for code in codes::ALL {
            let v = file
                .iter()
                .find(|v| v.code == *code)
                .unwrap_or_else(|| panic!("{code} has no entry in error-code-vectors.json"));
            assert_eq!(
                v.owner, "command",
                "{code} is raised at a command site but registered as {:?}",
                v.owner,
            );
        }
    }

    #[test]
    fn no_command_vector_is_unclaimed() {
        let declared: BTreeSet<&str> = codes::ALL.iter().copied().collect();
        for v in load().iter().filter(|v| v.owner == "command") {
            assert!(
                declared.contains(v.code.as_str()),
                "{} is registered as a command code but no const in dto_error::codes \
                 spells it — a call site using a bare literal is the usual cause",
                v.code,
            );
        }
    }

    #[test]
    fn command_codes_are_unique() {
        let mut seen = BTreeSet::new();
        for code in codes::ALL {
            assert!(seen.insert(*code), "{code} is listed twice in codes::ALL");
        }
    }

    #[test]
    fn constructors_always_produce_an_object_for_params() {
        // The renderer indexes `params` by name; a scalar or null there
        // would be a silent lookup failure rather than a loud one.
        assert!(
            ErrorDto::new(codes::KEYS_LABEL_REQUIRED, "label is required")
                .params
                .is_object()
        );
        assert!(ErrorDto::detail(codes::ACCOUNTS_LIST_FAILED, "boom")
            .params
            .is_object());
        assert!(ErrorDto::task_join("panic").params.is_object());
        assert!(ErrorDto::from(KeyError::NotFound("abc".to_string()))
            .params
            .is_object());
    }

    #[test]
    fn from_core_preserves_the_english_display_string() {
        // The renderer falls back to `message` for any code it cannot
        // translate, so this equality is what keeps a migrated command
        // rendering exactly as it did before.
        let e = KeyError::NotFound("6f9a".to_string());
        let english = e.to_string();
        let dto = ErrorDto::from(e);
        assert_eq!(dto.message, english);
        assert_eq!(dto.code, "keys.not_found");
        assert_eq!(dto.params["uuid"], "6f9a");
    }

    #[test]
    fn oauth_rate_limited_exposes_the_retry_window() {
        let dto = ErrorDto::from(OAuthError::RateLimited {
            retry_after_secs: 42,
        });
        assert_eq!(dto.code, "oauth.rate_limited");
        assert_eq!(dto.params["retry_after_secs"], 42);
        assert_eq!(dto.message, "rate limited — retry after 42s");
    }

    #[test]
    fn project_claude_running_exposes_the_path_not_just_the_cli_flag() {
        // The GUI has no `--force`; it needs the path to name the
        // project and offer a button. The English message keeps the
        // flag because the CLI prints it verbatim.
        let dto = ErrorDto::from(ProjectError::ClaudeRunning("/Users/me/proj".to_string()));
        assert_eq!(dto.code, "project.claude_running");
        assert_eq!(dto.params["path"], "/Users/me/proj");
        assert!(dto.message.contains("--force"), "{}", dto.message);
    }

    #[test]
    fn serializes_with_the_three_fields_the_renderer_expects() {
        let dto = ErrorDto::from(OAuthError::RateLimited {
            retry_after_secs: 7,
        });
        let json = serde_json::to_value(&dto).expect("serializes");
        assert_eq!(json["code"], "oauth.rate_limited");
        assert_eq!(json["params"]["retry_after_secs"], 7);
        assert_eq!(json["message"], "rate limited — retry after 7s");
        // `extractMessage` duck-types on this key alone.
        assert!(json.get("message").is_some());
    }
}

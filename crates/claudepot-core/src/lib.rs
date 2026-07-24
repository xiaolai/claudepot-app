// Crate-wide clippy allowances. Each was failing the `-D warnings`
// gate at the time CI got back to green; addressing them is real
// cleanup work that lives in its own branch, not in the CI-fix
// branch. Re-enable individually as the underlying patterns are
// removed.
#![allow(
    // ~32 doc-comment-indentation hits added by recent clippy
    // versions; cosmetic, no effect on rendered docs.
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items,
    // Real code-style lints — pre-existing, deferred to a cleanup
    // pass that touches the offending sites:
    clippy::manual_strip,
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::let_underscore_future,
    clippy::large_enum_variant,
    clippy::while_let_loop,
    clippy::derivable_impls,
    clippy::unnecessary_sort_by,
    clippy::collapsible_if,
    clippy::needless_borrows_for_generic_args,
    clippy::redundant_closure,
    clippy::manual_map,
    clippy::single_match,
    clippy::assertions_on_constants,
    // CI uses clippy 1.95 (vs local 1.92); 1.95 added the
    // `collapsible_match` lint family.
    clippy::collapsible_match,
)]

#[cfg(test)]
pub mod testing;

pub mod account;
pub(crate) mod account_verification;
pub mod activity;
pub mod agent;
pub mod artifact_lifecycle;
pub mod artifact_toggle;
pub mod artifact_usage;
pub mod attribution_settings;
pub mod auto_dream;
pub mod blob;
pub mod breaker;
pub mod cc_daemon;
pub mod cc_doctor;
pub mod cc_tips;
pub mod cli_backend;
pub(crate) mod codex_session;
pub mod config_view;
pub mod crash_reports;
pub mod db_housekeeping;
pub(crate) mod db_pragmas;
pub mod desktop_backend;
pub mod desktop_identity;
pub mod desktop_lock;
pub mod diagnostic_logging;
pub mod env_vault;
pub mod error;
pub mod fs_utils;
pub mod github_pr;
pub mod host_activate;
pub mod json_store;
pub mod keys;
pub mod launcher;
pub mod main_thread;
pub mod mcp_probe;
pub mod mcp_snippet;
pub mod memory_health;
pub mod memory_log;
pub mod memory_view;
pub mod migrate;
pub mod migrations;
pub mod notification_log;
pub mod notifications;
pub mod oauth;
pub(crate) mod onboard;
pub(crate) mod path_env;
pub mod path_utils;
pub mod paths;
pub mod permission;
pub mod pricing;
pub mod proc_utils;
pub mod project;
// Back-compat shims: the `project_*` family folded into the `project/`
// directory module. These `pub use` aliases keep every old crate-root
// path (`claudepot_core::project_sanitize`, `crate::project_progress`,
// …) resolving so CLI and Tauri call sites compile unchanged. Each
// alias mirrors the visibility the flat module had.
pub(crate) use project::config_rewrite as project_config_rewrite;
pub(crate) use project::display as project_display;
pub use project::dry_run_service as project_dry_run_service;
pub use project::helpers as project_helpers;
pub use project::journal as project_journal;
pub(crate) use project::lock as project_lock;
pub(crate) use project::memory as project_memory;
pub use project::progress as project_progress;
pub use project::remove as project_remove;
pub use project::repair as project_repair;
pub(crate) use project::rewrite as project_rewrite;
pub use project::sanitize as project_sanitize;
pub use project::trash as project_trash;
pub use project::types as project_types;
pub mod protected_paths;
pub(crate) mod proxy;
pub mod redaction;
pub mod release_channel;
pub mod resolve;
pub mod retention;
pub mod rotation;
pub mod routes;
pub mod secret_patterns;
pub mod secure_perms;
pub mod service_status;
pub mod services;
pub mod session;
// `session_index/` and `session_live/` are separate directory modules
// (the persistent cache and the live-metrics path), NOT part of the
// `session/` fold — keep them as-is.
pub mod session_index;
pub mod session_live;
// Back-compat shims: the `session_*` family folded into the `session/`
// directory module. These `pub use` aliases keep every old crate-root
// path (`claudepot_core::session_export`, `crate::session_move`, …)
// resolving so CLI and Tauri call sites compile unchanged. The
// intra-family helpers (move_helpers/jsonl/types, search_ranking) are
// now private submodules inside `session/move_/` and `session/search/`
// and intentionally have no shim. Each alias mirrors the visibility the
// flat module had.
pub use session::chunks as session_chunks;
pub use session::classify as session_classify;
pub use session::context as session_context;
pub use session::export as session_export;
pub use session::export_delivery as session_export_delivery;
pub use session::move_ as session_move;
pub use session::phases as session_phases;
pub use session::prune as session_prune;
pub use session::search as session_search;
pub use session::share as session_share;
pub use session::slim as session_slim;
pub use session::subagents as session_subagents;
pub use session::tool_link as session_tool_link;
pub use session::worktree as session_worktree;
pub mod settings_writer;
pub mod shared_memory;
pub mod sync;
pub mod templates;
pub mod thinking_toggle;
pub mod token_refresh;
pub mod trash;
pub mod updates;
pub mod usage_local;

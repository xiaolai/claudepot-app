//! Routes — third-party LLM backends Claude Code and Claude Desktop
//! can use *alongside* the user's first-party Anthropic identity.
//!
//! A route is `(provider, base URL, auth, models)`. It is **not** an
//! account — there is no Anthropic identity, no email, no OAuth.
//! Routes are additive: they install their own wrapper binary on
//! PATH (`~/.claudepot/bin/<name>`) for the CLI and a profile entry
//! in Claude Desktop's native multi-profile registry
//! (`~/Library/Application Support/Claude-3p/configLibrary/<uuid>.json`).
//! The first-party `claude` CLI and the regular Claude/ Desktop data
//! dir are never touched.
//!
//! Storage: route definitions live in `~/.claudepot/routes.json`,
//! managed by [`RouteStore`]. File-write helpers below the store
//! materialize a route into the CLI wrapper or Desktop profile.
//!
//! Full design: `dev-docs/third-party-llm-design.md`.

mod desktop;
mod error;
mod helper;
mod keychain;
mod slug;
mod store;
mod types;
mod wrapper;

pub use desktop::{
    activate_desktop, clear_desktop_active, enterprise_config_path, library_dir,
    write_library_profile,
};
pub use error::RouteError;
pub use helper::{delete_helpers, helper_path, helpers_dir, write_helper};
pub use keychain::{
    delete_all_for_route as delete_keychain_for_route, delete_secret as delete_keychain_secret,
    read_secret as read_keychain_secret, store_secret as store_keychain_secret,
    SecretField,
};
pub use slug::{derive_wrapper_slug, sanitize_wrapper_name, WrapperNameError};
pub use store::RouteStore;
pub use types::{
    AuthScheme, BedrockConfig, FoundryConfig, GatewayConfig, ProviderKind, Route,
    RouteId, RouteProvider, RouteSummary, VertexConfig,
};
pub use wrapper::{wrapper_dir, wrapper_path, write_wrapper, delete_wrapper};

/// Marker stamped on every Claudepot-written wrapper script and
/// Desktop `configLibrary/<uuid>.json` so future runs can distinguish
/// "managed by us" from "user hand-edited".
pub const CLAUDEPOT_MANAGED_MARKER: &str = "claudepot_managed";

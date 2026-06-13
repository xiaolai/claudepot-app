//! Legacy path shim. Boundary error enums live next to their owning
//! modules per rust-conventions ("one enum per module boundary"):
//!
//! - [`SwapError`] → `cli_backend::error`
//! - [`DesktopSwapError`] → `desktop_backend::error`
//! - [`OAuthError`] → `oauth::error`
//! - [`ProjectError`] → `project`
//! - [`LauncherError`] → `launcher`
//! - [`OnboardError`] → `onboard`
//!
//! These re-exports keep historical `crate::error::X` /
//! `claudepot_core::error::X` import paths compiling. New code should
//! import from the owning module; do NOT add new enums here.

pub use crate::cli_backend::error::SwapError;
pub use crate::desktop_backend::error::DesktopSwapError;
pub use crate::launcher::LauncherError;
pub use crate::oauth::error::OAuthError;
pub use crate::onboard::OnboardError;
pub use crate::project::ProjectError;

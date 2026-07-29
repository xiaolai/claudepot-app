//! Claude Code's `settings.json` `env` block — read, resolve, and write.
//!
//! Not a new domain noun. A tested module over one CC settings key, the same
//! shape as [`crate::cc_retention`], matching the `cc_` prefix that marks
//! "this describes Claude Code, not Claudepot".
//!
//! # The four properties that drive the design
//!
//! **Re-apply is additive-only.** CC re-applies `settings.env` to a running
//! session with `Object.assign` and nothing else — its own comment on
//! `state/onChangeAppState.ts:163` reads *"This is additive-only: new vars are
//! added, existing may be overwritten, nothing is deleted."* So setting or
//! changing a value is usually live, and **clearing one never is**: the old
//! value survives in the running session's environment until relaunch. Every
//! clear/restore confirmation has to say so.
//!
//! **Unset is not `0`.** CC's default for nearly every variable is the key
//! being absent, not a value. Restoring a default therefore *removes the key*;
//! writing the documented default would pin today's number into settings and
//! override whatever CC changes it to later. An explicit empty string is a
//! third state again — distinct from both.
//!
//! **There is a lower env source.** `~/.claude.json` carries its own `env`
//! block, applied *before* settings.json (`utils/managedEnv.ts:136,188`), so
//! settings wins where both set a key — but a variable absent from
//! settings.json may still be **set** by that lower source. A row with no
//! settings entry therefore reads "No settings.json override", never "CC
//! default": we cannot see the user's shell, so "CC default" is a claim we
//! are not entitled to make. See [`state::ResolvedSource`].
//!
//! **The snapshot is not the runtime.** [`spec::EnvSpec::undocumented_in_build`]
//! and every `present_in_build` flag describe one binary, on an exact version
//! match only. See [`spec::CrosscheckValidity`].
//!
//! # Layout
//!
//! - [`spec`] — the embedded, generated spec: control classes and the
//!   orthogonal [`spec::Safety`] attributes.
//! - [`settings`] — key-preserving read-modify-write of the `env` map,
//!   serialized through [`crate::settings_mutex`].
//! - [`state`] — per-variable resolution and the three buckets.
//! - [`errors`] — [`errors::CcEnvError`].

pub mod errors;
pub mod settings;
pub mod spec;
pub mod state;

pub use errors::CcEnvError;
pub use settings::{
    clear_user_env_var, read_env_map, read_legacy_global_env, set_user_env_var, user_settings_path,
    EnvWriteOutcome,
};
pub use spec::{
    Blocked, CrosscheckValidity, EnvCategory, EnvControl, EnvSpec, EnvVarSpec, Hazard, Safety,
};
pub use state::{
    load, resolve_all, resolve_installed_claude, EnvOverview, EnvValue, EnvValueKind, EnvVarState,
    ResolvedSource, UndocumentedBucket, UnrecognizedEntry,
};

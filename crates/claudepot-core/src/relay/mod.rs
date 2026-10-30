//! The relay: model access served to the user's other machines.
//!
//! A relay **server** answers Claude Code running in gateway mode
//! (`ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN` = a relay device token)
//! and forwards each request to an *upstream*: in this phase only the
//! server's subscription, i.e. one of its registered Claude accounts.
//! The plan of record is `docs/notes/relay.md` once P6 lands; until then
//! the design lives in the gitignored `dev-docs/relay-plan.md`.
//!
//! ## Not a new noun
//!
//! The subscription upstream is a *behaviour over `account`*, a sibling
//! of `rotation`: it references an account by id and uses the existing
//! credential store and lock. It never touches the `cli` slot.
//!
//! ## Separation from remote control
//!
//! Remote control's token grants prompt injection — code execution —
//! behind an admin password. A relay token grants inference billed to an
//! account. They share *functions* (`remote::bind::check`,
//! `remote::token` hashing, `remote::ct`), never a listener, a device
//! store, a token namespace or a trust root. A relay token carries the
//! [`devices::TOKEN_PREFIX`] so it cannot be mistaken for a remote one.
//!
//! ## Request path
//!
//! `auth → admission → parse once → grant → buffered body → handler →
//! Outcome`. [`target::parse`] is the only place a request line becomes a
//! [`target::Target`]; the grant check and dispatch both consume that one
//! value, so nothing authorizes one target and dispatches another.

pub mod cancel;
pub mod client;
pub mod client_ops;
pub mod devices;
pub mod forward;
pub mod health;
pub mod identity;
pub mod launch;
pub mod lease;
pub mod ledger;
pub mod listen;
pub mod pairing;
pub mod passthrough;
pub mod refusal;
pub mod revoke_socket;
pub mod route_secret;
pub mod runtime;
pub mod secret;
pub mod select;
pub mod serve_lock;
pub mod server;
pub mod servers;
pub mod service;
pub mod sse;
pub mod store;
pub mod target;
pub mod tls;
pub mod token_store;
pub mod translate;
pub mod upstream;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_health;
#[cfg(test)]
mod tests_translate;
#[cfg(test)]
mod tests_trust;

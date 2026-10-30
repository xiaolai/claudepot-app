//! `claudepot relay …` — both sides of the relay.
//!
//! Client: pair with a relay server, list paired servers, accept a
//! changed target, and run Claude Code through the fastest eligible
//! server. Server: serve, publish an account, bind, and manage devices.
//!
//! Thin over [`claudepot_core::relay::client_ops`] and
//! [`claudepot_core::relay::service`], which own every decision; what stays here is presentation — prompts, the `--json`
//! shapes, and the one stderr line `run` prints before handing the
//! terminal to Claude Code.

mod admin;
mod pair;
mod run;
mod serve;
mod servers;

pub use admin::{
    bind_cmd, devices_cmd, publish_cmd, publish_route_cmd, revoke_cmd, rotate_ca_cmd, status_cmd,
    unpublish_cmd,
};
pub use pair::pair_cmd;
pub use run::run_cmd;
pub use serve::serve_cmd;
pub use servers::{accept_cmd, servers_cmd};

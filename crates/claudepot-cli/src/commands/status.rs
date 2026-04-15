//! `claudepot status` — ground-truth authentication check.
//!
//! Equivalent of running `claude auth status`, but also cross-checks
//! against Claudepot's stored `active_cli` pointer and exits with a
//! status code that scripts can branch on:
//!
//! | Exit | Meaning                                                  |
//! |------|----------------------------------------------------------|
//! | 0    | CC's authenticated email equals Claudepot's active_cli   |
//! | 2    | drift — CC is signed in as a different account          |
//! | 3    | couldn't check (no blob, corrupt blob, /profile failed)  |
//!
//! Use in CI:
//!
//! ```sh
//! claudepot status && echo "safe to proceed"
//! ```

use crate::AppContext;
use anyhow::Result;
use claudepot_core::blob::CredentialBlob;
use claudepot_core::cli_backend;
use claudepot_core::cli_backend::swap::{DefaultProfileFetcher, ProfileFetcher};

pub async fn run(ctx: &AppContext) -> Result<()> {
    let platform = cli_backend::create_platform();
    let blob_str = match platform.read_default().await {
        Ok(Some(s)) => s,
        Ok(None) => {
            emit_no_blob(ctx);
            std::process::exit(3);
        }
        Err(e) => {
            emit_error(ctx, &format!("couldn't read CC credentials: {e}"));
            std::process::exit(3);
        }
    };
    let blob = match CredentialBlob::from_json(&blob_str) {
        Ok(b) => b,
        Err(e) => {
            emit_error(ctx, &format!("CC blob is not valid JSON: {e}"));
            std::process::exit(3);
        }
    };
    let fetcher = DefaultProfileFetcher;
    let cc_email = match fetcher
        .fetch_email(&blob.claude_ai_oauth.access_token)
        .await
    {
        Ok(e) => e,
        Err(e) => {
            emit_error(ctx, &format!("/profile returned: {e}"));
            std::process::exit(3);
        }
    };

    let claudepot_active: Option<String> = ctx
        .store
        .active_cli_uuid()
        .ok()
        .flatten()
        .and_then(|id| uuid::Uuid::parse_str(&id).ok())
        .and_then(|u| ctx.store.find_by_uuid(u).ok().flatten())
        .map(|a| a.email);

    let match_ = claudepot_active
        .as_ref()
        .is_some_and(|e| e.eq_ignore_ascii_case(&cc_email));

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "cc_email": cc_email,
                "claudepot_active_cli": claudepot_active,
                "match": match_,
            })
        );
    } else {
        println!("CC:       {cc_email}");
        match &claudepot_active {
            Some(e) => println!("Active:   {e}"),
            None => println!("Active:   (no active_cli set)"),
        }
        println!(
            "Status:   {}",
            if match_ {
                "MATCH"
            } else if claudepot_active.is_some() {
                "DRIFT — Claudepot's active_cli disagrees with CC"
            } else {
                "UNTRACKED — CC is signed in but Claudepot has no active_cli"
            }
        );
    }

    if match_ {
        Ok(())
    } else {
        std::process::exit(2);
    }
}

fn emit_no_blob(ctx: &AppContext) {
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "cc_email": serde_json::Value::Null,
                "claudepot_active_cli": serde_json::Value::Null,
                "match": false,
                "error": "CC has no stored credentials"
            })
        );
    } else {
        eprintln!("CC has no stored credentials");
    }
}

fn emit_error(ctx: &AppContext, msg: &str) {
    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "cc_email": serde_json::Value::Null,
                "claudepot_active_cli": serde_json::Value::Null,
                "match": false,
                "error": msg,
            })
        );
    } else {
        eprintln!("error: {msg}");
    }
}

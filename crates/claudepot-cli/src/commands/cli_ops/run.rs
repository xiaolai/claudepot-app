//! `run` verb — launch a command with a specific account's token
//! (Mode D), plus the flag/argv classification it dispatches on.
//!
//! Sub-module of `commands/cli_ops.rs`; see that file's header for
//! the per-verb layout rationale.

use super::*;

pub async fn run(
    ctx: &AppContext,
    email_input: &str,
    print_token: bool,
    args: &[String],
) -> Result<()> {
    use claudepot_core::launcher;
    use claudepot_core::resolve::resolve_email;

    let email = resolve_email(&ctx.store, email_input).map_err(|e| anyhow::anyhow!("{e}"))?;

    let account = ctx
        .store
        .find_by_email(&email)?
        .ok_or_else(|| anyhow::anyhow!("account not found: {email}"))?;

    if !account.has_cli_credentials {
        anyhow::bail!("no credentials stored for {email}");
    }

    match classify_run_mode(print_token, args).map_err(|e| anyhow::anyhow!("{e}"))? {
        RunMode::PrintToken => {
            eprintln!("⚠ WARNING: outputting raw access token. Do not log or share this value.");
            let token = launcher::get_access_token(account.uuid)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("{token}");
            Ok(())
        }
        RunMode::Exec => {
            // Drop the internal "Mode D" jargon — users shouldn't need
            // to know the implementation-plan's mode-letter taxonomy to
            // read a progress line. Show the bin name instead.
            let bin = args.first().map(String::as_str).unwrap_or("<cmd>");
            ctx.info(&format!("Running {bin} as {email}..."));
            let exit_code = launcher::run(account.uuid, args)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            std::process::exit(exit_code);
        }
    }
}

/// What `cli run` should do given the flag + argv combination. Pure,
/// testable, no I/O.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunMode {
    /// `--print-token` alone: refresh + print the access token.
    PrintToken,
    /// At least one positional arg: env-inject + exec.
    Exec,
}

/// Mis-combined `cli run` flags. Each variant has a stable `Display`
/// impl that maps 1:1 to a user-visible CLI error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunArgsError {
    /// `--print-token` was passed alongside a command. Previously these
    /// extra args were silently dropped; we now refuse to hide the
    /// mismatch.
    PrintTokenWithArgs,
    /// No `--print-token` and no command — nothing to do.
    NoCommand,
}

impl std::fmt::Display for RunArgsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrintTokenWithArgs => write!(
                f,
                "--print-token does not take a command; remove the extra args \
                 or drop --print-token"
            ),
            Self::NoCommand => write!(
                f,
                "no command specified. Usage: claudepot cli run <email> [--] <cmd...>"
            ),
        }
    }
}
impl std::error::Error for RunArgsError {}

pub(crate) fn classify_run_mode(
    print_token: bool,
    args: &[String],
) -> Result<RunMode, RunArgsError> {
    match (print_token, args.is_empty()) {
        (true, true) => Ok(RunMode::PrintToken),
        (true, false) => Err(RunArgsError::PrintTokenWithArgs),
        (false, true) => Err(RunArgsError::NoCommand),
        (false, false) => Ok(RunMode::Exec),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- classify_run_mode ---------------------------------------

    #[test]
    fn test_run_mode_print_token_alone_is_print_token() {
        assert_eq!(classify_run_mode(true, &[]), Ok(RunMode::PrintToken));
    }

    #[test]
    fn test_run_mode_args_without_print_token_is_exec() {
        let args = vec!["echo".to_string(), "hi".to_string()];
        assert_eq!(classify_run_mode(false, &args), Ok(RunMode::Exec));
    }

    #[test]
    fn test_run_mode_print_token_with_args_errors() {
        // Regression guard: old code silently ignored `echo hi` here.
        // The user might reasonably think the command ran as them, but
        // nothing actually ran. Refuse instead.
        let args = vec!["echo".to_string(), "hi".to_string()];
        assert_eq!(
            classify_run_mode(true, &args),
            Err(RunArgsError::PrintTokenWithArgs)
        );
    }

    #[test]
    fn test_run_mode_no_print_token_no_args_is_no_command() {
        assert_eq!(classify_run_mode(false, &[]), Err(RunArgsError::NoCommand));
    }

    #[test]
    fn test_run_args_error_messages_are_stable() {
        // User-facing error strings — locked down so future edits don't
        // silently reshape script-visible error output.
        assert_eq!(
            RunArgsError::NoCommand.to_string(),
            "no command specified. Usage: claudepot cli run <email> [--] <cmd...>"
        );
        let msg = RunArgsError::PrintTokenWithArgs.to_string();
        assert!(msg.starts_with("--print-token does not take a command"));
    }
}

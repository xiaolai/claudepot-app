use anyhow::Result;
use crate::AppContext;

pub fn status(_ctx: &AppContext) -> Result<()> {
    anyhow::bail!("desktop status not yet implemented (Step 7)")
}

pub async fn use_account(
    _ctx: &AppContext,
    _email_input: &str,
    _no_launch: bool,
) -> Result<()> {
    anyhow::bail!("desktop use not yet implemented (Step 7)")
}

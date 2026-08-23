//! `quick_prompt_*` — the chips above the remote panel's composer.
//!
//! A short name you tap and a longer text that gets sent. All the rules
//! live in `claudepot_core::quick_prompt`; this is the Tauri wrapper.
//!
//! **Save replaces the whole list.** Order is part of the data and
//! every edit the pane offers is "here is the new list", so an
//! add/update/delete/reorder API would be four verbs doing one job with
//! four chances to disagree about the order.
//!
//! `QuickPrompt` crosses to JS directly rather than through a mirrored
//! DTO: it is three owned strings with no path and no secret. Validation
//! failures come back as an `ErrorDto` code so the pane can say which
//! rule was broken rather than "save failed".

use crate::dto_error::ErrorDto;
use claudepot_core::quick_prompt::{self, QuickPrompt};

/// `quick_prompt_list` — the stored list, or the built-in four on a
/// machine that has never had one.
#[tauri::command]
pub async fn quick_prompt_list() -> Result<Vec<QuickPrompt>, ErrorDto> {
    Ok(quick_prompt::load().prompts)
}

/// `quick_prompt_save` — replace the list.
///
/// An empty vector is a legitimate save, not a no-op: it is how the
/// last chip gets deleted, and the store treats "file exists and is
/// empty" as a decision rather than as a machine that has never been
/// configured.
#[tauri::command]
pub async fn quick_prompt_save(prompts: Vec<QuickPrompt>) -> Result<Vec<QuickPrompt>, ErrorDto> {
    quick_prompt::save(prompts)
        .map(|f| f.prompts)
        .map_err(|e| ErrorDto::detail("quick_prompt.save", e))
}

/// `quick_prompt_defaults` — the built-in four, for a "start over"
/// control. Does not write; the pane saves them like any other edit, so
/// restoring is undoable in the same way every other change is.
#[tauri::command]
pub async fn quick_prompt_defaults() -> Result<Vec<QuickPrompt>, ErrorDto> {
    Ok(quick_prompt::defaults())
}

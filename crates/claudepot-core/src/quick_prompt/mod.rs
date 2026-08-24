//! User-authored quick prompts — the chips above the remote panel's
//! composer.
//!
//! A short **name** you tap and a longer **text** that gets sent. The
//! panel shipped four of these hardcoded (`Continue`, `Explain that`,
//! `Run the tests`, `Show me the diff`), which is the right list for
//! nobody in particular: the useful ones are the phrases a given person
//! types twenty times a week, and those are not knowable from here.
//!
//! **First run returns the built-in four, and after that the file
//! wins.** An absent file means "never configured" and gets defaults; a
//! file that exists and is empty means "I deleted them all" and gets
//! nothing. Collapsing those two would make the last delete un-doable —
//! the four would grow back every time.
//!
//! Recovers silently on corruption, unlike `remote-devices.json` or
//! `remote-config.json`. The asymmetry is deliberate and follows the
//! same rule as `remote-read-state.json`: those hold a revocation list
//! and a login throttle, where a silent reset hands something back to an
//! attacker. This is a list of phrases. Losing it costs retyping.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILENAME: &str = "quick-prompts.json";
const STORE: &str = "quick_prompt_store";

/// Room for a chip label, not a sentence. The panel lays these out in a
/// scrolling row, and a name that wraps is a name that has stopped
/// being a button.
pub const MAX_NAME: usize = 32;

/// A prompt is a message, so the cap is generous — but finite, because
/// this file is read on every panel poll and an unbounded field is an
/// unbounded response.
pub const MAX_TEXT: usize = 4000;

/// More than this and the row stops being scannable; the picker exists
/// for the long tail.
pub const MAX_PROMPTS: usize = 24;

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickPrompt {
    /// Stable across renames, so a reorder or an edit is not a delete
    /// plus an insert.
    pub id: String,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickPromptFile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub prompts: Vec<QuickPrompt>,
}

impl Default for QuickPromptFile {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            prompts: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("a quick prompt needs a name")]
    EmptyName,
    #[error("a quick prompt needs some text")]
    EmptyText,
    #[error("name is longer than {MAX_NAME} characters")]
    NameTooLong,
    #[error("text is longer than {MAX_TEXT} characters")]
    TextTooLong,
    #[error("there is already a prompt named that")]
    DuplicateName,
    #[error("that is more than {MAX_PROMPTS} prompts")]
    TooMany,
}

/// Validated on the way to disk as well as on the way in, so a file
/// hand-edited into an invalid state cannot be written by us and is
/// caught on load.
impl crate::json_store::Validate for QuickPromptFile {
    type Error = ValidationError;
    fn validate(&self) -> Result<(), Self::Error> {
        validate(&self.prompts)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialize: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("{0}")]
    Validation(#[from] ValidationError),
}

/// The list a fresh install starts with.
///
/// Exactly what the panel hardcoded, so nobody's chips change the day
/// this lands — they just become editable.
pub fn defaults() -> Vec<QuickPrompt> {
    [
        ("Continue", "Continue."),
        ("Explain that", "Explain what you just did and why."),
        ("Run the tests", "Run the test suite and report what fails."),
        (
            "Show me the diff",
            "Show me the diff of what you have changed so far.",
        ),
    ]
    .into_iter()
    .map(|(name, text)| QuickPrompt {
        // **Stable, not random.** `id` is this list's identity for
        // editing and reordering, and `defaults()` is called on every
        // load of a machine that has never saved the file — so a fresh
        // uuid per call meant the same four prompts arrived under
        // different ids each time. A client that keyed anything on the
        // id saw four deletions and four insertions on every poll, and
        // "restore defaults" could never be a no-op.
        //
        // Derived from the name so the value is readable in the file
        // and cannot silently collide.
        id: format!("builtin-{}", slug(name)),
        name: name.to_string(),
        text: text.to_string(),
    })
    .collect()
}

/// Lowercase, non-alphanumerics to `-`. Only ever applied to the four
/// built-in names above, which are ASCII.
fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

pub fn path() -> PathBuf {
    crate::paths::claudepot_data_dir().join(FILENAME)
}

/// Read the list, seeding the built-ins only when the file has never
/// existed.
pub fn load_at(path: &Path) -> QuickPromptFile {
    // `try_exists` distinguishes "no file" from "cannot stat it".
    // `exists()` reports both as false, so a permission error on the
    // data directory presented as a first run and quietly re-seeded the
    // built-ins over a list the user had edited.
    //
    // An error here is treated as "the file is there but unreadable",
    // which routes into `json_store_load`'s recovery rather than into
    // the first-run seed — losing the prompts is bad, silently
    // resurrecting deleted ones is worse, because the file's whole
    // contract is that empty and absent are different states.
    if matches!(path.try_exists(), Ok(false)) {
        return QuickPromptFile {
            schema_version: default_schema_version(),
            prompts: defaults(),
        };
    }
    json_store_load(path)
}

fn json_store_load(path: &Path) -> QuickPromptFile {
    crate::json_store::load_or_default::<QuickPromptFile>(path, STORE)
}

pub fn load() -> QuickPromptFile {
    load_at(&path())
}

/// Check a whole list before it replaces the stored one.
///
/// Validated as a SET rather than per item, because the two rules that
/// actually bite — a duplicate name and too many — are properties of
/// the list. A per-item check would pass every item and still produce a
/// row with two chips reading "Continue".
pub fn validate(prompts: &[QuickPrompt]) -> Result<(), ValidationError> {
    if prompts.len() > MAX_PROMPTS {
        return Err(ValidationError::TooMany);
    }
    let mut seen = std::collections::BTreeSet::new();
    for p in prompts {
        let name = p.name.trim();
        if name.is_empty() {
            return Err(ValidationError::EmptyName);
        }
        if p.text.trim().is_empty() {
            return Err(ValidationError::EmptyText);
        }
        if name.chars().count() > MAX_NAME {
            return Err(ValidationError::NameTooLong);
        }
        if p.text.chars().count() > MAX_TEXT {
            return Err(ValidationError::TextTooLong);
        }
        // Case-insensitively: two chips differing only in capitals are
        // two chips the reader cannot tell apart.
        if !seen.insert(name.to_lowercase()) {
            return Err(ValidationError::DuplicateName);
        }
    }
    Ok(())
}

/// Replace the list wholesale.
///
/// One write for the whole set rather than add/edit/delete verbs: the
/// order is part of the data, every mutation the UI offers is "here is
/// the new list", and a partial API would need a reorder verb that does
/// the same thing anyway.
pub fn save_at(path: &Path, prompts: Vec<QuickPrompt>) -> Result<QuickPromptFile, StoreError> {
    validate(&prompts)?;
    let file = QuickPromptFile {
        schema_version: default_schema_version(),
        prompts: prompts
            .into_iter()
            .map(|p| QuickPrompt {
                id: p.id,
                name: p.name.trim().to_string(),
                text: p.text.trim().to_string(),
            })
            .collect(),
    };
    crate::json_store::save(path, &file).map_err(|e| match e {
        crate::json_store::SaveError::Io(e) => StoreError::Io(e),
        crate::json_store::SaveError::Serde(e) => StoreError::Serde(e),
        crate::json_store::SaveError::Validation(e) => StoreError::Validation(e),
    })?;
    Ok(file)
}

pub fn save(prompts: Vec<QuickPrompt>) -> Result<QuickPromptFile, StoreError> {
    save_at(&path(), prompts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, text: &str) -> QuickPrompt {
        QuickPrompt {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            text: text.into(),
        }
    }

    #[test]
    fn a_fresh_install_gets_the_built_in_four() {
        let d = tempfile::tempdir().unwrap();
        let f = load_at(&d.path().join(FILENAME));
        assert_eq!(f.prompts.len(), 4);
        assert_eq!(f.prompts[0].name, "Continue");
    }

    #[test]
    fn deleting_them_all_sticks() {
        // The whole reason "absent" and "empty" are different states: if
        // an empty file re-seeded, the last delete would undo itself and
        // the four would grow back forever.
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join(FILENAME);
        save_at(&path, Vec::new()).unwrap();
        assert!(
            load_at(&path).prompts.is_empty(),
            "an empty list must stay empty"
        );
    }

    #[test]
    fn a_saved_list_round_trips_in_order() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join(FILENAME);
        let want = vec![
            p("Ship it", "Commit and push."),
            p("Undo", "Revert the last change."),
        ];
        save_at(&path, want.clone()).unwrap();
        let got = load_at(&path);
        assert_eq!(got.prompts, want, "order is data, not presentation");
    }

    #[test]
    fn names_and_text_are_trimmed_on_the_way_in() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join(FILENAME);
        let f = save_at(&path, vec![p("  Ship it  ", "  do it  ")]).unwrap();
        assert_eq!(f.prompts[0].name, "Ship it");
        assert_eq!(f.prompts[0].text, "do it");
    }

    #[test]
    fn an_empty_name_or_text_is_refused() {
        assert_eq!(validate(&[p("", "x")]), Err(ValidationError::EmptyName));
        assert_eq!(validate(&[p("  ", "x")]), Err(ValidationError::EmptyName));
        assert_eq!(validate(&[p("x", "")]), Err(ValidationError::EmptyText));
        assert_eq!(validate(&[p("x", "   ")]), Err(ValidationError::EmptyText));
    }

    #[test]
    fn two_chips_the_reader_cannot_tell_apart_are_refused() {
        // Case-insensitively: "Continue" and "continue" are one label.
        assert_eq!(
            validate(&[p("Continue", "a"), p("continue", "b")]),
            Err(ValidationError::DuplicateName)
        );
    }

    #[test]
    fn the_caps_hold() {
        let long_name = "n".repeat(MAX_NAME + 1);
        assert_eq!(
            validate(&[p(&long_name, "x")]),
            Err(ValidationError::NameTooLong)
        );
        let long_text = "t".repeat(MAX_TEXT + 1);
        assert_eq!(
            validate(&[p("x", &long_text)]),
            Err(ValidationError::TextTooLong)
        );
        let many: Vec<_> = (0..=MAX_PROMPTS)
            .map(|i| p(&format!("n{i}"), "x"))
            .collect();
        assert_eq!(validate(&many), Err(ValidationError::TooMany));
    }

    #[test]
    fn an_invalid_list_never_reaches_disk() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join(FILENAME);
        save_at(&path, vec![p("ok", "fine")]).unwrap();
        let err = save_at(&path, vec![p("", "x")]);
        assert!(matches!(
            err,
            Err(StoreError::Validation(ValidationError::EmptyName))
        ));
        // and the good list survives the rejected write
        assert_eq!(load_at(&path).prompts[0].name, "ok");
    }

    #[test]
    fn the_defaults_themselves_validate() {
        // They ship as the first thing a user sees; a default list that
        // its own rules reject would be a bad first impression.
        validate(&defaults()).unwrap();
    }

    #[test]
    fn a_corrupt_file_recovers_rather_than_failing() {
        // A list of phrases, not a revocation list — losing it costs
        // retyping, so it must never be fatal at boot.
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join(FILENAME);
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(load_at(&path).prompts.is_empty());
    }

    #[test]
    fn the_built_in_ids_are_stable_across_calls() {
        // `id` is the list's identity for editing and reordering, and
        // `defaults()` runs on every load of a never-configured machine.
        // Fresh uuids made two identical loads look like four deletions
        // and four insertions.
        let a: Vec<_> = defaults().into_iter().map(|p| p.id).collect();
        let b: Vec<_> = defaults().into_iter().map(|p| p.id).collect();
        assert_eq!(a, b);
        assert_eq!(
            a,
            vec![
                "builtin-continue",
                "builtin-explain-that",
                "builtin-run-the-tests",
                "builtin-show-me-the-diff",
            ]
        );
    }

    #[test]
    fn the_built_in_ids_are_distinct_and_valid() {
        let ids: std::collections::HashSet<_> = defaults().into_iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 4, "a collision would shadow a prompt");
        assert!(ids.iter().all(|i| !i.is_empty()));
        // And the list still validates as a whole.
        assert!(validate(&defaults()).is_ok());
    }

    #[test]
    fn an_absent_file_seeds_but_an_empty_one_does_not() {
        // The documented distinction: never configured yields the four
        // built-ins, "I deleted them all" yields nothing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quick-prompts.json");
        assert_eq!(load_at(&path).prompts.len(), 4);

        std::fs::write(
            &path,
            serde_json::to_vec(&QuickPromptFile {
                schema_version: default_schema_version(),
                prompts: vec![],
            })
            .unwrap(),
        )
        .unwrap();
        assert!(
            load_at(&path).prompts.is_empty(),
            "an empty file must stay empty — the last delete must not undo itself"
        );
    }
}

//! Repair journal DTOs — surface for the Repair view.

use serde::Serialize;

#[derive(Serialize)]
pub struct JournalFlagsDto {
    pub merge: bool,
    pub overwrite: bool,
    pub force: bool,
    pub no_move: bool,
}

impl From<&claudepot_core::project_journal::JournalFlags> for JournalFlagsDto {
    fn from(f: &claudepot_core::project_journal::JournalFlags) -> Self {
        Self {
            merge: f.merge,
            overwrite: f.overwrite,
            force: f.force,
            no_move: f.no_move,
        }
    }
}

#[derive(Serialize)]
pub struct JournalEntryDto {
    pub id: String,
    pub path: String,
    pub status: String,
    pub old_path: String,
    pub new_path: String,
    pub started_at: String,
    pub started_unix_secs: u64,
    pub phases_completed: Vec<String>,
    pub snapshot_paths: Vec<String>,
    pub last_error: Option<String>,
    pub flags: JournalFlagsDto,
}

impl From<&claudepot_core::project_repair::JournalEntry> for JournalEntryDto {
    fn from(e: &claudepot_core::project_repair::JournalEntry) -> Self {
        Self {
            id: e.id.clone(),
            path: e.path.to_string_lossy().to_string(),
            status: e.status.tag().to_string(),
            old_path: e.journal.old_path.clone(),
            new_path: e.journal.new_path.clone(),
            started_at: e.journal.started_at.clone(),
            started_unix_secs: e.journal.started_unix_secs,
            phases_completed: e.journal.phases_completed.clone(),
            snapshot_paths: e
                .journal
                .snapshot_paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            last_error: e.journal.last_error.clone(),
            flags: JournalFlagsDto::from(&e.journal.flags),
        }
    }
}

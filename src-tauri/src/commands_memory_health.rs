//! Tauri surface for the memory-health card. Pure pass-through —
//! the metrics live in `claudepot_core::memory_health`. This module
//! exists only to mirror the core types into Tauri DTOs (so future
//! GUI-only fields don't leak back into core) and register the
//! command.

use claudepot_core::memory_health::{self, FileHealth, MemoryHealthReport};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct FileHealthDto {
    pub path: String,
    pub missing: bool,
    pub line_count: usize,
    pub char_count: usize,
    pub lines_past_cutoff: usize,
    pub chars_past_cutoff: usize,
    pub est_tokens: usize,
}

impl From<FileHealth> for FileHealthDto {
    fn from(h: FileHealth) -> Self {
        Self {
            path: h.path,
            missing: h.missing,
            line_count: h.line_count,
            char_count: h.char_count,
            lines_past_cutoff: h.lines_past_cutoff,
            chars_past_cutoff: h.chars_past_cutoff,
            est_tokens: h.est_tokens,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MemoryHealthReportDto {
    pub claude_md: FileHealthDto,
    pub memory_md: FileHealthDto,
    pub line_cutoff: usize,
}

impl From<MemoryHealthReport> for MemoryHealthReportDto {
    fn from(r: MemoryHealthReport) -> Self {
        Self {
            claude_md: r.claude_md.into(),
            memory_md: r.memory_md.into(),
            line_cutoff: r.line_cutoff,
        }
    }
}

/// Audit `~/.claude/CLAUDE.md` and `~/.claude/memory/MEMORY.md`,
/// returning per-file health metrics. Missing files are reported
/// inline (`missing: true`); only non-NotFound I/O errors surface
/// here, in which case the GUI shows a "couldn't audit" tile rather
/// than zeros.
#[tauri::command]
pub async fn memory_health_get() -> Result<MemoryHealthReportDto, String> {
    let report = memory_health::build_report().map_err(|e| format!("memory_health: {e}"))?;
    Ok(report.into())
}

//! Diagnostic log retention.
//!
//! `tracing_appender::rolling::daily` rolls the active file every
//! midnight UTC but never deletes the rolled-off siblings. Without
//! a sweep, `~/Library/Logs/com.claudepot.app/` accumulates
//! `claudepot.log.YYYY-MM-DD` files indefinitely. This module
//! prunes anything older than `RETENTION_DAYS` at startup.
//!
//! Best-effort: any I/O error on a single file is swallowed so the
//! pass never blocks boot. The active `claudepot.log` (no suffix)
//! is always kept.

use std::path::Path;

const RETENTION_DAYS: u64 = 7;
const SECONDS_PER_DAY: u64 = 86_400;

/// Delete rolled log files older than 7 days in `log_dir`.
/// Returns the number of files pruned. Returns 0 if the directory
/// doesn't exist (first-boot case) or can't be read.
pub fn prune_old_logs(log_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return 0;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(
            RETENTION_DAYS * SECONDS_PER_DAY,
        ))
        .unwrap_or(std::time::UNIX_EPOCH);
    let mut pruned = 0usize;
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // Match the rolled siblings only. The live file is
        // `claudepot.log` (no suffix) and stays no matter how old.
        if !name.starts_with("claudepot.log.") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff && std::fs::remove_file(entry.path()).is_ok() {
            pruned += 1;
        }
    }
    pruned
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, mtime: SystemTime) {
        std::fs::write(path, b"").unwrap();
        let mtime = filetime::FileTime::from_system_time(mtime);
        filetime::set_file_mtime(path, mtime).unwrap();
    }

    #[test]
    fn prunes_old_rolled_files_keeps_recent_and_live() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        let old = dir.join("claudepot.log.2026-04-01");
        let recent = dir.join("claudepot.log.2026-06-04");
        let live = dir.join("claudepot.log");
        let unrelated = dir.join("notes.txt");

        let now = SystemTime::now();
        let old_mtime = now - Duration::from_secs(30 * SECONDS_PER_DAY);
        let recent_mtime = now - Duration::from_secs(2 * SECONDS_PER_DAY);

        touch(&old, old_mtime);
        touch(&recent, recent_mtime);
        touch(&live, now);
        touch(&unrelated, old_mtime);

        let pruned = prune_old_logs(dir);
        assert_eq!(pruned, 1);
        assert!(!old.exists());
        assert!(recent.exists());
        assert!(live.exists(), "the active log file must never be pruned");
        assert!(unrelated.exists(), "non-log files must not be touched");
    }

    #[test]
    fn missing_directory_returns_zero() {
        let pruned = prune_old_logs(Path::new("/nonexistent/claudepot-log-dir"));
        assert_eq!(pruned, 0);
    }
}

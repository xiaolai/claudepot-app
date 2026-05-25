//! DTO for the `cc_daemon` status scrape.
//!
//! Mirrors [`claudepot_core::cc_daemon::DaemonStatus`] with camelCase
//! serde tags. No adapter on the renderer side. Parallel to
//! [`crate::dto_cc_doctor`].

use claudepot_core::cc_daemon::{DaemonParseStatus, DaemonStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonParseStatusDto {
    Ok,
    Degraded { reason: String },
    Failed { reason: String },
}

impl From<DaemonParseStatus> for DaemonParseStatusDto {
    fn from(s: DaemonParseStatus) -> Self {
        match s {
            DaemonParseStatus::Ok => Self::Ok,
            DaemonParseStatus::Degraded { reason } => Self::Degraded { reason },
            DaemonParseStatus::Failed { reason } => Self::Failed { reason },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatusDto {
    pub running: bool,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub bg_workers: Option<u32>,
    pub sock_dir: Option<String>,
    pub control_sock: Option<String>,
    pub roster_path: Option<String>,
    pub log_path: Option<String>,
    pub parse_status: DaemonParseStatusDto,
}

impl From<DaemonStatus> for DaemonStatusDto {
    fn from(s: DaemonStatus) -> Self {
        Self {
            running: s.running,
            pid: s.pid,
            uptime_secs: s.uptime_secs,
            bg_workers: s.bg_workers,
            sock_dir: s.sock_dir.map(|p| p.display().to_string()),
            control_sock: s.control_sock.map(|p| p.display().to_string()),
            roster_path: s.roster_path.map(|p| p.display().to_string()),
            log_path: s.log_path.map(|p| p.display().to_string()),
            parse_status: s.parse_status.into(),
        }
    }
}

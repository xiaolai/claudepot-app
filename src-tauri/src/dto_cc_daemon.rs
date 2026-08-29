//! DTO for the `cc_daemon` status read.
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

/// Six fields used to ride along here: `uptime_secs`, `sock_dir`,
/// `control_sock` and `log_path` were obtainable only from the
/// `claude daemon status` subprocess that issue #94 deleted, and no
/// renderer read any of them. `running` and `pid` went with them for a
/// sharper reason — CC's roster carries no `procStart` for the
/// supervisor, so neither could be guarded against PID reuse, and an
/// unguardable boolean sitting beside a guarded count is a trap for
/// whoever reaches for it next. See [`claudepot_core::cc_daemon`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatusDto {
    pub bg_workers: Option<u32>,
    pub roster_path: Option<String>,
    pub parse_status: DaemonParseStatusDto,
}

impl From<DaemonStatus> for DaemonStatusDto {
    fn from(s: DaemonStatus) -> Self {
        Self {
            bg_workers: s.bg_workers,
            roster_path: s.roster_path.map(|p| p.display().to_string()),
            parse_status: s.parse_status.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::cc_daemon::{DaemonParseStatus, DaemonStatus};
    use std::path::PathBuf;

    /// The renderer reads these three keys by name and nothing
    /// typechecks Rust against TypeScript, so a rename here becomes
    /// `undefined` at runtime with every other gate green. This test
    /// is the only thing holding the wire names.
    #[test]
    fn the_wire_shape_is_exactly_what_the_renderer_reads() {
        let dto: DaemonStatusDto = DaemonStatus {
            bg_workers: Some(3),
            roster_path: Some(PathBuf::from("/home/u/.claude/daemon/roster.json")),
            parse_status: DaemonParseStatus::Ok,
        }
        .into();
        let v = serde_json::to_value(&dto).unwrap();

        assert_eq!(v["bgWorkers"], 3);
        assert_eq!(v["rosterPath"], "/home/u/.claude/daemon/roster.json");
        assert_eq!(v["parseStatus"]["kind"], "ok");

        // The six fields deleted with the `claude daemon status`
        // subprocess must stay gone. A well-meaning re-add would ship
        // permanent nulls across IPC.
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 3, "unexpected keys: {:?}", obj.keys());
        for gone in [
            "running",
            "pid",
            "uptimeSecs",
            "sockDir",
            "controlSock",
            "logPath",
        ] {
            assert!(obj.get(gone).is_none(), "{gone} should not be on the wire");
        }
    }

    /// `null` is "couldn't tell" and must survive the crossing as
    /// `null`, not as `0` — the renderer treats the two differently.
    #[test]
    fn an_unknown_count_crosses_as_null_not_zero() {
        let dto: DaemonStatusDto = DaemonStatus {
            bg_workers: None,
            roster_path: None,
            parse_status: DaemonParseStatus::Degraded {
                reason: "roster.json declares proto 2".into(),
            },
        }
        .into();
        let v = serde_json::to_value(&dto).unwrap();
        assert!(v["bgWorkers"].is_null());
        assert_eq!(v["parseStatus"]["kind"], "degraded");
        assert_eq!(v["parseStatus"]["reason"], "roster.json declares proto 2");
    }
}

//! No-op scheduler: returns "unsupported platform" for state-
//! mutating ops, but provides accurate `next_runs` math + truthful
//! capabilities. Used in CI, on unsupported targets, and as the
//! fallback before each per-OS adapter ships.

use chrono::{DateTime, Utc};

use crate::agent::error::AgentError;
use crate::agent::types::{Agent, AgentId, Trigger};

use super::{cron_next_runs, RegisteredEntry, Scheduler, SchedulerCapabilities};

pub struct NoopScheduler;

impl Scheduler for NoopScheduler {
    fn register(&self, agent: &Agent) -> Result<(), AgentError> {
        // Manual + Event triggers don't need a scheduler. The Noop
        // adapter can honor them even on unsupported hosts; Run-Now
        // and the in-app event orchestrator don't depend on
        // platform-specific scheduling.
        if agent.trigger.has_no_os_schedule() {
            return Ok(());
        }
        Err(AgentError::UnsupportedPlatform(
            "no scheduler adapter is wired for this host yet",
        ))
    }

    fn unregister(&self, _id: &AgentId) -> Result<(), AgentError> {
        // Always succeed — "nothing to remove" is the right answer.
        Ok(())
    }

    fn kickstart(&self, _id: &AgentId) -> Result<(), AgentError> {
        Err(AgentError::UnsupportedPlatform(
            "kickstart is unavailable without a scheduler adapter",
        ))
    }

    fn list_managed(&self) -> Result<Vec<RegisteredEntry>, AgentError> {
        Ok(Vec::new())
    }

    fn expected_identifier(&self, id: &AgentId) -> String {
        // The Noop adapter registers nothing; a stable synthetic
        // identifier keeps the trait total without implying an
        // artifact exists.
        format!("noop.agent.{id}")
    }

    fn next_runs(
        &self,
        trigger: &Trigger,
        from: DateTime<Utc>,
        n: usize,
    ) -> Result<Vec<DateTime<Utc>>, AgentError> {
        match trigger {
            Trigger::Cron { cron, timezone: _ } => cron_next_runs(cron, from, n),
            // Manual + Event carry no schedule — no upcoming OS runs.
            Trigger::Manual | Trigger::Event { .. } => Ok(Vec::new()),
        }
    }

    fn capabilities(&self) -> SchedulerCapabilities {
        SchedulerCapabilities {
            wake_to_run: false,
            catch_up_if_missed: false,
            run_when_logged_out: false,
            native_label: "none",
            artifact_dir: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use uuid::Uuid;

    #[test]
    fn unregister_is_idempotent_success() {
        let s = NoopScheduler;
        s.unregister(&Uuid::new_v4()).unwrap();
    }

    #[test]
    fn list_managed_empty() {
        let s = NoopScheduler;
        assert!(s.list_managed().unwrap().is_empty());
    }

    #[test]
    fn capabilities_are_all_off() {
        let s = NoopScheduler;
        let caps = s.capabilities();
        assert!(!caps.wake_to_run);
        assert!(!caps.catch_up_if_missed);
        assert!(!caps.run_when_logged_out);
        assert_eq!(caps.native_label, "none");
        assert!(caps.artifact_dir.is_none());
    }

    #[test]
    fn next_runs_works_through_trait() {
        let s = NoopScheduler;
        let trigger = Trigger::Cron {
            cron: "0 9 * * *".into(),
            timezone: None,
        };
        let from = Utc.with_ymd_and_hms(2026, 4, 28, 8, 0, 0).unwrap();
        let next = s.next_runs(&trigger, from, 2).unwrap();
        assert_eq!(next.len(), 2);
    }
}

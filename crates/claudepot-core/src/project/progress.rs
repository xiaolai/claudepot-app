//! Progress reporting surface for long-running project operations.
//!
//! Replaces the prior `Fn(usize, usize)` callback shim (spec §8 Q1
//! option b) so the Tauri layer can emit structured phase events on
//! per-operation channels without parsing numeric arguments.

/// Phase lifecycle state emitted to a [`ProgressSink`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseStatus {
    /// Phase has started but not finished. Currently unused by the
    /// core — phases emit `Complete` once `mark_phase` succeeds — but
    /// the variant is kept for future work.
    Running,
    /// Phase finished successfully.
    Complete,
    /// Phase failed. The payload is the human error message already
    /// written to the journal's `last_error` field.
    Error(String),
}

/// Structured progress sink. Implementations are expected to be cheap
/// per call — `sub_progress` is invoked per file during P6 and can
/// fire hundreds of times.
pub trait ProgressSink: Send + Sync {
    /// Called at a phase boundary.
    fn phase(&self, phase: &str, status: PhaseStatus);

    /// Called during a phase to report within-phase progress
    /// (currently only P6 file rewriting).
    fn sub_progress(&self, phase: &str, done: usize, total: usize);
}

/// No-op sink for callers that don't care about progress.
/// Passed as `&NoopSink` (zero-sized; no allocation).
pub struct NoopSink;

impl ProgressSink for NoopSink {
    fn phase(&self, _phase: &str, _status: PhaseStatus) {}
    fn sub_progress(&self, _phase: &str, _done: usize, _total: usize) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSink {
        phases: Mutex<Vec<(String, PhaseStatus)>>,
        subs: Mutex<Vec<(String, usize, usize)>>,
    }

    impl ProgressSink for RecordingSink {
        fn phase(&self, phase: &str, status: PhaseStatus) {
            self.phases
                .lock()
                .unwrap()
                .push((phase.to_string(), status));
        }
        fn sub_progress(&self, phase: &str, done: usize, total: usize) {
            self.subs
                .lock()
                .unwrap()
                .push((phase.to_string(), done, total));
        }
    }

    #[test]
    fn test_noop_sink_is_silent() {
        let sink = NoopSink;
        sink.phase("P3", PhaseStatus::Complete);
        sink.sub_progress("P6", 1, 10);
        // Nothing to assert — the point is it compiles and doesn't panic.
    }

    #[test]
    fn test_recording_sink_captures_events() {
        let sink = RecordingSink::default();
        sink.phase("P3", PhaseStatus::Complete);
        sink.phase("P6", PhaseStatus::Error("boom".to_string()));
        sink.sub_progress("P6", 5, 10);

        let phases = sink.phases.lock().unwrap();
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].0, "P3");
        assert_eq!(phases[0].1, PhaseStatus::Complete);
        assert_eq!(phases[1].1, PhaseStatus::Error("boom".to_string()));

        let subs = sink.subs.lock().unwrap();
        assert_eq!(subs[0], ("P6".to_string(), 5, 10));
    }
}

//! Says so in the log when the main thread stops servicing its event loop.
//!
//! The main thread runs the window, the menu-bar icon, and the handler for
//! every IPC request. On 2026-09-22 the app stayed up for 33 hours with its
//! background index loop logging normally while the window and the tray
//! icon did nothing, and it was restarted without anyone being able to say
//! what the main thread was doing, because nothing had recorded it. This
//! module is the record.
//!
//! A dedicated OS thread (not a tokio task: the async runtime being wedged
//! is one of the things this has to survive) posts a no-op to the main
//! thread once a second and notes whether the previous one ran. A ping that
//! goes unanswered for [`STALL_AFTER`] logs a warning; the answer that ends
//! the stall logs its length. Only one ping is ever outstanding, so a main
//! thread stuck for hours does not come back to a queue of thousands.
//!
//! On macOS a stall that reaches [`SAMPLE_AFTER`] also runs `/usr/bin/sample`
//! against this process and leaves the stacks of every thread beside the
//! log — the evidence that settles what the main thread was blocked on. At
//! most one sample per [`SAMPLE_MIN_GAP`], and only the newest
//! [`SAMPLES_KEPT`] files are kept.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::AppHandle;

const PING_EVERY: Duration = Duration::from_secs(1);
/// Long enough that a heavy but legitimate main-thread moment (a large IPC
/// payload, a window resize) never reads as a hang.
const STALL_AFTER: Duration = Duration::from_secs(5);
#[cfg(target_os = "macos")]
const SAMPLE_AFTER: Duration = Duration::from_secs(15);
#[cfg(target_os = "macos")]
const SAMPLE_MIN_GAP: Duration = Duration::from_secs(60 * 60);
#[cfg(target_os = "macos")]
const SAMPLES_KEPT: usize = 5;
#[cfg(target_os = "macos")]
const SAMPLE_PREFIX: &str = "main-thread-stall-";

pub fn spawn(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("main-thread-watchdog".into())
        .spawn(move || run(app));
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "main-thread watchdog: could not start");
    }
}

fn run(app: AppHandle) {
    let outstanding = Arc::new(AtomicBool::new(false));
    let mut sent_at = Instant::now();
    let mut stall = StallTracker::default();
    #[cfg(target_os = "macos")]
    let mut last_sample: Option<Instant> = None;

    loop {
        if !outstanding.load(Ordering::Acquire) {
            outstanding.store(true, Ordering::Release);
            sent_at = Instant::now();
            let answered = Arc::clone(&outstanding);
            if app
                .run_on_main_thread(move || answered.store(false, Ordering::Release))
                .is_err()
            {
                // The event loop is gone: the app is exiting.
                return;
            }
        }
        std::thread::sleep(PING_EVERY);

        let unanswered_for = outstanding
            .load(Ordering::Acquire)
            .then(|| sent_at.elapsed());
        match stall.observe(unanswered_for) {
            Some(Transition::Started(waited)) => tracing::warn!(
                waited_s = waited.as_secs(),
                "main thread is not servicing its event loop — the window and the \
                 menu-bar icon are unresponsive until it does"
            ),
            Some(Transition::Ended(lasted)) => tracing::warn!(
                lasted_s = lasted.as_secs(),
                "main thread is servicing its event loop again"
            ),
            None => {}
        }

        #[cfg(target_os = "macos")]
        if let Some(waited) = unanswered_for {
            let due = last_sample.is_none_or(|t| t.elapsed() >= SAMPLE_MIN_GAP);
            if waited >= SAMPLE_AFTER && due {
                last_sample = Some(Instant::now());
                capture_sample();
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transition {
    /// The outstanding ping has gone unanswered this long.
    Started(Duration),
    /// The stall that began with an unanswered ping has ended; it lasted
    /// at least this long (the answer landed within one tick of it).
    Ended(Duration),
}

/// The pure half: turns "how long has the current ping been unanswered"
/// into stall start / end transitions, each reported exactly once.
#[derive(Debug, Default)]
struct StallTracker {
    /// Longest unanswered wait seen during the current stall, if in one.
    stalled_for: Option<Duration>,
}

impl StallTracker {
    fn observe(&mut self, unanswered_for: Option<Duration>) -> Option<Transition> {
        match (self.stalled_for, unanswered_for) {
            (None, Some(w)) if w >= STALL_AFTER => {
                self.stalled_for = Some(w);
                Some(Transition::Started(w))
            }
            (Some(_), Some(w)) => {
                self.stalled_for = Some(w);
                None
            }
            (Some(longest), None) => {
                self.stalled_for = None;
                Some(Transition::Ended(longest))
            }
            _ => None,
        }
    }
}

/// Run `/usr/bin/sample` against this process on a helper thread, writing
/// into the log directory, then trim old samples. Every failure is logged
/// and swallowed: this is evidence-gathering, never a reason to disturb
/// the app further.
#[cfg(target_os = "macos")]
fn capture_sample() {
    let dir = claudepot_core::paths::log_dir();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("{SAMPLE_PREFIX}{stamp}.txt"));
    let pid = std::process::id();
    let spawned = std::thread::Builder::new()
        .name("main-thread-sample".into())
        .spawn(move || {
            let out = std::process::Command::new("/usr/bin/sample")
                .arg(pid.to_string())
                .arg("3")
                .arg("-mayDie")
                .arg("-file")
                .arg(&path)
                .output();
            match out {
                Ok(o) if o.status.success() => tracing::warn!(
                    path = %path.display(),
                    "main-thread watchdog: captured a stack sample of the stall"
                ),
                Ok(o) => tracing::warn!(
                    status = %o.status,
                    stderr = %String::from_utf8_lossy(&o.stderr).trim(),
                    "main-thread watchdog: /usr/bin/sample failed"
                ),
                Err(e) => tracing::warn!(error = %e, "main-thread watchdog: could not run /usr/bin/sample"),
            }
            prune_samples(&dir);
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "main-thread watchdog: could not start the sampler");
    }
}

/// Keep the newest [`SAMPLES_KEPT`] sample files. The timestamp in the
/// name sorts lexically in time order.
#[cfg(target_os = "macos")]
fn prune_samples(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut samples: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(SAMPLE_PREFIX))
        })
        .collect();
    samples.sort();
    let excess = samples.len().saturating_sub(SAMPLES_KEPT);
    for old in &samples[..excess] {
        if let Err(e) = std::fs::remove_file(old) {
            tracing::debug!(path = %old.display(), error = %e, "main-thread watchdog: prune failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_secs(1);

    #[test]
    fn a_ping_answered_in_time_is_never_a_stall() {
        let mut t = StallTracker::default();
        assert_eq!(t.observe(None), None);
        assert_eq!(t.observe(Some(SHORT)), None);
        assert_eq!(t.observe(None), None);
    }

    #[test]
    fn a_stall_is_reported_once_when_it_starts_and_once_when_it_ends() {
        let mut t = StallTracker::default();
        assert_eq!(t.observe(Some(SHORT)), None);
        assert_eq!(
            t.observe(Some(STALL_AFTER)),
            Some(Transition::Started(STALL_AFTER))
        );
        // Still stuck: no repeat warning every second.
        assert_eq!(t.observe(Some(STALL_AFTER * 3)), None);
        assert_eq!(t.observe(Some(STALL_AFTER * 9)), None);
        // The ending reports the longest wait seen, not the first.
        assert_eq!(t.observe(None), Some(Transition::Ended(STALL_AFTER * 9)));
        assert_eq!(t.observe(None), None);
    }

    #[test]
    fn a_second_stall_after_recovery_is_reported_again() {
        let mut t = StallTracker::default();
        assert!(matches!(
            t.observe(Some(STALL_AFTER)),
            Some(Transition::Started(_))
        ));
        assert!(matches!(t.observe(None), Some(Transition::Ended(_))));
        assert!(matches!(
            t.observe(Some(STALL_AFTER)),
            Some(Transition::Started(_))
        ));
    }
}

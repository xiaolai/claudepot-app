//! Background poller for `claude doctor`.
//!
//! Spawned once from `setup()`; lives for the app's lifetime. Each
//! cycle runs a fresh `scrape_with_probes`, pushes the verdict to
//! [`crate::state::TrayHealthState`], and rebuilds the tray menu so
//! the Health row stays current even when the window is closed.
//!
//! **The watcher feeds the tray only.** The IPC command's 60 s
//! snapshot cache (`commands::cc_doctor::CcDoctorState`) is
//! renderer-owned: only `cc_doctor_snapshot` writes it, and the
//! renderer's 60 s poll keeps it fresh whenever the window is open
//! — the only scenario where IPC staleness matters. The watcher's
//! scrape deliberately does not touch it; exposing a cache setter
//! would widen the command's surface to save at most one scrape per
//! open-window tick.
//!
//! Cadence (5 min) is intentionally slow:
//!
//! - Each scrape is 6–10 s of blocking work — the pty must wait for
//!   CC's npm dist-tag fetch in the Updates section. A tighter
//!   cadence would burn CPU for no real change in health status.
//! - The first tick is delayed [`FIRST_TICK_DELAY`] so the renderer's
//!   own first scrape lands first and the user doesn't see a
//!   double-scrape race at boot.
//!
//! No single-flight gate: a tick that overlaps with the next tick
//! is harmless — the second scrape overwrites TrayHealthState with
//! the same-or-newer verdict.

use std::time::Duration;
use tauri::AppHandle;
use tauri::Manager;

use crate::commands::cc_doctor::push_to_tray_health;
use crate::state::TrayHealthState;

const FIRST_TICK_DELAY: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How many consecutive unparseable scrapes before this watcher stops
/// spawning `claude doctor` at all.
///
/// # Why a watcher that polls a CLI needs a breaker
///
/// `claude doctor` is the last CLI spawn Claudepot puts on a timer.
/// CC's grammar is `claude [options] [command] [prompt]`, so if that
/// verb is ever renamed or removed the positional becomes a *prompt*:
/// a headless model call, billed, every five minutes, forever. That is
/// exactly what issue #94 was — `claude daemon status` against a build
/// with no `daemon` subcommand, ~20K uncached tokens a minute, 288
/// sessions in a day.
///
/// `cc_daemon` fixed its half by reading a file instead. That remedy
/// does not transfer here: `doctor` is a computed diagnostic, not
/// stored state, so there is nothing to read. A version floor
/// ([`claudepot_core::cc_capability`]) does not help either — a floor
/// guards the lower bound only and cannot see a verb being removed.
///
/// What does work is the observation that a fall-through can never
/// produce CC's "Diagnostics" header, so it lands as
/// [`ParseStatus::Degraded`] or `Failed` on **every** attempt. Three
/// of those in a row and we stop. That caps the damage at three calls
/// rather than 288 a day — and had it existed, it would have capped
/// issue #94 the same way.
const MAX_CONSECUTIVE_UNPARSED: u32 = 3;

/// Breaker state threaded through the poll loop. Functional state
/// passing via `spawn_poller_with_state`, so there is no shared mutex
/// and the judgement below stays a pure function of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Breaker {
    consecutive_unparsed: u32,
    tripped: bool,
}

/// What one scrape says about whether `claude doctor` still exists.
///
/// **Three outcomes, not two.** An earlier revision counted every
/// non-`Ok` parse status, which folded in `ParseStatus::Failed` — the
/// status core returns when `claude` is missing, the pty cannot be
/// opened, or the spawn errors. None of those can be a billed prompt,
/// and three of them in a row would have tripped the breaker
/// permanently: a machine without Claude Code installed would have
/// killed its own tray health signal on the third tick, and installing
/// CC afterwards would not have brought it back until a restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScrapeVerdict {
    /// Output parsed as `claude doctor` output. The verb is there.
    Parsed,
    /// Spawn succeeded and the output was **not** doctor output — no
    /// `Diagnostics` header. This is the fall-through signature: a
    /// prompt reply is text, and it is never that header.
    Unparseable,
    /// The spawn itself failed. Says nothing either way.
    Infrastructure,
}

impl ScrapeVerdict {
    fn of(snapshot: &claudepot_core::cc_doctor::DoctorSnapshot) -> Self {
        use claudepot_core::cc_doctor::ParseStatus;
        match snapshot.parse_status {
            ParseStatus::Ok => Self::Parsed,
            ParseStatus::Degraded { .. } => Self::Unparseable,
            ParseStatus::Failed { .. } => Self::Infrastructure,
        }
    }
}

/// Pure judgement, split from the scrape so it is testable without a
/// pty, a CC binary or a Tauri handle — the same split
/// `check-envvar-layout` needed after its measurement half turned out
/// to be unrunnable in CI.
fn advance(prev: Breaker, verdict: ScrapeVerdict) -> Breaker {
    if prev.tripped {
        return prev;
    }
    match verdict {
        // A single good scrape clears the count. Drift that comes and
        // goes is CC being flaky, not the verb being gone — the
        // failure this guards against is permanent by construction.
        ScrapeVerdict::Parsed => Breaker::default(),
        // No evidence: neither counts toward a trip nor clears one, so
        // a flapping install cannot mask a real run of unparseable
        // output by resetting the counter between them.
        ScrapeVerdict::Infrastructure => prev,
        ScrapeVerdict::Unparseable => {
            let consecutive_unparsed = prev.consecutive_unparsed.saturating_add(1);
            Breaker {
                consecutive_unparsed,
                tripped: consecutive_unparsed >= MAX_CONSECUTIVE_UNPARSED,
            }
        }
    }
}

/// Once tripped, stay tripped for the life of the process.
///
/// Deliberately not a backoff. The two causes of a permanently
/// unparseable scrape are CC changing its output format — which a
/// Claudepot update fixes, not a retry — and the verb being gone,
/// where every retry is billed. Neither is helped by trying again
/// later, and a breaker that reopens on a timer is a slower version of
/// the bug. A Claudepot restart re-probes, which is the honest way
/// back.
fn should_scrape(b: Breaker) -> bool {
    !b.tripped
}

pub fn spawn(app: AppHandle) {
    crate::poller::spawn_poller_with_state(
        app,
        "cc_doctor_watcher",
        FIRST_TICK_DELAY,
        Breaker::default(),
        |app, breaker| async move {
            let next = tick(&app, breaker).await;
            (next, POLL_INTERVAL)
        },
    );
}

/// Run `scrape` only when the breaker allows it, and fold the result
/// back into the breaker.
///
/// The scrape is injected so the one guarantee that costs money if it
/// breaks — **a tripped breaker never reaches the scrape** — is
/// testable without a Tauri handle, a pty, or a CC binary. Without
/// this seam a refactor could hoist the spawn above the guard and
/// leave every `advance` test green.
async fn guarded_scrape<F, Fut>(
    breaker: Breaker,
    scrape: F,
) -> (Breaker, Option<claudepot_core::cc_doctor::DoctorSnapshot>)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<claudepot_core::cc_doctor::DoctorSnapshot>>,
{
    if !should_scrape(breaker) {
        return (breaker, None);
    }
    // A join failure produces no snapshot and no verdict — it says
    // nothing about CC's output, so it must not advance the breaker.
    let Some(snapshot) = scrape().await else {
        return (breaker, None);
    };
    let breaker = advance(breaker, ScrapeVerdict::of(&snapshot));
    if breaker.tripped {
        tracing::warn!(
            consecutive = breaker.consecutive_unparsed,
            "cc_doctor_watcher: `claude doctor` output has been unparseable \
             {MAX_CONSECUTIVE_UNPARSED} times running; stopping the poll. If the \
             subcommand was removed upstream, each further spawn would be billed \
             as a model prompt. Restart Claudepot to re-probe."
        );
    }
    (breaker, Some(snapshot))
}

async fn tick(app: &AppHandle, breaker: Breaker) -> Breaker {
    // `scrape_with_probes` over bare `scrape_doctor`: the watcher's
    // verdict feeds the tray menu copy, which is closed-window
    // users' only health signal. If the TUI parser breaks (and the
    // renderer's pane isn't open to surface the failure), the
    // probes still give us cc_version + install identity so the
    // tray label reads "Health: ok" instead of "Health: 1 issue"
    // (the old aggregate_severity's forced-Warning behavior).
    let (breaker, snapshot) = guarded_scrape(breaker, || async {
        tokio::task::spawn_blocking(claudepot_core::cc_doctor::scrape_with_probes)
            .await
            .inspect_err(|e| {
                tracing::warn!("cc_doctor_watcher: blocking task join failed: {e}");
            })
            .ok()
    })
    .await;

    let Some(snapshot) = snapshot else {
        return breaker;
    };

    // Mirror to tray state, then ask for a tray rebuild. The IPC
    // command's snapshot cache is deliberately NOT written here —
    // see the module doc ("the watcher feeds the tray only").
    if let Some(tray_state) = app.try_state::<TrayHealthState>() {
        push_to_tray_health(&tray_state, &snapshot);
    } else {
        tracing::warn!("cc_doctor_watcher: TrayHealthState not managed; tick wasted");
        return breaker;
    }
    if let Err(e) = crate::tray::rebuild(app).await {
        tracing::warn!("cc_doctor_watcher: tray rebuild failed: {e}");
    }
    breaker
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::cc_doctor::{DoctorSeverity, DoctorSnapshot, ParseStatus};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ScrapeVerdict::{Infrastructure, Parsed, Unparseable};

    /// Drive `advance` over a sequence of verdicts.
    fn run(verdicts: &[ScrapeVerdict]) -> Breaker {
        verdicts
            .iter()
            .fold(Breaker::default(), |b, v| advance(b, *v))
    }

    fn snapshot(parse_status: ParseStatus) -> DoctorSnapshot {
        DoctorSnapshot {
            cc_version: None,
            install_type: None,
            install_path: None,
            severity: DoctorSeverity::Healthy,
            sections: Vec::new(),
            raw_bytes: 0,
            parse_status,
            captured_at_ms: 0,
        }
    }

    #[test]
    fn a_healthy_watcher_never_trips() {
        assert!(should_scrape(run(&[Parsed; 100])));
    }

    #[test]
    fn trips_after_the_threshold_and_not_before() {
        // The boundary is the whole point: one call too many is a
        // billed model prompt on a build that lost the subcommand.
        assert!(
            should_scrape(run(&[Unparseable, Unparseable])),
            "2 failures must not trip"
        );
        assert!(
            !should_scrape(run(&[Unparseable, Unparseable, Unparseable])),
            "3 must"
        );
    }

    #[test]
    fn stays_tripped_once_tripped() {
        // Not a backoff. A breaker that reopens on a timer is a slower
        // version of the bug it exists to stop.
        let tripped = run(&[Unparseable; 3]);
        assert!(!should_scrape(advance(tripped, Parsed)));
    }

    #[test]
    fn one_good_scrape_clears_the_count() {
        // Intermittent drift is CC being flaky; the failure this
        // guards against is permanent by construction, so a run that
        // recovers must not accumulate toward a trip.
        assert!(should_scrape(run(&[
            Unparseable,
            Unparseable,
            Parsed,
            Unparseable,
            Unparseable
        ])));
    }

    /// The regression this three-way split exists for. A machine with
    /// no `claude` installed returns `Failed` on every tick; counting
    /// those would kill the tray's health signal on the third one and
    /// never recover it, even after the user installed Claude Code.
    #[test]
    fn infrastructure_failures_never_trip_the_breaker() {
        assert!(
            should_scrape(run(&[Infrastructure; 50])),
            "a missing binary or a pty error cannot be a billed prompt"
        );
    }

    /// ...but they must not launder a real run of unparseable output
    /// by resetting the counter between them either.
    #[test]
    fn infrastructure_failures_do_not_clear_the_count() {
        assert!(!should_scrape(run(&[
            Unparseable,
            Infrastructure,
            Unparseable,
            Infrastructure,
            Unparseable
        ])));
    }

    /// The mapping from core's status to our verdict, pinned — this is
    /// where the regression actually lived.
    #[test]
    fn only_a_degraded_parse_counts_as_a_fallthrough() {
        assert_eq!(ScrapeVerdict::of(&snapshot(ParseStatus::Ok)), Parsed);
        assert_eq!(
            ScrapeVerdict::of(&snapshot(ParseStatus::Degraded {
                reason: "no Diagnostics header".into()
            })),
            Unparseable
        );
        assert_eq!(
            ScrapeVerdict::of(&snapshot(ParseStatus::Failed {
                reason: "claude binary not found".into()
            })),
            Infrastructure
        );
    }

    #[test]
    fn the_count_cannot_overflow_into_a_reset() {
        // `saturating_add`, not `wrapping_add`: a rollover would take
        // the count back to 0 and silently resume spawning.
        //
        // `tripped` must be FALSE here or the early return in
        // `advance` answers before the arithmetic ever runs — which is
        // what the first version of this test did, passing happily
        // against a `wrapping_add` build while claiming to guard it.
        let b = Breaker {
            consecutive_unparsed: u32::MAX,
            tripped: false,
        };
        assert!(
            !should_scrape(advance(b, Unparseable)),
            "a saturated count must still read as tripped"
        );
    }

    /// The guarantee that costs money if it breaks: once tripped, the
    /// scrape closure is never reached. Asserted through the seam
    /// rather than through `advance`, because every `advance` test
    /// would stay green if a refactor hoisted the spawn above the
    /// guard.
    #[tokio::test]
    async fn a_tripped_breaker_never_reaches_the_scrape() {
        let calls = AtomicUsize::new(0);
        let tripped = Breaker {
            consecutive_unparsed: MAX_CONSECUTIVE_UNPARSED,
            tripped: true,
        };
        let (next, snap) = guarded_scrape(tripped, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(snapshot(ParseStatus::Ok))
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 0, "must not spawn `claude`");
        assert!(snap.is_none());
        assert_eq!(next, tripped, "a skipped tick changes nothing");
    }

    #[tokio::test]
    async fn an_untripped_breaker_does_reach_the_scrape() {
        // The other direction — without this, a `guarded_scrape` that
        // never called anything would pass the test above.
        let calls = AtomicUsize::new(0);
        let (next, snap) = guarded_scrape(Breaker::default(), || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(snapshot(ParseStatus::Ok))
        })
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(snap.is_some());
        assert!(should_scrape(next));
    }

    /// A join failure yields no snapshot, and must leave the breaker
    /// untouched — our own runtime hiccup is not evidence about CC.
    #[tokio::test]
    async fn a_scrape_that_yields_nothing_does_not_advance_the_breaker() {
        let before = run(&[Unparseable]);
        let (after, snap) = guarded_scrape(before, || async { None }).await;
        assert!(snap.is_none());
        assert_eq!(after, before);
    }
}

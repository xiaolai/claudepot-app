//! Choosing a server for one upstream (plan §7.2). Pure: the probes and
//! the clock are inputs, so every rule here is a unit test.
//!
//! The **upstream** is the user's choice. Scoring only picks which of
//! the servers publishing it to use:
//!
//! - **Eligible** only if the live target fingerprint is the approved
//!   one, the device's acceptance is current, the credential is ready,
//!   there is a free slot, no cooldown, and — subscription only — the
//!   utilization is unknown or under 95 % (5 h) / 98 % (7 d).
//! - **Measured tier** when the server has fresh TTFB samples for the
//!   reference model; ordered by `rtt + ttfb`.
//! - **Connect tier** otherwise; ordered by `rtt + connect_ms`, and always
//!   after the measured tier. The two are never added into one number: a
//!   cold server must not win because its inference cost was never seen.
//! - Ties go to the user's pinned order.

use chrono::{DateTime, Utc};

use super::client::Probe;
use super::health::{OwnerState, UpstreamHealth, Utilization};
use super::identity::TargetKind;
use super::ledger::SAMPLE_TTL;
use super::servers::{ApprovedUpstream, ServerRecord};
use super::target::UpstreamId;

/// Subscription utilization at or above these is not eligible.
pub const FIVE_HOUR_CEILING: f64 = 95.0;
pub const SEVEN_DAY_CEILING: f64 = 98.0;

/// One paired server and what probing it returned.
pub struct Observation<'a> {
    pub server: &'a ServerRecord,
    /// The error's display text when the probe failed.
    pub probe: Result<Probe, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier {
    /// Fresh TTFB samples exist for `model`.
    Measured { model: String, ttfb_ms: u64 },
    /// Reachable; inference speed unmeasured.
    Connect,
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Measured { model, ttfb_ms } => {
                write!(f, "measured ({model}, ttfb {ttfb_ms} ms)")
            }
            Tier::Connect => f.write_str("reachable, speed unmeasured"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked {
    pub server_id: String,
    pub name: String,
    pub tier: Tier,
    pub rtt_ms: u64,
    /// `rtt + ttfb` (measured) or `rtt + connect_ms` (connect). Compared
    /// only within a tier.
    pub score_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ineligible {
    Unreachable(String),
    /// The server answered as a different server id than was paired.
    ServerIdChanged,
    /// This client never approved the upstream on this server.
    NotApproved,
    /// Approved here under a different target than the one chosen — the
    /// same name on two servers is the same target only when the
    /// fingerprints match.
    OtherTarget,
    /// Not in the health report: unpublished, or not granted to this
    /// device.
    NotOffered,
    /// The live fingerprint differs from the approved one: the server
    /// changed what this upstream is. `relay accept` shows the new one.
    FingerprintChanged,
    /// Same target, but the server bumped its generation or epoch.
    NotAccepted,
    CredentialNotReady(Option<String>),
    OwnerConflict,
    NoCapacity,
    Cooldown(u64),
    Saturated {
        window: &'static str,
        utilization: f64,
    },
}

impl std::fmt::Display for Ineligible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ineligible::Unreachable(e) => write!(f, "unreachable: {e}"),
            Ineligible::ServerIdChanged => {
                f.write_str("answered as a different relay than the one paired")
            }
            Ineligible::NotApproved => f.write_str("upstream not approved on this relay"),
            Ineligible::OtherTarget => f.write_str("publishes a different target under this name"),
            Ineligible::NotOffered => f.write_str("upstream not published or not granted"),
            Ineligible::FingerprintChanged => {
                f.write_str("target changed since you approved it — run `claudepot relay accept`")
            }
            Ineligible::NotAccepted => {
                f.write_str("acceptance required — run `claudepot relay accept`")
            }
            Ineligible::CredentialNotReady(None) => f.write_str("credential not ready"),
            Ineligible::CredentialNotReady(Some(e)) => write!(f, "credential not ready: {e}"),
            Ineligible::OwnerConflict => {
                f.write_str("Claude Code on the relay is signed into this account")
            }
            Ineligible::NoCapacity => f.write_str("no free request slots"),
            Ineligible::Cooldown(s) => write!(f, "rate-limited for {s} s more"),
            Ineligible::Saturated {
                window,
                utilization,
            } => write!(f, "{window} usage at {utilization:.0}%"),
        }
    }
}

impl Ineligible {
    /// A stable code for surfaces that must not carry relay-supplied
    /// text. `Unreachable` wraps the probe's error text and
    /// `CredentialNotReady` the health report's, so neither payload
    /// leaves through this.
    pub fn code(&self) -> &'static str {
        match self {
            Ineligible::Unreachable(_) => "unreachable",
            Ineligible::ServerIdChanged => "server_id_changed",
            Ineligible::NotApproved => "not_approved",
            Ineligible::OtherTarget => "other_target",
            Ineligible::NotOffered => "not_offered",
            Ineligible::FingerprintChanged => "fingerprint_changed",
            Ineligible::NotAccepted => "not_accepted",
            Ineligible::CredentialNotReady(_) => "credential_not_ready",
            Ineligible::OwnerConflict => "owner_conflict",
            Ineligible::NoCapacity => "no_capacity",
            Ineligible::Cooldown(_) => "cooldown",
            Ineligible::Saturated { .. } => "saturated",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Excluded {
    pub server_id: String,
    pub name: String,
    pub reason: Ineligible,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Selection {
    /// Best first.
    pub ranked: Vec<Ranked>,
    pub excluded: Vec<Excluded>,
    /// Pairs of server names reporting the same subscription account
    /// fingerprint. Advisory (plan §5.4): two independent logins are
    /// safe, a login copied between machines is not, and the client
    /// cannot tell which.
    pub duplicate_accounts: Vec<(String, String)>,
}

impl Selection {
    pub fn pick(&self) -> Option<&Ranked> {
        self.ranked.first()
    }
}

fn saturated(u: &Utilization, now: DateTime<Utc>) -> Option<Ineligible> {
    // A window whose reset has passed is unknown, not high.
    let live = |w: &Option<super::health::Window>| {
        w.as_ref()
            .filter(|w| w.resets_at > now && w.utilization.is_finite())
            .map(|w| w.utilization)
    };
    if let Some(v) = live(&u.five_hour).filter(|v| *v >= FIVE_HOUR_CEILING) {
        return Some(Ineligible::Saturated {
            window: "5-hour",
            utilization: v,
        });
    }
    live(&u.seven_day)
        .filter(|v| *v >= SEVEN_DAY_CEILING)
        .map(|v| Ineligible::Saturated {
            window: "7-day",
            utilization: v,
        })
}

fn eligibility(
    approved_fp: &str,
    up: &UpstreamHealth,
    now: DateTime<Utc>,
) -> Result<(), Ineligible> {
    if up.target_fingerprint != approved_fp {
        return Err(Ineligible::FingerprintChanged);
    }
    if !up.accepted {
        return Err(Ineligible::NotAccepted);
    }
    if up.owner_state == Some(OwnerState::Conflict) {
        return Err(Ineligible::OwnerConflict);
    }
    if !up.credential_ready {
        return Err(Ineligible::CredentialNotReady(up.credential_error.clone()));
    }
    if up.capacity_free == 0 {
        return Err(Ineligible::NoCapacity);
    }
    if let Some(s) = up.cooldown_secs.filter(|s| *s > 0) {
        return Err(Ineligible::Cooldown(s));
    }
    if up.kind == TargetKind::Subscription {
        if let Some(r) = up.utilization.as_ref().and_then(|u| saturated(u, now)) {
            return Err(r);
        }
    }
    Ok(())
}

fn fresh_ttfb(up: &UpstreamHealth, model: &str) -> Option<u64> {
    up.latency
        .iter()
        .find(|l| l.model == model && l.count > 0 && l.newest_age_secs <= SAMPLE_TTL.as_secs())
        .map(|l| l.median_ttfb_ms)
}

/// Rank the servers in `observed` (in pinned order) for `upstream`.
///
/// The logical target is the fingerprint approved on the first server
/// that approved `upstream`; a server that approved a different one is
/// [`Ineligible::OtherTarget`]. `model_hint` names the model to compare
/// TTFB on; when no candidate has samples for it, the model with the
/// most samples across candidates is used, so every measured server is
/// compared on the same model.
pub fn select(
    observed: &[Observation<'_>],
    upstream: &UpstreamId,
    model_hint: Option<&str>,
    now: DateTime<Utc>,
) -> Selection {
    let mut out = Selection::default();
    let target_fp = observed
        .iter()
        .find_map(|o| o.server.approved(upstream))
        .map(|a| a.fingerprint.as_str());

    // Eligible candidates, still in pinned order.
    let mut candidates: Vec<Candidate<'_>> = Vec::new();
    let mut accounts: Vec<(&str, &str)> = Vec::new();
    for o in observed {
        let s = o.server;
        let screened = offered(o, upstream, target_fp).and_then(|(probe, approved, up)| {
            // Counted before eligibility: a saturated copy of a login is
            // still a copy.
            if let Some(fp) = subscription_account(up) {
                accounts.push((s.name.as_str(), fp));
            }
            eligibility(&approved.fingerprint, up, now).map(|()| (probe, up))
        });
        match screened {
            Ok((probe, up)) => candidates.push((s, probe, up)),
            Err(reason) => out.excluded.push(Excluded {
                server_id: s.server_id.clone(),
                name: s.name.clone(),
                reason,
            }),
        }
    }

    out.duplicate_accounts = duplicate_accounts(&accounts);
    let reference = reference_model(&candidates, model_hint);
    out.ranked = rank(&candidates, reference.as_deref());
    out
}

/// An eligible server: its record, its probe, and its report for the
/// upstream being selected.
type Candidate<'a> = (&'a ServerRecord, &'a Probe, &'a UpstreamHealth);

/// The probe, approval and health report `o` offers `upstream` under, or
/// why it offers none. Eligibility of the offer is [`eligibility`]'s.
fn offered<'o>(
    o: &'o Observation<'_>,
    upstream: &UpstreamId,
    target_fp: Option<&str>,
) -> Result<(&'o Probe, &'o ApprovedUpstream, &'o UpstreamHealth), Ineligible> {
    let s = o.server;
    let probe = o
        .probe
        .as_ref()
        .map_err(|e| Ineligible::Unreachable(e.clone()))?;
    if probe.health.server_id != s.server_id {
        return Err(Ineligible::ServerIdChanged);
    }
    let approved = s.approved(upstream).ok_or(Ineligible::NotApproved)?;
    if Some(approved.fingerprint.as_str()) != target_fp {
        return Err(Ineligible::OtherTarget);
    }
    let up = probe
        .health
        .upstreams
        .iter()
        .find(|u| u.id == upstream.as_str())
        .ok_or(Ineligible::NotOffered)?;
    Ok((probe, approved, up))
}

/// The account fingerprint a subscription upstream reports.
fn subscription_account(up: &UpstreamHealth) -> Option<&str> {
    if up.kind != TargetKind::Subscription {
        return None;
    }
    up.account_fingerprint.as_deref()
}

/// Every pair of server names reporting the same account, in pinned order.
fn duplicate_accounts(accounts: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, (a, fa)) in accounts.iter().enumerate() {
        for (b, fb) in &accounts[i + 1..] {
            if fa == fb {
                out.push(((*a).to_owned(), (*b).to_owned()));
            }
        }
    }
    out
}

/// The one model every measured candidate is compared on: the hint when
/// some candidate has fresh samples for it, else the model with the most
/// fresh samples across candidates.
fn reference_model(candidates: &[Candidate<'_>], model_hint: Option<&str>) -> Option<String> {
    let hint_measured = model_hint.filter(|m| {
        candidates
            .iter()
            .any(|(_, _, up)| fresh_ttfb(up, m).is_some())
    });
    if let Some(m) = hint_measured {
        return Some(m.to_owned());
    }
    let mut totals: Vec<(String, usize)> = Vec::new();
    for (_, _, up) in candidates {
        for l in &up.latency {
            if fresh_ttfb(up, &l.model).is_none() {
                continue;
            }
            match totals.iter_mut().find(|(m, _)| *m == l.model) {
                Some((_, n)) => *n += l.count,
                None => totals.push((l.model.clone(), l.count)),
            }
        }
    }
    // Most samples wins; equal counts go to the smaller name, so the
    // choice does not depend on report order.
    totals
        .into_iter()
        .max_by(|(ma, na), (mb, nb)| na.cmp(nb).then(mb.cmp(ma)))
        .map(|(m, _)| m)
}

/// Score each candidate and order them: the measured tier first, then by
/// score within a tier, then by pinned order.
fn rank(candidates: &[Candidate<'_>], reference: Option<&str>) -> Vec<Ranked> {
    let mut ranked: Vec<(usize, Ranked)> = candidates
        .iter()
        .enumerate()
        .map(|(pin, (s, probe, up))| {
            let measured = reference.and_then(|m| fresh_ttfb(up, m).map(|t| (m, t)));
            let (tier, score_ms) = match measured {
                Some((m, ttfb)) => (
                    Tier::Measured {
                        model: m.to_owned(),
                        ttfb_ms: ttfb,
                    },
                    probe.rtt_ms.saturating_add(ttfb),
                ),
                None => (Tier::Connect, probe.rtt_ms.saturating_add(probe.connect_ms)),
            };
            (
                pin,
                Ranked {
                    server_id: s.server_id.clone(),
                    name: s.name.clone(),
                    tier,
                    rtt_ms: probe.rtt_ms,
                    score_ms,
                },
            )
        })
        .collect();
    ranked.sort_by(|(pa, a), (pb, b)| {
        let tier = |r: &Ranked| matches!(r.tier, Tier::Connect) as u8;
        tier(a)
            .cmp(&tier(b))
            .then(a.score_ms.cmp(&b.score_ms))
            .then(pa.cmp(pb))
    });
    ranked.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::super::health::{HealthReport, Window};
    use super::super::ledger::LatencySummary;
    use super::super::servers::ApprovedUpstream;
    use super::*;

    fn id(s: &str) -> UpstreamId {
        UpstreamId::parse(s).unwrap()
    }

    fn server(sid: &str, approved: &[(&str, &str)]) -> ServerRecord {
        ServerRecord {
            server_id: sid.into(),
            name: format!("{sid}-name"),
            base_url: "https://relay.test:8787".into(),
            ca_pem: Some(String::new()),
            ca_fingerprint: None,
            device_id: "d".into(),
            default_upstream: None,
            upstreams: approved
                .iter()
                .map(|(u, fp)| ApprovedUpstream {
                    id: id(u),
                    fingerprint: (*fp).into(),
                    generation: 1,
                })
                .collect(),
            paired_at: Utc::now(),
        }
    }

    fn healthy(upstream: &str, kind: TargetKind, fp: &str) -> UpstreamHealth {
        UpstreamHealth {
            id: upstream.into(),
            kind,
            default: false,
            target_fingerprint: fp.into(),
            accepted_generation: 1,
            current_generation: 1,
            accepted: true,
            credential_ready: true,
            credential_error: None,
            owner_state: (kind == TargetKind::Subscription).then_some(OwnerState::Clear),
            utilization: None,
            cooldown_secs: None,
            capacity_free: 8,
            latency: Vec::new(),
            account_fingerprint: None,
        }
    }

    fn probe(sid: &str, connect_ms: u64, rtt_ms: u64, ups: Vec<UpstreamHealth>) -> Probe {
        Probe {
            connect_ms,
            rtt_ms,
            health: HealthReport {
                ok: true,
                server_id: sid.into(),
                version: "test".into(),
                upstreams: ups,
            },
        }
    }

    fn sample(model: &str, ttfb: u64, count: usize, age: u64) -> LatencySummary {
        LatencySummary {
            model: model.into(),
            count,
            median_ttfb_ms: ttfb,
            newest_age_secs: age,
        }
    }

    fn sub() -> UpstreamId {
        id("sub")
    }

    #[test]
    fn a_connect_tier_server_never_outranks_a_measured_one() {
        // A: 700 ms TTFB measured. B: 20 ms to connect, never measured.
        let a = server("a", &[("sub", "fp")]);
        let b = server("b", &[("sub", "fp")]);
        let mut ua = healthy("sub", TargetKind::Subscription, "fp");
        ua.latency = vec![sample("claude-x", 700, 5, 10)];
        let ub = healthy("sub", TargetKind::Subscription, "fp");
        let obs = [
            Observation {
                server: &b,
                probe: Ok(probe("b", 20, 5, vec![ub])),
            },
            Observation {
                server: &a,
                probe: Ok(probe("a", 400, 300, vec![ua])),
            },
        ];
        let sel = select(&obs, &sub(), None, Utc::now());
        assert_eq!(sel.pick().unwrap().server_id, "a");
        assert!(matches!(sel.ranked[0].tier, Tier::Measured { .. }));
        assert_eq!(sel.ranked[0].score_ms, 1000);
        assert_eq!(sel.ranked[1].tier, Tier::Connect);
        assert_eq!(sel.ranked[1].score_ms, 25);
    }

    #[test]
    fn within_a_tier_the_lower_score_wins_and_ties_go_to_the_pinned_order() {
        let a = server("a", &[("sub", "fp")]);
        let b = server("b", &[("sub", "fp")]);
        let c = server("c", &[("sub", "fp")]);
        let with = |ttfb| {
            let mut u = healthy("sub", TargetKind::Subscription, "fp");
            u.latency = vec![sample("m", ttfb, 3, 1)];
            u
        };
        let obs = [
            Observation {
                server: &a,
                probe: Ok(probe("a", 50, 100, vec![with(400)])),
            },
            Observation {
                server: &b,
                probe: Ok(probe("b", 50, 100, vec![with(300)])),
            },
            Observation {
                server: &c,
                probe: Ok(probe("c", 50, 100, vec![with(400)])),
            },
        ];
        let sel = select(&obs, &sub(), None, Utc::now());
        let order: Vec<_> = sel.ranked.iter().map(|r| r.server_id.as_str()).collect();
        assert_eq!(order, ["b", "a", "c"]);
    }

    #[test]
    fn a_stale_sample_is_not_a_measurement() {
        let a = server("a", &[("sub", "fp")]);
        let b = server("b", &[("sub", "fp")]);
        let mut ua = healthy("sub", TargetKind::Subscription, "fp");
        ua.latency = vec![sample("m", 10, 9, SAMPLE_TTL.as_secs() + 1)];
        let mut ub = healthy("sub", TargetKind::Subscription, "fp");
        ub.latency = vec![sample("m", 900, 1, 5)];
        let obs = [
            Observation {
                server: &a,
                probe: Ok(probe("a", 1, 1, vec![ua])),
            },
            Observation {
                server: &b,
                probe: Ok(probe("b", 1, 1, vec![ub])),
            },
        ];
        let sel = select(&obs, &sub(), None, Utc::now());
        assert_eq!(sel.pick().unwrap().server_id, "b");
        assert_eq!(sel.ranked[1].tier, Tier::Connect);
    }

    #[test]
    fn measured_servers_are_compared_on_the_same_model() {
        // A is fast on haiku only; B is measured on the hinted model.
        let a = server("a", &[("sub", "fp")]);
        let b = server("b", &[("sub", "fp")]);
        let mut ua = healthy("sub", TargetKind::Subscription, "fp");
        ua.latency = vec![sample("haiku", 50, 20, 1)];
        let mut ub = healthy("sub", TargetKind::Subscription, "fp");
        ub.latency = vec![sample("opus", 800, 2, 1)];
        let obs = [
            Observation {
                server: &a,
                probe: Ok(probe("a", 1, 1, vec![ua.clone()])),
            },
            Observation {
                server: &b,
                probe: Ok(probe("b", 1, 1, vec![ub.clone()])),
            },
        ];
        let sel = select(&obs, &sub(), Some("opus"), Utc::now());
        assert_eq!(sel.pick().unwrap().server_id, "b");
        assert_eq!(sel.ranked[1].tier, Tier::Connect);

        // No hint: the most-sampled model is the reference.
        let sel = select(&obs, &sub(), None, Utc::now());
        assert_eq!(sel.pick().unwrap().server_id, "a");
    }

    #[test]
    fn unknown_or_expired_utilization_is_eligible_and_high_utilization_is_not() {
        let now = Utc::now();
        let a = server("a", &[("sub", "fp")]);
        let with = |five: f64, resets_in: i64| {
            let mut u = healthy("sub", TargetKind::Subscription, "fp");
            u.utilization = Some(Utilization {
                five_hour: Some(Window {
                    utilization: five,
                    resets_at: now + chrono::Duration::seconds(resets_in),
                }),
                seven_day: None,
            });
            u
        };
        let run = |u: UpstreamHealth| {
            let obs = [Observation {
                server: &a,
                probe: Ok(probe("a", 1, 1, vec![u])),
            }];
            select(&obs, &sub(), None, now)
        };
        // Unknown.
        assert!(run(healthy("sub", TargetKind::Subscription, "fp"))
            .pick()
            .is_some());
        // Under the ceiling.
        assert!(run(with(94.9, 600)).pick().is_some());
        // At the ceiling.
        let sel = run(with(95.0, 600));
        assert!(sel.pick().is_none());
        assert!(matches!(
            sel.excluded[0].reason,
            Ineligible::Saturated {
                window: "5-hour",
                ..
            }
        ));
        // High, but its reset has passed: unknown, not high.
        assert!(run(with(99.0, -1)).pick().is_some());

        // Seven-day ceiling is 98.
        let mut u = healthy("sub", TargetKind::Subscription, "fp");
        u.utilization = Some(Utilization {
            five_hour: None,
            seven_day: Some(Window {
                utilization: 98.0,
                resets_at: now + chrono::Duration::days(1),
            }),
        });
        assert!(run(u).pick().is_none());
    }

    #[test]
    fn utilization_does_not_gate_a_route() {
        let now = Utc::now();
        let a = server("a", &[("ds", "fp")]);
        let mut u = healthy("ds", TargetKind::PassThrough, "fp");
        u.utilization = Some(Utilization {
            five_hour: Some(Window {
                utilization: 100.0,
                resets_at: now + chrono::Duration::hours(1),
            }),
            seven_day: None,
        });
        let obs = [Observation {
            server: &a,
            probe: Ok(probe("a", 1, 1, vec![u])),
        }];
        assert!(select(&obs, &id("ds"), None, now).pick().is_some());
    }

    #[test]
    fn a_mismatched_target_fingerprint_is_excluded() {
        let a = server("a", &[("sub", "fp-approved")]);
        let obs = [Observation {
            server: &a,
            probe: Ok(probe(
                "a",
                1,
                1,
                vec![healthy("sub", TargetKind::Subscription, "fp-new")],
            )),
        }];
        let sel = select(&obs, &sub(), None, Utc::now());
        assert!(sel.pick().is_none());
        assert_eq!(sel.excluded[0].reason, Ineligible::FingerprintChanged);
    }

    #[test]
    fn the_same_name_approved_as_a_different_target_is_another_target() {
        // Both servers publish "ds", but the client approved different
        // targets on each: only the first pinned one's target is chosen.
        let a = server("a", &[("ds", "fp-a")]);
        let b = server("b", &[("ds", "fp-b")]);
        let obs = [
            Observation {
                server: &a,
                probe: Ok(probe(
                    "a",
                    500,
                    500,
                    vec![healthy("ds", TargetKind::PassThrough, "fp-a")],
                )),
            },
            Observation {
                server: &b,
                probe: Ok(probe(
                    "b",
                    1,
                    1,
                    vec![healthy("ds", TargetKind::PassThrough, "fp-b")],
                )),
            },
        ];
        let sel = select(&obs, &id("ds"), None, Utc::now());
        assert_eq!(sel.pick().unwrap().server_id, "a");
        assert_eq!(sel.excluded[0].reason, Ineligible::OtherTarget);
    }

    #[test]
    fn readiness_is_per_upstream_in_both_directions() {
        // A blocked subscription does not block a healthy route on the
        // same server, and a blocked route does not block the
        // subscription.
        let a = server("a", &[("sub", "fs"), ("ds", "fd")]);
        let mut blocked_sub = healthy("sub", TargetKind::Subscription, "fs");
        blocked_sub.owner_state = Some(OwnerState::Conflict);
        blocked_sub.credential_ready = false;
        let ok_route = healthy("ds", TargetKind::PassThrough, "fd");
        let obs = [Observation {
            server: &a,
            probe: Ok(probe("a", 1, 1, vec![blocked_sub, ok_route])),
        }];
        let sel = select(&obs, &id("ds"), None, Utc::now());
        assert_eq!(sel.pick().unwrap().server_id, "a");
        let sel = select(&obs, &sub(), None, Utc::now());
        assert!(sel.pick().is_none());
        assert_eq!(sel.excluded[0].reason, Ineligible::OwnerConflict);

        let ok_sub = healthy("sub", TargetKind::Subscription, "fs");
        let mut blocked_route = healthy("ds", TargetKind::PassThrough, "fd");
        blocked_route.credential_ready = false;
        blocked_route.credential_error = Some("credential_unavailable".into());
        let obs = [Observation {
            server: &a,
            probe: Ok(probe("a", 1, 1, vec![ok_sub, blocked_route])),
        }];
        assert!(select(&obs, &sub(), None, Utc::now()).pick().is_some());
        let sel = select(&obs, &id("ds"), None, Utc::now());
        assert_eq!(
            sel.excluded[0].reason,
            Ineligible::CredentialNotReady(Some("credential_unavailable".into()))
        );
    }

    #[test]
    fn each_gate_excludes_with_its_reason() {
        let a = server("a", &[("sub", "fp")]);
        let cases: Vec<(Box<dyn Fn(&mut UpstreamHealth)>, Ineligible)> = vec![
            (Box::new(|u| u.accepted = false), Ineligible::NotAccepted),
            (Box::new(|u| u.capacity_free = 0), Ineligible::NoCapacity),
            (
                Box::new(|u| u.cooldown_secs = Some(12)),
                Ineligible::Cooldown(12),
            ),
            (
                Box::new(|u| u.credential_ready = false),
                Ineligible::CredentialNotReady(None),
            ),
        ];
        for (mutate, want) in cases {
            let mut u = healthy("sub", TargetKind::Subscription, "fp");
            mutate(&mut u);
            let obs = [Observation {
                server: &a,
                probe: Ok(probe("a", 1, 1, vec![u])),
            }];
            let sel = select(&obs, &sub(), None, Utc::now());
            assert!(sel.pick().is_none(), "{want:?} must exclude");
            assert_eq!(sel.excluded[0].reason, want);
        }
    }

    #[test]
    fn unreachable_unapproved_unoffered_and_impostor_servers_are_excluded() {
        let a = server("a", &[("sub", "fp")]);
        let b = server("b", &[]);
        let c = server("c", &[("sub", "fp")]);
        let d = server("d", &[("sub", "fp")]);
        let obs = [
            Observation {
                server: &a,
                probe: Err("timed out".into()),
            },
            Observation {
                server: &b,
                probe: Ok(probe(
                    "b",
                    1,
                    1,
                    vec![healthy("sub", TargetKind::Subscription, "fp")],
                )),
            },
            Observation {
                server: &c,
                probe: Ok(probe("c", 1, 1, vec![])),
            },
            Observation {
                server: &d,
                probe: Ok(probe(
                    "someone-else",
                    1,
                    1,
                    vec![healthy("sub", TargetKind::Subscription, "fp")],
                )),
            },
        ];
        let sel = select(&obs, &sub(), None, Utc::now());
        assert!(sel.pick().is_none());
        let reasons: Vec<_> = sel.excluded.iter().map(|e| e.reason.clone()).collect();
        assert_eq!(
            reasons,
            [
                Ineligible::Unreachable("timed out".into()),
                Ineligible::NotApproved,
                Ineligible::NotOffered,
                Ineligible::ServerIdChanged,
            ]
        );
    }

    #[test]
    fn equal_account_fingerprints_across_servers_warn_and_distinct_ones_do_not() {
        let a = server("a", &[("sub", "fp")]);
        let b = server("b", &[("sub", "fp")]);
        let c = server("c", &[("sub", "fp")]);
        let acct = |f: &str| {
            let mut u = healthy("sub", TargetKind::Subscription, "fp");
            u.account_fingerprint = Some(f.into());
            u
        };
        let obs = [
            Observation {
                server: &a,
                probe: Ok(probe("a", 1, 1, vec![acct("same")])),
            },
            Observation {
                server: &b,
                probe: Ok(probe("b", 1, 1, vec![acct("same")])),
            },
            Observation {
                server: &c,
                probe: Ok(probe("c", 1, 1, vec![acct("other")])),
            },
        ];
        let sel = select(&obs, &sub(), None, Utc::now());
        assert_eq!(
            sel.duplicate_accounts,
            [("a-name".to_owned(), "b-name".to_owned())]
        );
        // Advisory only: all three stay eligible.
        assert_eq!(sel.ranked.len(), 3);
    }
}

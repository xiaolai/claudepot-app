//! What every dispatched request cost, and how fast it was.
//!
//! Two consumers of one [`OutcomeRecord`]:
//!
//! - [`Ledger`] appends one line to `relay-usage.jsonl` (append-only;
//!   nothing prunes it). Each line names the device, the upstream and its
//!   target digest, the model the client **asked for** and the one that
//!   **answered** — different whenever a route maps names — and the
//!   usage. Cost is priced through [`PriceBook`] **only for an Anthropic
//!   upstream**, and always marked approximate: the book is keyed by
//!   model name alone, so a client `sonnet` mapped to another provider
//!   would otherwise be priced as Claude.
//! - [`Stats`] keeps a short, in-memory window of time-to-first-byte
//!   samples per `(upstream, actual model)` for `GET /relay/health`.
//!   A sample has an age, so a client can tell a fresh measurement from
//!   an old one; it is never persisted.

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Utc};
use parking_lot::Mutex;

use super::identity::TargetKind;
use super::server::OutcomeRecord;
use super::upstream::{OutcomeKind, ProviderUsage, UpstreamId, Usage};
use crate::pricing::PriceBook;
use crate::session::{PremiumUsage, TokenUsage};
use crate::session_live::pricing::RateConfidence;

pub const USAGE_FILENAME: &str = "relay-usage.jsonl";
pub const LEDGER_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LedgerUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    /// Reasoning tokens, inside `output_tokens`, when a translated
    /// provider reports them. Absent on Anthropic's shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

impl From<Usage> for LedgerUsage {
    fn from(u: Usage) -> Self {
        Self {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_creation_input_tokens: u.cache_creation_input_tokens,
            cache_read_input_tokens: u.cache_read_input_tokens,
            reasoning_tokens: None,
        }
    }
}

/// In the provider's semantics: `input_tokens` still includes the cached
/// ones, which `cache_read_input_tokens` repeats. Providers report no
/// cache-creation count.
impl From<ProviderUsage> for LedgerUsage {
    fn from(u: ProviderUsage) -> Self {
        Self {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: u.cached_input_tokens,
            reasoning_tokens: u.reasoning_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UsagePair {
    /// The provider's own counters. For a forwarded upstream the bytes are
    /// the provider's, so this equals `normalized`; a translated one (P10)
    /// fills it from the provider's terminal `usage`, in the provider's
    /// semantics, and leaves it `null` when the provider sent none —
    /// never a copy of `normalized` dressed up as the provider's.
    pub native: Option<LedgerUsage>,
    /// What the client saw, in Anthropic's shape.
    pub normalized: LedgerUsage,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cost {
    pub usd: f64,
    /// Always `true`: a relay cost is an estimate from published rates.
    pub approximate: bool,
    /// The model id was not in the rate table; its family's rate was used.
    pub family_estimate: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LedgerLine {
    pub schema: u32,
    pub ts: DateTime<Utc>,
    pub device: String,
    pub upstream: UpstreamId,
    pub target: String,
    pub generation: u64,
    pub operation: String,
    pub requested_model: Option<String>,
    pub model: Option<String>,
    pub outcome: OutcomeKind,
    pub status: Option<u16>,
    pub ttfb_ms: Option<u64>,
    pub upstream_request_id: Option<String>,
    pub error: Option<String>,
    pub bytes: u64,
    pub usage: Option<UsagePair>,
    /// `None` for an unpriced upstream, or when no rate is known.
    pub cost: Option<Cost>,
}

impl LedgerLine {
    pub fn from_record(rec: &OutcomeRecord, ts: DateTime<Utc>, prices: &PriceBook) -> Self {
        let o = &rec.outcome;
        let forwarded = rec.target.kind != TargetKind::Translated;
        let usage = o.usage.map(|u| UsagePair {
            native: if forwarded {
                Some(u.into())
            } else {
                o.native_usage.map(Into::into)
            },
            normalized: u.into(),
        });
        let cost = match (rec.target.kind, o.usage, o.model.as_deref()) {
            (TargetKind::Subscription, Some(u), Some(model)) => price(prices, model, ts, u),
            _ => None,
        };
        Self {
            schema: LEDGER_SCHEMA,
            ts,
            device: rec.device_id.clone(),
            upstream: rec.target.id.clone(),
            target: rec.target.fingerprint.clone(),
            generation: rec.target.generation,
            operation: rec.operation.path().to_owned(),
            requested_model: rec.requested_model.clone(),
            model: o.model.clone(),
            outcome: o.kind,
            status: o.status,
            ttfb_ms: o.ttfb.map(|d| d.as_millis() as u64),
            upstream_request_id: o.upstream_request_id.clone(),
            error: o.error.clone(),
            bytes: o.bytes,
            usage,
            cost,
        }
    }
}

fn price(prices: &PriceBook, model: &str, ts: DateTime<Utc>, u: Usage) -> Option<Cost> {
    let tokens = TokenUsage {
        input: u.input_tokens,
        output: u.output_tokens,
        cache_creation: u.cache_creation_input_tokens,
        cache_read: u.cache_read_input_tokens,
        cache_creation_1h: 0,
        web_search_requests: 0,
    };
    let day = (ts.year(), ts.month(), ts.day());
    let priced = prices.cost(model, day, &tokens, &PremiumUsage::default())?;
    Some(Cost {
        usd: priced.usd,
        approximate: true,
        family_estimate: priced.confidence == RateConfidence::FamilyEstimate,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("relay usage ledger {path}: {source}")]
pub struct LedgerError {
    path: String,
    #[source]
    source: std::io::Error,
}

/// The append-only usage file. One `write_all` per line on an
/// `O_APPEND` handle, under a mutex, so concurrent requests never
/// interleave within a line.
pub struct Ledger {
    path: PathBuf,
    prices: PriceBook,
    lock: Mutex<()>,
}

impl Ledger {
    pub fn new(path: PathBuf, prices: PriceBook) -> Self {
        Self {
            path,
            prices,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(
        &self,
        rec: &OutcomeRecord,
        ts: DateTime<Utc>,
    ) -> Result<LedgerLine, LedgerError> {
        let line = LedgerLine::from_record(rec, ts, &self.prices);
        let err = |source| LedgerError {
            path: self.path.display().to_string(),
            source,
        };
        let mut text =
            serde_json::to_string(&line).map_err(|e| err(std::io::Error::other(e.to_string())))?;
        text.push('\n');
        let _g = self.lock.lock();
        // Device ids and usage are private to this user, not secret;
        // the file is still user-only like the rest of the data dir.
        crate::secure_perms::precreate_user_only(&self.path);
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)
            .map_err(err)?;
        // A crash mid-write leaves a tail with no newline; terminate it
        // so this line starts its own, rather than being glued to the
        // fragment and skipped with it.
        if !ends_with_newline_or_empty(&mut f).map_err(err)? {
            text.insert(0, '\n');
        }
        f.write_all(text.as_bytes()).map_err(err)?;
        Ok(line)
    }

    /// Every line, oldest first, collected. Prefer [`Self::for_each`]:
    /// nothing prunes the file, so a collected read grows without bound.
    pub fn read(path: &Path) -> Result<Vec<LedgerLine>, LedgerError> {
        let mut out = Vec::new();
        Self::for_each(path, |l| out.push(l))?;
        Ok(out)
    }

    /// Hand every line to `f`, oldest first, holding one line at a time:
    /// memory is bounded by [`MAX_LINE`], not by the file, which nothing
    /// prunes. Lines that do not parse are skipped — a line torn by a
    /// crash mid-write must not hide the rest of the file — and so is a
    /// line over [`MAX_LINE`], which no append writes. Read as bytes, not
    /// text: a tear inside a multibyte character would otherwise fail the
    /// UTF-8 check. Time is still linear in the file; this is a sync read
    /// for a caller already off the async runtime.
    pub fn for_each(path: &Path, mut f: impl FnMut(LedgerLine)) -> Result<(), LedgerError> {
        use std::io::BufRead as _;
        let err = |source| LedgerError {
            path: path.display().to_string(),
            source,
        };
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(err(source)),
        };
        let mut reader = std::io::BufReader::new(file);
        let mut line: Vec<u8> = Vec::new();
        let mut over = false;
        loop {
            let buf = match reader.fill_buf() {
                Ok(buf) => buf,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(source) => return Err(err(source)),
            };
            if buf.is_empty() {
                break;
            }
            let (part, end) = match buf.iter().position(|b| *b == b'\n') {
                Some(i) => (&buf[..i], Some(i + 1)),
                None => (buf, None),
            };
            if !over {
                if line.len() + part.len() > MAX_LINE {
                    over = true;
                    line = Vec::new();
                } else {
                    line.extend_from_slice(part);
                }
            }
            let used = end.unwrap_or(part.len());
            reader.consume(used);
            if end.is_some() {
                if !over {
                    if let Ok(l) = serde_json::from_slice(&line) {
                        f(l);
                    }
                }
                line.clear();
                over = false;
            }
        }
        if !over {
            if let Ok(l) = serde_json::from_slice(&line) {
                f(l);
            }
        }
        Ok(())
    }
}

/// Longest ledger line read back. An appended line is a few hundred
/// bytes; anything this long is not one, and is skipped unbuffered.
pub const MAX_LINE: usize = 64 * 1024;

fn ends_with_newline_or_empty(f: &mut std::fs::File) -> std::io::Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    if f.metadata()?.len() == 0 {
        return Ok(true);
    }
    f.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    f.read_exact(&mut last)?;
    Ok(last[0] == b'\n')
}

/// Samples kept per key, and how old one may be before it is dropped.
pub const MAX_SAMPLES: usize = 32;
pub const SAMPLE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    upstream: UpstreamId,
    model: String,
}

/// TTFB of completed inference, keyed by `(upstream, actual model)`.
#[derive(Default)]
pub struct Stats {
    samples: Mutex<HashMap<Key, VecDeque<(Instant, Duration)>>>,
}

/// One key's summary, as health reports it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LatencySummary {
    pub model: String,
    pub count: usize,
    pub median_ttfb_ms: u64,
    /// Age of the newest sample.
    pub newest_age_secs: u64,
}

impl Stats {
    /// Record a sample if it is one: a completed inference with a TTFB and
    /// a known model. A failure's TTFB measures how fast it failed.
    pub fn record(&self, rec: &OutcomeRecord, now: Instant) {
        let o = &rec.outcome;
        let (OutcomeKind::Complete, Some(ttfb), Some(model)) = (o.kind, o.ttfb, o.model.as_ref())
        else {
            return;
        };
        if rec.operation != super::upstream::Operation::Messages {
            return;
        }
        let mut map = self.samples.lock();
        let q = map
            .entry(Key {
                upstream: rec.target.id.clone(),
                model: model.clone(),
            })
            .or_default();
        q.push_back((now, ttfb));
        while q.len() > MAX_SAMPLES {
            q.pop_front();
        }
    }

    pub fn summary(&self, upstream: &UpstreamId, now: Instant) -> Vec<LatencySummary> {
        let mut map = self.samples.lock();
        map.retain(|_, q| {
            q.retain(|(at, _)| now.saturating_duration_since(*at) <= SAMPLE_TTL);
            !q.is_empty()
        });
        let mut out: Vec<_> = map
            .iter()
            .filter(|(k, _)| &k.upstream == upstream)
            .map(|(k, q)| {
                let mut ms: Vec<u64> = q.iter().map(|(_, d)| d.as_millis() as u64).collect();
                ms.sort_unstable();
                let newest = q.iter().map(|(at, _)| *at).max().unwrap_or(now);
                LatencySummary {
                    model: k.model.clone(),
                    count: ms.len(),
                    median_ttfb_ms: ms[ms.len() / 2],
                    newest_age_secs: now.saturating_duration_since(newest).as_secs(),
                }
            })
            .collect();
        out.sort_by(|a, b| a.model.cmp(&b.model));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::upstream::{Operation, Outcome, TargetIdentity};

    fn record(kind: TargetKind, model: Option<&str>, requested: Option<&str>) -> OutcomeRecord {
        OutcomeRecord {
            device_id: "dev1".into(),
            target: TargetIdentity {
                id: UpstreamId::parse("sub").unwrap(),
                fingerprint: "digest".into(),
                generation: 3,
                kind,
            },
            operation: Operation::Messages,
            requested_model: requested.map(str::to_owned),
            outcome: Outcome {
                kind: OutcomeKind::Complete,
                status: Some(200),
                ttfb: Some(Duration::from_millis(420)),
                upstream_request_id: Some("req_1".into()),
                bytes: 900,
                usage: Some(Usage {
                    input_tokens: 1_000_000,
                    output_tokens: 0,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                }),
                native_usage: None,
                model: model.map(str::to_owned),
                error: None,
            },
        }
    }

    fn ts() -> DateTime<Utc> {
        "2026-10-29T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn a_subscription_line_is_priced_approximately_with_both_models() {
        let r = record(
            TargetKind::Subscription,
            Some("claude-sonnet-4-5-20250929"),
            Some("claude-sonnet-4-5"),
        );
        let l = LedgerLine::from_record(&r, ts(), &PriceBook::bundled_only());
        assert_eq!(l.requested_model.as_deref(), Some("claude-sonnet-4-5"));
        assert_eq!(l.model.as_deref(), Some("claude-sonnet-4-5-20250929"));
        let c = l.cost.expect("an Anthropic model is priced");
        assert!(c.approximate);
        assert!(c.usd > 0.0);
        let u = l.usage.unwrap();
        assert_eq!(u.native.as_ref(), Some(&u.normalized));
        assert_eq!(l.ttfb_ms, Some(420));
    }

    #[test]
    fn a_route_upstream_is_unpriced_even_for_a_claude_name() {
        for kind in [TargetKind::PassThrough, TargetKind::Translated] {
            let r = record(kind, Some("claude-sonnet-4-5"), Some("sonnet"));
            let l = LedgerLine::from_record(&r, ts(), &PriceBook::bundled_only());
            assert!(l.cost.is_none(), "{kind:?} must not be priced as Claude");
        }
    }

    #[test]
    fn a_translated_line_never_claims_native_counters_it_did_not_get() {
        let r = record(TargetKind::Translated, Some("gpt-5"), Some("sonnet"));
        let l = LedgerLine::from_record(&r, ts(), &PriceBook::bundled_only());
        let u = l.usage.unwrap();
        assert!(u.native.is_none());
        assert_eq!(u.normalized.input_tokens, 1_000_000);
    }

    #[test]
    fn a_torn_tail_is_skipped_and_the_next_append_survives() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(USAGE_FILENAME);
        let ledger = Ledger::new(p.clone(), PriceBook::bundled_only());
        let r = record(TargetKind::Subscription, Some("claude-opus-4-1"), None);
        ledger.append(&r, ts()).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"{\"schema\":1,\"ts\":")
            .unwrap();
        ledger.append(&r, ts()).unwrap();
        let back = Ledger::read(&p).unwrap();
        assert_eq!(back.len(), 2, "only the torn fragment is lost");
        assert!(back.iter().all(|l| l.device == "dev1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn a_tail_torn_inside_a_multibyte_character_hides_only_itself() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(USAGE_FILENAME);
        let ledger = Ledger::new(p.clone(), PriceBook::bundled_only());
        let r = record(TargetKind::Subscription, Some("claude-opus-4-1"), None);
        ledger.append(&r, ts()).unwrap();
        // The first byte of a three-byte character, with the rest lost.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"{\"model\":\"\xe4")
            .unwrap();
        ledger.append(&r, ts()).unwrap();
        assert_eq!(Ledger::read(&p).unwrap().len(), 2);
    }

    #[test]
    fn concurrent_appends_never_interleave_within_a_line() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(USAGE_FILENAME);
        let ledger = std::sync::Arc::new(Ledger::new(p.clone(), PriceBook::bundled_only()));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let l = ledger.clone();
                std::thread::spawn(move || {
                    let r = record(TargetKind::Subscription, Some("claude-opus-4-1"), None);
                    for _ in 0..50 {
                        l.append(&r, ts()).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(Ledger::read(&p).unwrap().len(), 400);
    }

    #[test]
    fn an_oversized_line_is_skipped_and_the_lines_around_it_still_read() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(USAGE_FILENAME);
        let ledger = Ledger::new(p.clone(), PriceBook::bundled_only());
        let r = record(TargetKind::Subscription, Some("claude-opus-4-1"), None);
        let first = ledger.append(&r, ts()).unwrap();
        // Valid JSON, so only the cap can be what skips it.
        let mut huge = serde_json::to_string(&first).unwrap();
        huge.pop();
        huge.push_str(&format!(",\"pad\":\"{}\"}}\n", "x".repeat(MAX_LINE * 2)));
        assert!(serde_json::from_str::<LedgerLine>(huge.trim_end()).is_ok());
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(huge.as_bytes())
            .unwrap();
        ledger.append(&r, ts()).unwrap();
        let mut seen = 0;
        Ledger::for_each(&p, |l| {
            assert_eq!(l, first);
            seen += 1;
        })
        .unwrap();
        assert_eq!(
            seen, 2,
            "the oversized line is skipped, its neighbours are not"
        );
    }

    #[test]
    fn a_final_line_without_a_newline_still_reads() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(USAGE_FILENAME);
        let ledger = Ledger::new(p.clone(), PriceBook::bundled_only());
        let r = record(TargetKind::Subscription, Some("claude-opus-4-1"), None);
        ledger.append(&r, ts()).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, bytes.trim_ascii_end()).unwrap();
        assert_eq!(Ledger::read(&p).unwrap().len(), 1);
    }

    #[test]
    fn a_missing_ledger_reads_as_empty() {
        let d = tempfile::tempdir().unwrap();
        assert!(Ledger::read(&d.path().join("nope.jsonl"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn stats_keep_completed_inference_by_actual_model_and_age_out() {
        let s = Stats::default();
        let t0 = Instant::now();
        let mut r = record(TargetKind::Subscription, Some("m-a"), None);
        for ms in [100, 300, 200] {
            r.outcome.ttfb = Some(Duration::from_millis(ms));
            s.record(&r, t0);
        }
        let mut failed = r.clone();
        failed.outcome.kind = OutcomeKind::Failed;
        failed.outcome.ttfb = Some(Duration::from_millis(1));
        s.record(&failed, t0);
        let mut count = r.clone();
        count.operation = Operation::CountTokens;
        s.record(&count, t0);

        let id = UpstreamId::parse("sub").unwrap();
        let sum = s.summary(&id, t0 + Duration::from_secs(5));
        assert_eq!(sum.len(), 1);
        assert_eq!(
            (sum[0].count, sum[0].median_ttfb_ms, sum[0].newest_age_secs),
            (3, 200, 5)
        );
        assert!(s
            .summary(&id, t0 + SAMPLE_TTL + Duration::from_secs(1))
            .is_empty());
    }

    #[test]
    fn stats_are_bounded_per_key() {
        let s = Stats::default();
        let r = record(TargetKind::Subscription, Some("m"), None);
        let t0 = Instant::now();
        for _ in 0..(MAX_SAMPLES * 3) {
            s.record(&r, t0);
        }
        let id = UpstreamId::parse("sub").unwrap();
        assert_eq!(s.summary(&id, t0)[0].count, MAX_SAMPLES);
    }
}

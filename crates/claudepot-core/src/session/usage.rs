//! What an assistant message cost, read once per message.
//!
//! # One message, several lines
//!
//! Claude Code writes an API response as one transcript line **per
//! content block** — a thinking line, a text line, a tool-use line —
//! and every one of those lines repeats the message's full `usage`
//! under the same `message.id`. Summing `usage` per line therefore
//! counts a message once per block. It did, in every total Claudepot
//! kept: on the reference machine the stored session totals ran 2–6×
//! the real ones (one session recorded 16.1M output tokens against
//! 2.7M actually used), and every cost figure built on them inherited
//! the factor.
//!
//! [`UsageLedger`] charges a message once. Measured on 21,550 usage
//! lines, all 12,608 repeats carried usage identical to the message's
//! first line, so charging the first sighting is exact. A line with no
//! `message.id` is charged on its own, which is what the old code did
//! for every line.
//!
//! # What the usage block carries now
//!
//! Beyond the four token counts, 2.1.274's `usage` holds the parts that
//! change the bill, and this module reads each the way CC's own cost
//! function does:
//!
//! - `cache_creation.ephemeral_1h_input_tokens` — writes to the one-hour
//!   cache, billed at 2× input rather than the five-minute 1.25×. CC
//!   caps it at the total write count, and so does [`read_usage`]. On
//!   the reference machine 65% of all cache-write tokens were one-hour.
//! - `server_tool_use.web_search_requests` — billed per request.
//! - `speed: "fast"` — fast mode, a separate price table.
//! - `inference_geo: "us"` — US-only inference, 1.1× on token cost.

use super::TokenUsage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// Which non-standard price a message was billed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PremiumKind {
    Standard,
    /// `speed: "fast"`.
    Fast,
    /// `inference_geo: "us"`.
    Us,
    /// Both at once; the geo multiplier applies to the fast price.
    FastUs,
}

impl PremiumKind {
    /// Read from a message's `usage` block.
    pub fn of(usage: &Value) -> Self {
        let fast = usage.get("speed").and_then(Value::as_str) == Some("fast");
        let us = usage.get("inference_geo").and_then(Value::as_str) == Some("us");
        match (fast, us) {
            (false, false) => Self::Standard,
            (true, false) => Self::Fast,
            (false, true) => Self::Us,
            (true, true) => Self::FastUs,
        }
    }

    /// Stable column value; `None` for standard.
    pub fn as_column(self) -> Option<&'static str> {
        match self {
            Self::Standard => None,
            Self::Fast => Some("fast"),
            Self::Us => Some("us"),
            Self::FastUs => Some("fast_us"),
        }
    }

    pub fn from_column(s: Option<&str>) -> Self {
        match s {
            Some("fast") => Self::Fast,
            Some("us") => Self::Us,
            Some("fast_us") => Self::FastUs,
            _ => Self::Standard,
        }
    }
}

/// The part of a token total that was billed above the standard rate.
/// Each bucket is a subset of the total it travels with; whatever is in
/// none of them was billed at the standard rate.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PremiumUsage {
    #[serde(default, skip_serializing_if = "TokenUsage::is_zero")]
    pub fast: TokenUsage,
    #[serde(default, skip_serializing_if = "TokenUsage::is_zero")]
    pub us: TokenUsage,
    #[serde(default, skip_serializing_if = "TokenUsage::is_zero")]
    pub fast_us: TokenUsage,
}

impl PremiumUsage {
    pub fn is_empty(&self) -> bool {
        self.fast.is_zero() && self.us.is_zero() && self.fast_us.is_zero()
    }

    /// Add one message's tokens to the bucket its kind names.
    pub fn add(&mut self, kind: PremiumKind, tokens: &TokenUsage) {
        match kind {
            PremiumKind::Standard => {}
            PremiumKind::Fast => self.fast.add(tokens),
            PremiumKind::Us => self.us.add(tokens),
            PremiumKind::FastUs => self.fast_us.add(tokens),
        }
    }

    /// A premium holding `tokens` in the bucket `kind` names.
    pub fn single(kind: PremiumKind, tokens: &TokenUsage) -> Self {
        let mut p = Self::default();
        p.add(kind, tokens);
        p
    }

    /// Serialized for the index; `None` when there is nothing to store.
    pub fn to_column(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        serde_json::to_string(self).ok()
    }

    /// Read back from the index. An unreadable value is treated as no
    /// premium: the totals are still right, and the figure falls back
    /// to the standard rate rather than to nothing.
    pub fn from_column(s: Option<&str>) -> Self {
        s.and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default()
    }
}

/// Read the token counts of one `usage` block.
pub fn read_usage(usage: &Value) -> TokenUsage {
    let n = |v: Option<&Value>| v.and_then(Value::as_u64).unwrap_or(0);
    let cache_creation = n(usage.get("cache_creation_input_tokens"));
    TokenUsage {
        input: n(usage.get("input_tokens")),
        output: n(usage.get("output_tokens")),
        cache_creation,
        cache_read: n(usage.get("cache_read_input_tokens")),
        // CC: `Math.min(ephemeral_1h_input_tokens, cache_creation_input_tokens)`.
        cache_creation_1h: n(usage
            .get("cache_creation")
            .and_then(|c| c.get("ephemeral_1h_input_tokens")))
        .min(cache_creation),
        web_search_requests: n(usage
            .get("server_tool_use")
            .and_then(|s| s.get("web_search_requests"))),
    }
}

/// One message's usage, if its line carries any.
pub struct MessageUsage {
    pub tokens: TokenUsage,
    pub kind: PremiumKind,
}

pub fn read_message_usage(message: &Value) -> Option<MessageUsage> {
    let usage = message.get("usage")?;
    Some(MessageUsage {
        tokens: read_usage(usage),
        kind: PremiumKind::of(usage),
    })
}

/// Charges each API message once. See the module docs.
#[derive(Debug, Default)]
pub struct UsageLedger {
    charged: HashSet<String>,
}

impl UsageLedger {
    /// `true` the first time `message` is seen, and for every line that
    /// carries no `message.id`.
    pub fn first_sighting(&mut self, message: &Value) -> bool {
        match message.get("id").and_then(Value::as_str) {
            Some(id) => self.charged.insert(id.to_string()),
            None => true,
        }
    }

    /// Whether `message` has been charged, without charging it.
    pub fn is_charged(&self, message: &Value) -> bool {
        message
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| self.charged.contains(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_usage_block_reads_every_billed_part() {
        let u = json!({
            "input_tokens": 3,
            "output_tokens": 5,
            "cache_creation_input_tokens": 100,
            "cache_read_input_tokens": 7,
            "cache_creation": {"ephemeral_1h_input_tokens": 60, "ephemeral_5m_input_tokens": 40},
            "server_tool_use": {"web_search_requests": 2},
            "speed": "standard",
            "inference_geo": "not_available"
        });
        let t = read_usage(&u);
        assert_eq!(
            (
                t.input,
                t.output,
                t.cache_creation,
                t.cache_read,
                t.cache_creation_1h,
                t.web_search_requests
            ),
            (3, 5, 100, 7, 60, 2)
        );
        assert_eq!(PremiumKind::of(&u), PremiumKind::Standard);
    }

    #[test]
    fn one_hour_writes_are_capped_at_the_write_total() {
        // CC's own `Math.min`; a malformed block cannot bill more
        // one-hour tokens than were written.
        let u = json!({"cache_creation_input_tokens": 10, "cache_creation": {"ephemeral_1h_input_tokens": 99}});
        assert_eq!(read_usage(&u).cache_creation_1h, 10);
    }

    #[test]
    fn speed_and_geo_pick_the_premium_bucket() {
        assert_eq!(
            PremiumKind::of(&json!({"speed": "fast"})),
            PremiumKind::Fast
        );
        assert_eq!(
            PremiumKind::of(&json!({"inference_geo": "us"})),
            PremiumKind::Us
        );
        assert_eq!(
            PremiumKind::of(&json!({"speed": "fast", "inference_geo": "us"})),
            PremiumKind::FastUs
        );
        for kind in [
            PremiumKind::Standard,
            PremiumKind::Fast,
            PremiumKind::Us,
            PremiumKind::FastUs,
        ] {
            assert_eq!(PremiumKind::from_column(kind.as_column()), kind);
        }
    }

    #[test]
    fn a_message_is_charged_once_however_many_lines_repeat_it() {
        let mut ledger = UsageLedger::default();
        let m = json!({"id": "msg_1"});
        assert!(ledger.first_sighting(&m));
        assert!(ledger.is_charged(&m));
        assert!(!ledger.first_sighting(&m));
        // No id: nothing to deduplicate by, so each line counts.
        let anonymous = json!({});
        assert!(ledger.first_sighting(&anonymous));
        assert!(ledger.first_sighting(&anonymous));
        assert!(!ledger.is_charged(&anonymous));
    }

    #[test]
    fn premium_round_trips_through_its_column() {
        let t = TokenUsage {
            output: 4,
            ..TokenUsage::default()
        };
        let p = PremiumUsage::single(PremiumKind::Fast, &t);
        assert_eq!(PremiumUsage::from_column(p.to_column().as_deref()), p);
        assert_eq!(PremiumUsage::default().to_column(), None);
        assert_eq!(
            PremiumUsage::from_column(Some("not json")),
            PremiumUsage::default()
        );
    }
}

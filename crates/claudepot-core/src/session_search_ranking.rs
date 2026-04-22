//! Scoring + stable ranking primitives for session search.
//!
//! Kept in its own module so `session_search.rs` stays under the
//! 350-line production budget and so the scoring rules are easy to
//! find. These primitives are pure — no I/O, no Tauri, no `SessionRow`
//! dependency — which also keeps tests trivial.

use std::cmp::Ordering;

use crate::session_search::SearchHit;

/// Exact phrase: match is bounded by non-alphanumeric chars on both
/// sides of the haystack (start/end of string counts as a boundary).
pub(crate) const SCORE_PHRASE: f32 = 1.0;
/// Word-prefix: match starts at a word boundary but ends inside a word.
pub(crate) const SCORE_PREFIX: f32 = 0.7;
/// Pure substring: match is embedded inside a word on at least one side.
pub(crate) const SCORE_SUBSTRING: f32 = 0.4;

/// Classify a match at `byte_off` in `haystack` with matched byte
/// length `byte_len`. "Word boundary" is Unicode `is_alphanumeric`, so
/// `café` counts as one word and `·` (middle dot) is a separator.
pub(crate) fn classify_match(haystack: &str, byte_off: usize, byte_len: usize) -> f32 {
    let before_is_boundary = match haystack[..byte_off].chars().last() {
        None => true,
        Some(c) => !c.is_alphanumeric(),
    };
    let after_off = byte_off + byte_len;
    let after_is_boundary = match haystack[after_off..].chars().next() {
        None => true,
        Some(c) => !c.is_alphanumeric(),
    };
    match (before_is_boundary, after_is_boundary) {
        (true, true) => SCORE_PHRASE,
        (true, false) => SCORE_PREFIX,
        _ => SCORE_SUBSTRING,
    }
}

/// Stable sort hits by `score` desc, then `last_ts` desc. `None` ts
/// sorts last (treated as oldest). Stable so equal keys preserve the
/// caller's input order — callers that want recency-before-score can
/// pre-sort then rely on stability.
pub fn rank_hits(mut hits: Vec<SearchHit>) -> Vec<SearchHit> {
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.last_ts.cmp(&a.last_ts))
    });
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::path::PathBuf;

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(s.parse::<DateTime<Utc>>().unwrap())
    }

    fn synthetic(score: f32, last_ts: Option<DateTime<Utc>>, id: &str) -> SearchHit {
        SearchHit {
            session_id: id.into(),
            slug: "-r".into(),
            file_path: PathBuf::from("/x"),
            project_path: "/repo".into(),
            role: "user".into(),
            snippet: String::new(),
            match_offset: 0,
            last_ts,
            score,
        }
    }

    #[test]
    fn classify_phrase_prefix_substring() {
        assert_eq!(classify_match("auth is cool", 0, 4), SCORE_PHRASE);
        assert_eq!(classify_match("authentication rules", 0, 4), SCORE_PREFIX);
        assert_eq!(classify_match("unauthorized now", 2, 4), SCORE_SUBSTRING);
        assert_eq!(classify_match("(auth).", 1, 4), SCORE_PHRASE);
        assert_eq!(classify_match("no auth", 3, 4), SCORE_PHRASE);
    }

    #[test]
    fn classify_unicode_word_chars() {
        assert_eq!(classify_match("café latte", 0, 3), SCORE_PREFIX);
        assert_eq!(classify_match("cafés wars", 1, 4), SCORE_SUBSTRING);
        assert_eq!(classify_match("a·auth·z", 3, 4), SCORE_PHRASE);
    }

    #[test]
    fn rank_sorts_by_score_then_recency() {
        let new_ts = ts("2026-04-10T10:00:00Z");
        let old_ts = ts("2020-01-01T00:00:00Z");
        let hits = vec![
            synthetic(0.4, old_ts, "sub-old"),
            synthetic(1.0, old_ts, "phrase-old"),
            synthetic(1.0, new_ts, "phrase-new"),
            synthetic(0.7, new_ts, "prefix-new"),
        ];
        let ranked = rank_hits(hits);
        let ids: Vec<_> = ranked.iter().map(|h| h.session_id.as_str()).collect();
        assert_eq!(ids, vec!["phrase-new", "phrase-old", "prefix-new", "sub-old"]);
    }

    #[test]
    fn rank_is_stable_for_equal_keys() {
        let t = ts("2026-04-10T10:00:00Z");
        let hits = vec![
            synthetic(1.0, t, "a"),
            synthetic(1.0, t, "b"),
            synthetic(1.0, t, "c"),
        ];
        let ranked = rank_hits(hits);
        let ids: Vec<_> = ranked.iter().map(|h| h.session_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn rank_puts_none_ts_last() {
        let t = ts("2026-04-10T10:00:00Z");
        let hits = vec![synthetic(1.0, None, "none"), synthetic(1.0, t, "has-ts")];
        let ranked = rank_hits(hits);
        let ids: Vec<_> = ranked.iter().map(|h| h.session_id.as_str()).collect();
        assert_eq!(ids, vec!["has-ts", "none"]);
    }

    #[test]
    fn rank_empty_is_noop() {
        assert!(rank_hits(Vec::<SearchHit>::new()).is_empty());
    }
}

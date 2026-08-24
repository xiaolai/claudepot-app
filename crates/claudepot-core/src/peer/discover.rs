//! Finding and naming the sessions a caller can address.
//!
//! `send_prompt` needs a `PeerTarget`, and the only handle a human has
//! is whatever they can see: a pid, a session id, or the short name CC
//! derives from the working directory (`ccspace-96`). This module turns
//! any of those into exactly one session, or refuses.
//!
//! The resolution rule is deliberately the same one account resolution
//! uses: **exactly one match wins; zero or several is an error.** No
//! fuzzy matching, no edit distance, no "did you mean". A prompt sent to
//! the wrong session is not recoverable by apologising afterwards.

use std::path::Path;

use super::PeerError;
use crate::session_live::registry::{poll_dir, SysinfoCheck};
use crate::session_live::types::PidRecord;

/// One addressable session, as shown to a human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addressable {
    pub record: PidRecord,
}

impl Addressable {
    /// The short label CC derives from the cwd, when it has one.
    pub fn name(&self) -> Option<&str> {
        self.record.name.as_deref()
    }

    /// First 8 characters of the session id — enough to identify one
    /// session among a handful, and what a human will actually type.
    pub fn short_id(&self) -> &str {
        let id = &self.record.session_id;
        id.split_once('-').map_or(id.as_str(), |(head, _)| head)
    }
}

/// Every live session that currently exposes an inbox.
///
/// A session without `messagingSocketPath` is filtered out here rather
/// than surfaced and rejected later: it is not addressable, so listing
/// it as a candidate would invite the user to type a name that can
/// never work.
pub fn list_addressable(sessions_dir: &Path) -> Result<Vec<Addressable>, PeerError> {
    let check = SysinfoCheck::new();
    let outcome =
        poll_dir(sessions_dir, &check).map_err(|source| PeerError::RegistryUnreadable {
            path: sessions_dir.display().to_string(),
            source,
        })?;

    let mut out: Vec<Addressable> = outcome
        .live
        .into_iter()
        .filter(|r| {
            r.messaging_socket_path
                .as_deref()
                .is_some_and(|p| !p.is_empty())
        })
        .map(|record| Addressable { record })
        .collect();
    // Stable, human-meaningful order: newest session first.
    out.sort_by(|a, b| b.record.started_at_ms.cmp(&a.record.started_at_ms));
    Ok(out)
}

/// Resolve a user-typed needle to exactly one session.
///
/// Pure so the precedence rules are testable without live processes.
///
/// An exact pid match wins outright and short-circuits: a pid is
/// unambiguous by construction, and letting it compete with prefix
/// matches would let a session *named* `1234…` shadow the process
/// actually numbered 1234.
pub fn resolve<'a>(
    candidates: &'a [Addressable],
    needle: &str,
) -> Result<&'a Addressable, PeerError> {
    let needle = needle.trim();
    if needle.is_empty() {
        return Err(PeerError::NoSuchSession {
            needle: needle.to_string(),
            candidates: describe(candidates),
        });
    }

    if let Ok(pid) = needle.parse::<u32>() {
        if let Some(hit) = candidates.iter().find(|c| c.record.pid == pid) {
            return Ok(hit);
        }
    }

    let lowered = needle.to_ascii_lowercase();
    let matches: Vec<&Addressable> = candidates
        .iter()
        .filter(|c| {
            c.record
                .session_id
                .to_ascii_lowercase()
                .starts_with(&lowered)
                || c.name()
                    .is_some_and(|n| n.to_ascii_lowercase().starts_with(&lowered))
        })
        .collect();

    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(PeerError::NoSuchSession {
            needle: needle.to_string(),
            candidates: describe(candidates),
        }),
        _ => Err(PeerError::AmbiguousSession {
            needle: needle.to_string(),
            matches: describe_refs(&matches),
        }),
    }
}

fn describe(items: &[Addressable]) -> Vec<String> {
    items.iter().map(describe_one).collect()
}

fn describe_refs(items: &[&Addressable]) -> Vec<String> {
    items.iter().map(|a| describe_one(a)).collect()
}

fn describe_one(a: &Addressable) -> String {
    match a.name() {
        Some(name) => format!("{} ({}, pid {})", name, a.short_id(), a.record.pid),
        None => format!("{} (pid {})", a.short_id(), a.record.pid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(pid: u32, session_id: &str, name: Option<&str>, started: i64) -> Addressable {
        Addressable {
            record: PidRecord {
                pid,
                session_id: session_id.into(),
                cwd: "/tmp".into(),
                started_at_ms: started,
                updated_at_ms: None,
                version: None,
                kind: Some("interactive".into()),
                entrypoint: None,
                name: name.map(str::to_string),
                status: Some("idle".into()),
                waiting_for: None,
                messaging_socket_path: Some(format!("/tmp/cc-socks/{pid}.sock")),
                peer_protocol: Some(1),
            },
        }
    }

    fn fixture() -> Vec<Addressable> {
        vec![
            mk(
                101,
                "aaaa1111-0000-0000-0000-000000000000",
                Some("ccspace-96"),
                3,
            ),
            mk(
                202,
                "bbbb2222-0000-0000-0000-000000000000",
                Some("vmark-b1"),
                2,
            ),
            mk(
                303,
                "aaaa3333-0000-0000-0000-000000000000",
                Some("vmark-c2"),
                1,
            ),
        ]
    }

    #[test]
    fn resolves_an_exact_pid() {
        let c = fixture();
        assert_eq!(resolve(&c, "202").unwrap().record.pid, 202);
    }

    #[test]
    fn an_exact_pid_beats_a_name_that_starts_with_the_same_digits() {
        let mut c = fixture();
        c.push(mk(
            999,
            "cccc4444-0000-0000-0000-000000000000",
            Some("202-report"),
            0,
        ));
        // Without the short-circuit this is ambiguous; with it the
        // process actually numbered 202 wins.
        assert_eq!(resolve(&c, "202").unwrap().record.pid, 202);
    }

    #[test]
    fn resolves_a_unique_name_prefix() {
        let c = fixture();
        assert_eq!(resolve(&c, "ccspace").unwrap().record.pid, 101);
    }

    #[test]
    fn resolves_a_unique_session_id_prefix() {
        let c = fixture();
        assert_eq!(resolve(&c, "bbbb").unwrap().record.pid, 202);
    }

    #[test]
    fn name_matching_ignores_case() {
        let c = fixture();
        assert_eq!(resolve(&c, "CCSpace").unwrap().record.pid, 101);
    }

    #[test]
    fn an_ambiguous_prefix_is_refused_and_lists_the_matches() {
        let c = fixture();
        let err = resolve(&c, "vmark").unwrap_err();
        let PeerError::AmbiguousSession { matches, .. } = &err else {
            panic!("expected AmbiguousSession, got {err:?}");
        };
        assert_eq!(matches.len(), 2);
        assert!(err.to_string().contains("vmark"));
    }

    #[test]
    fn an_ambiguous_session_id_prefix_is_refused() {
        // Two ids share the `aaaa` prefix — a real hazard, since short
        // ids are what a human copies off a listing.
        let c = fixture();
        assert!(matches!(
            resolve(&c, "aaaa").unwrap_err(),
            PeerError::AmbiguousSession { .. }
        ));
    }

    #[test]
    fn no_match_lists_what_was_available() {
        let c = fixture();
        let err = resolve(&c, "nope").unwrap_err();
        let PeerError::NoSuchSession { candidates, .. } = &err else {
            panic!("expected NoSuchSession, got {err:?}");
        };
        assert_eq!(candidates.len(), 3, "the error must be the discovery path");
    }

    #[test]
    fn an_empty_needle_never_matches_everything() {
        let c = fixture();
        // An empty string is a prefix of every id, so a naive
        // implementation resolves it to "ambiguous" at best and to the
        // first session at worst.
        assert!(matches!(
            resolve(&c, "  ").unwrap_err(),
            PeerError::NoSuchSession { .. }
        ));
    }

    #[test]
    fn short_id_is_the_first_uuid_group() {
        let c = fixture();
        assert_eq!(c[0].short_id(), "aaaa1111");
    }
}

//! The one parser from request line to [`Target`].
//!
//! Everything downstream — the allowlist, the grant check, dispatch —
//! consumes the value this returns, so a request cannot be authorized as
//! one upstream and dispatched to another.
//!
//! ## What is accepted
//!
//! Pinned by hand from what installed Claude Code 2.1.312 was observed
//! calling (P0, `dev-docs/spikes/cc-proxy/p0/FINDINGS.md`), not widened
//! automatically:
//!
//! | Request | Meaning |
//! |---|---|
//! | `HEAD /api/hello` | CC's connectivity probe; answered without auth |
//! | `POST /v1/messages[?beta=true]` | inference |
//! | `POST /v1/messages/count_tokens[?beta=true]` | token count (CC calls it for sizing checks) |
//!
//! Each may carry a `/u/<upstream-id>` prefix — installed CC keeps a
//! base-URL path on every call, with and without a trailing slash. Bare
//! means the server's subscription upstream. Control paths
//! (`/relay/<name>`) exist only at the server origin.
//!
//! ## What is refused
//!
//! Absolute-form request targets, any percent-encoding, empty segments
//! (doubled or trailing slashes), `.` / `..` segments, backslashes, and
//! any query other than `beta=true`. Refusing encodings outright rather
//! than decoding them is the point: a decoder is where `/u/a%2F..` turns
//! into a different upstream than the one that was checked.

use axum::http::{Method, Uri};

/// An upstream id as it appears in `/u/<id>`: lowercase ASCII letters,
/// digits and `-`, 1–64 characters, starting with a letter or digit.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct UpstreamId(String);

impl UpstreamId {
    pub fn parse(s: &str) -> Option<Self> {
        let ok = !s.is_empty()
            && s.len() <= 64
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !s.starts_with('-');
        ok.then(|| Self(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for UpstreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for UpstreamId {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Self::parse(&s).ok_or_else(|| format!("invalid upstream id {s:?}"))
    }
}

impl From<UpstreamId> for String {
    fn from(id: UpstreamId) -> String {
        id.0
    }
}

/// Which upstream the request names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamSel {
    /// Bare `/v1/…`: the server's subscription upstream.
    Default,
    Named(UpstreamId),
}

/// An inference-surface operation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Messages,
    CountTokens,
}

impl Operation {
    /// The upstream path, without the `/u/<id>` prefix.
    pub fn path(self) -> &'static str {
        match self {
            Operation::Messages => "/v1/messages",
            Operation::CountTokens => "/v1/messages/count_tokens",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub upstream: UpstreamSel,
    pub operation: Operation,
    /// `?beta=true` was present; forwarded as-is.
    pub beta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// `HEAD /api/hello`, bare or prefixed. No credential is presented.
    Hello,
    Api(Target),
    /// `/relay/<name>` at the server origin.
    Control(String),
}

/// Every refusal is a 404: the path is not something this server
/// serves, and the upstream is never contacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotFound;

pub fn parse(method: &Method, uri: &Uri) -> Result<Parsed, NotFound> {
    if uri.scheme().is_some() || uri.authority().is_some() {
        return Err(NotFound);
    }
    let path = uri.path();
    if !path.starts_with('/') || path.contains('%') || path.contains('\\') {
        return Err(NotFound);
    }
    let beta = match uri.query() {
        None => false,
        Some("beta=true") => true,
        Some(_) => return Err(NotFound),
    };

    let segments: Vec<&str> = path[1..].split('/').collect();
    if segments
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == "..")
    {
        return Err(NotFound);
    }

    let (upstream, rest) = match segments.as_slice() {
        ["u", id, rest @ ..] => {
            let id = UpstreamId::parse(id).ok_or(NotFound)?;
            (UpstreamSel::Named(id), rest)
        }
        ["relay", name] => {
            let valid = name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-');
            return if valid && !beta {
                Ok(Parsed::Control((*name).to_owned()))
            } else {
                Err(NotFound)
            };
        }
        rest => (UpstreamSel::Default, rest),
    };

    match (method, rest) {
        (&Method::HEAD, ["api", "hello"]) if !beta => Ok(Parsed::Hello),
        (&Method::POST, ["v1", "messages"]) => Ok(Parsed::Api(Target {
            upstream,
            operation: Operation::Messages,
            beta,
        })),
        (&Method::POST, ["v1", "messages", "count_tokens"]) => Ok(Parsed::Api(Target {
            upstream,
            operation: Operation::CountTokens,
            beta,
        })),
        _ => Err(NotFound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(method: Method, target: &str) -> Result<Parsed, NotFound> {
        parse(&method, &target.parse::<Uri>().unwrap())
    }

    fn named(id: &str, operation: Operation, beta: bool) -> Parsed {
        Parsed::Api(Target {
            upstream: UpstreamSel::Named(UpstreamId::parse(id).unwrap()),
            operation,
            beta,
        })
    }

    #[test]
    fn the_calls_installed_cc_makes_are_accepted_bare_and_prefixed() {
        assert_eq!(p(Method::HEAD, "/api/hello"), Ok(Parsed::Hello));
        assert_eq!(p(Method::HEAD, "/u/sub/api/hello"), Ok(Parsed::Hello));
        assert_eq!(
            p(Method::POST, "/v1/messages?beta=true"),
            Ok(Parsed::Api(Target {
                upstream: UpstreamSel::Default,
                operation: Operation::Messages,
                beta: true
            }))
        );
        assert_eq!(
            p(Method::POST, "/u/deepseek/v1/messages?beta=true"),
            Ok(named("deepseek", Operation::Messages, true))
        );
        assert_eq!(
            p(Method::POST, "/u/sub/v1/messages/count_tokens"),
            Ok(named("sub", Operation::CountTokens, false))
        );
    }

    #[test]
    fn control_paths_exist_only_at_the_server_origin() {
        assert_eq!(
            p(Method::GET, "/relay/health"),
            Ok(Parsed::Control("health".into()))
        );
        assert_eq!(p(Method::GET, "/u/sub/relay/health"), Err(NotFound));
        assert_eq!(p(Method::GET, "/relay/health/x"), Err(NotFound));
    }

    #[test]
    fn malformed_targets_are_refused() {
        for t in [
            "//v1/messages",
            "/v1//messages",
            "/v1/messages/",
            "/u//v1/messages",
            "/u/sub//v1/messages",
            "/u/sub/./v1/messages",
            "/u/sub/../v1/messages",
            "/v1/./messages",
            "/u/a%2Fb/v1/messages",
            "/u/%73ub/v1/messages",
            "/v1/messages%3Fbeta=true",
            "/u/Sub/v1/messages",
            "/u/-x/v1/messages",
            "/v1/messages?beta=false",
            "/v1/messages?beta=true&x=1",
            "/v1/messages?",
            "/v1/complete",
            "/v1/models",
            "/u/sub/v1/messages/batches",
        ] {
            assert_eq!(p(Method::POST, t), Err(NotFound), "{t}");
        }
    }

    #[test]
    fn absolute_form_targets_are_refused() {
        assert_eq!(
            p(Method::POST, "http://evil.example/v1/messages"),
            Err(NotFound)
        );
    }

    #[test]
    fn the_method_is_part_of_the_allowlist() {
        assert_eq!(p(Method::GET, "/v1/messages"), Err(NotFound));
        assert_eq!(p(Method::GET, "/api/hello"), Err(NotFound));
        assert_eq!(p(Method::POST, "/api/hello"), Err(NotFound));
        assert_eq!(p(Method::HEAD, "/api/hello?beta=true"), Err(NotFound));
    }

    #[test]
    fn upstream_ids_are_a_narrow_alphabet() {
        assert!(UpstreamId::parse("deepseek-v3").is_some());
        assert!(UpstreamId::parse("").is_none());
        assert!(UpstreamId::parse("a_b").is_none());
        assert!(UpstreamId::parse(&"a".repeat(65)).is_none());
    }
}

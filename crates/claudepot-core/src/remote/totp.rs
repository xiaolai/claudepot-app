//! RFC 6238 TOTP — the optional second factor for the remote surface.
//!
//! Optional on purpose. If the client is the phone and the authenticator
//! app is on the same phone, a second factor is close to ceremonial: one
//! device compromise defeats both. What it genuinely defends is a
//! credential used from *somewhere else* — a cracked password hash, a
//! shoulder-surfed login. Worth offering, not worth forcing.
//!
//! ## Why this is a second factor and never the only one
//!
//! **A TOTP secret cannot be hashed.** The server has to store it in
//! recoverable form, because it must compute the expected code. The
//! password next door is stored as a one-way scrypt hash, so reading
//! that file yields a cracking job; reading a TOTP secret yields
//! permanent working access with no work at all.
//!
//! Using TOTP *instead of* the password would therefore make the stored
//! credential strictly more valuable to whoever reads the file. It also
//! has no recovery story — a lost phone locks the owner out — and the
//! usual remedy, printed backup codes, is a password again with worse
//! ergonomics.
//!
//! ## SHA-1 is correct here
//!
//! RFC 6238's default, and the only algorithm Google Authenticator reads
//! from a plain `otpauth://` URI. It is used inside HMAC, where SHA-1's
//! collision weakness does not apply. Choosing SHA-256 would be
//! defensible cryptographically and would silently break enrolment in
//! the most common authenticator app.

use hmac::{Hmac, Mac};
use sha1::Sha1;

use super::ct::constant_time_eq;
use super::token;

type HmacSha1 = Hmac<Sha1>;

/// Seconds per code. RFC 6238's default and what every authenticator
/// assumes when the URI omits it.
pub const PERIOD_SECS: u64 = 30;

/// Digits in a code.
pub const DIGITS: u32 = 6;

/// How many periods either side of "now" are accepted.
///
/// One step covers ordinary clock drift between a phone and a laptop.
/// It also means a code is live for up to 90 seconds, which is exactly
/// why [`TotpState::consume`] burns it — see there.
pub const SKEW_STEPS: u64 = 1;

/// 160 bits, RFC 4226's recommendation and what authenticator apps
/// expect.
const SECRET_LEN: usize = 20;

/// An enrolment secret. Never `Debug`-printed in full: this is the
/// entire second factor, and a derived `Debug` on a secret-bearing
/// struct has already been a bug once in this module tree.
#[derive(Clone, PartialEq, Eq)]
pub struct TotpSecret(Vec<u8>);

impl std::fmt::Debug for TotpSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TotpSecret(<redacted, {} bytes>)", self.0.len())
    }
}

impl TotpSecret {
    /// Uses `token::random_bytes`, which discards the UUID bytes whose
    /// bits are fixed. Slicing a UUID directly would reintroduce the
    /// version-nibble defect that cost the pairing code a symbol
    /// position — in a module where the loss would be silent.
    pub fn generate() -> Self {
        Self(token::random_bytes(SECRET_LEN))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Base32, unpadded — the encoding `otpauth://` URIs use.
    pub fn to_base32(&self) -> String {
        base32_encode(&self.0)
    }

    /// Parse a stored secret, refusing anything that is not exactly
    /// [`SECRET_LEN`] bytes.
    ///
    /// Length is the contract, not a formality. HMAC-SHA1 accepts a key
    /// of any length — including an empty one — so a truncated or
    /// blanked `totp_secret_base32` does not fail here, it produces a
    /// *working* second factor built on a guessable key. Refusing it
    /// makes a damaged secret look like what it is (see
    /// `config::auth`), instead of like a weaker one nobody noticed.
    pub fn from_base32(s: &str) -> Option<Self> {
        let bytes = base32_decode(s)?;
        (bytes.len() == SECRET_LEN).then_some(Self(bytes))
    }

    /// The URI an authenticator app scans.
    ///
    /// `issuer` appears twice by design: once as a label prefix for apps
    /// that only read the label, once as a parameter for apps that read
    /// parameters. Omitting either makes the entry show up unlabelled in
    /// one popular client or another.
    pub fn provisioning_uri(&self, issuer: &str, account: &str) -> String {
        // Percent-encoding is defined over BYTES. Encoding the code
        // point instead produced `%E9` for `é` (which decodes to a
        // stray Latin-1 byte) and `%1F600` for an emoji, which is not
        // a percent-escape at all — the URI simply fails to parse in
        // the authenticator app. The account label is user-supplied, so
        // this is reachable.
        let enc = |s: &str| {
            let mut out = String::with_capacity(s.len());
            for b in s.bytes() {
                match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        out.push(b as char)
                    }
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            out
        };
        format!(
            "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
            enc(issuer),
            enc(account),
            self.to_base32(),
            enc(issuer),
            DIGITS,
            PERIOD_SECS
        )
    }
}

/// The code for a given counter (`unix_seconds / PERIOD_SECS`).
pub fn code_at_counter(secret: &TotpSecret, counter: u64) -> String {
    // `.claude/rules/rust-conventions.md`: never `expect` in core. HMAC
    // does accept any key length, so the old `.expect` was unreachable
    // — but "unreachable today" is a property of the current key type,
    // and the rule exists so nobody has to re-derive that.
    let Ok(mut mac) = HmacSha1::new_from_slice(secret.as_bytes()) else {
        // Cannot happen for any `TotpSecret`, which is fixed-width by
        // construction. A code that can never match is the right
        // failure shape here: `consume` compares in constant time and
        // will simply reject.
        return "0".repeat(DIGITS as usize);
    };
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();

    // RFC 4226 dynamic truncation.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((digest[offset] & 0x7f) as u32) << 24
        | (digest[offset + 1] as u32) << 16
        | (digest[offset + 2] as u32) << 8
        | (digest[offset + 3] as u32);
    let modulus = 10u32.pow(DIGITS);
    format!("{:0width$}", binary % modulus, width = DIGITS as usize)
}

pub fn counter_at(unix_secs: u64) -> u64 {
    unix_secs / PERIOD_SECS
}

/// Per-user TOTP state. `last_used_counter` is what makes a code
/// single-use.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TotpState {
    pub last_used_counter: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotpVerdict {
    Accepted {
        counter: u64,
    },
    /// The code did not match any counter in the accepted window.
    Wrong,
    /// The code matched, but that counter has already been spent.
    ///
    /// This is a distinct verdict rather than folded into `Wrong`
    /// because it is the signature of a **replay**: someone is
    /// presenting a code that was legitimately valid. A caller may
    /// reasonably treat it more harshly than a typo.
    Replayed {
        counter: u64,
    },
}

impl TotpState {
    /// Verify and, on success, burn the counter.
    ///
    /// Without burning, a code stays valid for up to
    /// `(2 * SKEW_STEPS + 1) * PERIOD_SECS` — 90 seconds by default —
    /// and anyone who observes one can replay it for the rest of that
    /// window. RFC 6238 §5.2 requires exactly this and it is the part
    /// most implementations leave out.
    pub fn consume(&mut self, secret: &TotpSecret, candidate: &str, unix_secs: u64) -> TotpVerdict {
        let now = counter_at(unix_secs);
        let lo = now.saturating_sub(SKEW_STEPS);
        let hi = now.saturating_add(SKEW_STEPS);

        let mut matched: Option<u64> = None;
        for counter in lo..=hi {
            // Compared in constant time, and every candidate in the
            // window is evaluated: returning early on a match would leak
            // which counter matched through timing.
            if constant_time_eq(
                candidate.as_bytes(),
                code_at_counter(secret, counter).as_bytes(),
            ) && matched.is_none()
            {
                matched = Some(counter);
            }
        }

        let Some(counter) = matched else {
            return TotpVerdict::Wrong;
        };
        if self.last_used_counter.is_some_and(|last| counter <= last) {
            return TotpVerdict::Replayed { counter };
        }
        self.last_used_counter = Some(counter);
        TotpVerdict::Accepted { counter }
    }
}

const B32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for byte in data {
        buffer = (buffer << 8) | *byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(B32[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B32[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.chars().filter(|c| *c != '=' && !c.is_whitespace()) {
        let v = B32
            .iter()
            .position(|b| *b as char == c.to_ascii_uppercase())? as u32;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 Appendix B, SHA-1 rows. The ASCII secret is
    /// "12345678901234567890"; the reference vectors use 8 digits, so
    /// the expected values here are the low 6 of each.
    fn rfc_secret() -> TotpSecret {
        TotpSecret::from_bytes(b"12345678901234567890".to_vec())
    }

    #[test]
    fn matches_the_rfc_6238_test_vectors() {
        // (unix time, 8-digit reference from the RFC)
        for (t, eight) in [
            (59u64, "94287082"),
            (1111111109, "07081804"),
            (1111111111, "14050471"),
            (1234567890, "89005924"),
            (2000000000, "69279037"),
            (20000000000, "65353130"),
        ] {
            let want = &eight[eight.len() - DIGITS as usize..];
            let got = code_at_counter(&rfc_secret(), counter_at(t));
            assert_eq!(got, want, "t={t}");
        }
    }

    #[test]
    fn a_code_is_six_digits() {
        let s = TotpSecret::generate();
        for t in [0u64, 1, 59, 1_700_000_000] {
            let c = code_at_counter(&s, counter_at(t));
            assert_eq!(c.len(), DIGITS as usize);
            assert!(c.chars().all(|ch| ch.is_ascii_digit()), "{c}");
        }
    }

    #[test]
    fn the_current_code_verifies() {
        let s = TotpSecret::generate();
        let now = 1_700_000_000;
        let code = code_at_counter(&s, counter_at(now));
        let mut st = TotpState::default();
        assert!(matches!(
            st.consume(&s, &code, now),
            TotpVerdict::Accepted { .. }
        ));
    }

    #[test]
    fn one_step_of_drift_either_way_is_tolerated() {
        let s = TotpSecret::generate();
        let now = 1_700_000_000;
        for offset in [-(PERIOD_SECS as i64), PERIOD_SECS as i64] {
            let code = code_at_counter(&s, counter_at((now as i64 + offset) as u64));
            let mut st = TotpState::default();
            assert!(
                matches!(st.consume(&s, &code, now), TotpVerdict::Accepted { .. }),
                "offset {offset}s should be inside the skew window"
            );
        }
    }

    #[test]
    fn two_steps_of_drift_is_rejected() {
        let s = TotpSecret::generate();
        let now = 1_700_000_000;
        let far = code_at_counter(&s, counter_at(now + PERIOD_SECS * 3));
        let mut st = TotpState::default();
        assert_eq!(st.consume(&s, &far, now), TotpVerdict::Wrong);
    }

    #[test]
    fn a_used_code_cannot_be_replayed() {
        // Without burning, a code stays valid for up to 90s and anyone
        // who sees it can reuse it. This is the assertion that matters.
        let s = TotpSecret::generate();
        let now = 1_700_000_000;
        let code = code_at_counter(&s, counter_at(now));
        let mut st = TotpState::default();
        assert!(matches!(
            st.consume(&s, &code, now),
            TotpVerdict::Accepted { .. }
        ));
        assert!(
            matches!(st.consume(&s, &code, now), TotpVerdict::Replayed { .. }),
            "the same code must not work twice"
        );
    }

    #[test]
    fn an_earlier_still_in_window_code_cannot_be_used_after_a_later_one() {
        // Burning must be a high-water mark, not a set of one: accepting
        // the previous counter after the current one would leave a
        // 30-second replay hole.
        let s = TotpSecret::generate();
        let now = 1_700_000_000;
        let previous = code_at_counter(&s, counter_at(now) - 1);
        let current = code_at_counter(&s, counter_at(now));
        let mut st = TotpState::default();
        assert!(matches!(
            st.consume(&s, &current, now),
            TotpVerdict::Accepted { .. }
        ));
        assert!(matches!(
            st.consume(&s, &previous, now),
            TotpVerdict::Replayed { .. }
        ));
    }

    #[test]
    fn a_wrong_code_is_wrong_and_burns_nothing() {
        let s = TotpSecret::generate();
        let now = 1_700_000_000;
        let mut st = TotpState::default();
        assert_eq!(st.consume(&s, "000000", now), TotpVerdict::Wrong);
        assert_eq!(
            st.last_used_counter, None,
            "a failure must not spend a counter"
        );
        let code = code_at_counter(&s, counter_at(now));
        assert!(matches!(
            st.consume(&s, &code, now),
            TotpVerdict::Accepted { .. }
        ));
    }

    #[test]
    fn another_secrets_code_does_not_verify() {
        let a = TotpSecret::generate();
        let b = TotpSecret::generate();
        let now = 1_700_000_000;
        let code = code_at_counter(&b, counter_at(now));
        let mut st = TotpState::default();
        assert_eq!(st.consume(&a, &code, now), TotpVerdict::Wrong);
    }

    #[test]
    fn base32_round_trips() {
        // The CODEC round-trips any length — that is a property of
        // base32, and it is tested against the codec directly.
        // `TotpSecret::from_base32` deliberately does not, because a
        // secret has a contract width; see the test below.
        for len in [1usize, 5, 10, 20, 32] {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            let back = base32_decode(&base32_encode(&bytes)).unwrap();
            assert_eq!(back, bytes, "len {len}");
        }
    }

    #[test]
    fn a_secret_of_the_wrong_width_is_refused() {
        // HMAC-SHA1 takes a key of ANY length, so a truncated or
        // emptied `totp_secret_base32` still generates codes — it just
        // generates them from a key an attacker can guess. Nothing
        // downstream would notice, which is why the width is checked
        // here at the parse boundary.
        for len in [0usize, 1, 10, 19, 21, 32] {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            assert!(
                TotpSecret::from_base32(&base32_encode(&bytes)).is_none(),
                "a {len}-byte secret must not parse"
            );
        }
        let good: Vec<u8> = (0..SECRET_LEN).map(|i| (i * 7 + 3) as u8).collect();
        let parsed = TotpSecret::from_base32(&base32_encode(&good)).expect("exact width parses");
        assert_eq!(parsed.as_bytes(), &good[..]);
    }

    #[test]
    fn a_non_ascii_label_percent_encodes_its_utf8_bytes() {
        // Encoding the CODE POINT gave `%E9` for `é` — a bare Latin-1
        // byte — and `%1F600` for an emoji, which is not a valid escape
        // at all, so the authenticator app fails to parse the URI.
        let secret = TotpSecret::generate();
        let uri = secret.provisioning_uri("Claudepot", "café@example.com");
        assert!(uri.contains("caf%C3%A9"), "{uri}");
        assert!(!uri.contains("%E9"), "{uri}");

        let emoji = secret.provisioning_uri("Claudepot", "grin😀");
        assert!(emoji.contains("grin%F0%9F%98%80"), "{emoji}");

        // Every escape in the URI is exactly two hex digits.
        for part in emoji.split('%').skip(1) {
            assert!(
                part.len() >= 2 && part[..2].bytes().all(|b| b.is_ascii_hexdigit()),
                "malformed escape in {emoji}"
            );
        }
    }

    #[test]
    fn base32_uses_only_the_rfc4648_alphabet() {
        let s = TotpSecret::generate().to_base32();
        assert!(
            s.chars().all(|c| B32.contains(&(c as u8))),
            "an authenticator app will refuse anything else: {s}"
        );
    }

    #[test]
    fn a_generated_secret_is_160_bits() {
        assert_eq!(TotpSecret::generate().as_bytes().len(), SECRET_LEN);
    }

    #[test]
    fn generated_secrets_differ() {
        use std::collections::HashSet;
        let set: HashSet<Vec<u8>> = (0..200)
            .map(|_| TotpSecret::generate().as_bytes().to_vec())
            .collect();
        assert_eq!(set.len(), 200);
    }

    #[test]
    fn the_secret_debug_does_not_print_the_secret() {
        let s = TotpSecret::generate();
        let rendered = format!("{s:?}");
        assert!(rendered.contains("redacted"));
        assert!(!rendered.contains(&s.to_base32()));
        // A derived Debug would print the byte vector verbatim.
        assert!(!rendered.contains(&format!("{}", s.as_bytes()[0])) || rendered.contains("bytes"));
    }

    #[test]
    fn the_provisioning_uri_carries_what_authenticator_apps_read() {
        let s = TotpSecret::generate();
        let uri = s.provisioning_uri("Claudepot", "macbook air");
        assert!(uri.starts_with("otpauth://totp/Claudepot:macbook%20air?"));
        assert!(uri.contains(&format!("secret={}", s.to_base32())));
        assert!(uri.contains("issuer=Claudepot"));
        assert!(uri.contains("algorithm=SHA1"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
    }
}

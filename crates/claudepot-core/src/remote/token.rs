//! Secrets for the remote-control surface: device tokens and the
//! short pairing codes used to issue them.
//!
//! ## Two secrets, opposite threat models
//!
//! A **device token** is machine-generated, 256 bits, and lives in the
//! phone's storage forever. Nobody guesses it, so the work goes into
//! never storing it in recoverable form.
//!
//! A **pairing code** is eight characters a human reads off one screen
//! and types into another. It is *deliberately* low entropy, so the
//! work goes into making guessing useless: it expires in minutes, it is
//! single-use, and the attempt counter kills it long before a search
//! becomes viable. Give a pairing code a long life and you have built a
//! password nobody chose.
//!
//! Getting these two backwards — a short token, or a pairing code with
//! no attempt cap — is the failure this module is shaped to prevent.
//!
//! ## Why no new dependency
//!
//! Randomness comes from `Uuid::new_v4`, which is `getrandom`-backed
//! (a CSPRNG, not a PRNG) and already a dependency. Pulling `rand` in
//! to produce 32 bytes does not clear the dependency-hygiene bar.
//!
//! Hashing is SHA-256, not Argon2, and that is deliberate rather than
//! lazy: Argon2 exists to make brute-forcing *low-entropy human
//! passwords* expensive. A 256-bit machine-generated token has nothing
//! to brute force, and a memory-hard KDF on every request would be a
//! real cost bought for nothing. The pairing code, which *is* low
//! entropy, is defended by expiry and an attempt cap — the controls
//! that actually apply to it.
//!
//! Comparison is constant-time and hand-rolled: `subtle` would be a new
//! dependency for five lines that are easier to audit than to justify.

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Characters a human can read off a screen and retype without
/// ambiguity. No `0`/`O`, no `1`/`I`/`l`, no `5`/`S`, no `2`/`Z`.
/// A pairing code that gets mistyped trains the user to paste secrets
/// around instead, which is a worse outcome than a shorter alphabet.
const CODE_ALPHABET: &[u8] = b"ACDEFGHJKMNPQRTUVWXY346789";

/// Characters in a pairing code. With a 26-symbol alphabet this is
/// ~37.6 bits — meaningless on its own, which is why `MAX_CODE_ATTEMPTS`
/// and the expiry below are load-bearing rather than belt-and-braces.
///
/// That figure is only true because `random_bytes` discards the UUID
/// bytes whose bits are fixed; consuming them cost 0.7 bits and a whole
/// symbol position. See `random_bytes`.
pub const CODE_LEN: usize = 8;

/// Wrong guesses before a pairing code is destroyed.
pub const MAX_CODE_ATTEMPTS: u32 = 5;

/// A freshly minted device token. The plaintext exists **only** in this
/// value and is shown to the user exactly once; the store keeps the
/// hash. Losing it means re-pairing, which is the correct trade.
#[derive(Clone)]
pub struct NewToken {
    pub plaintext: String,
    pub hash: String,
}

/// Hand-written so the bearer token cannot reach a log through a
/// `{:?}`. A derived `Debug` printed it in full, which is one
/// `tracing::debug!` away from a credential in a log file — and the
/// only reason it never happened is that nothing had logged this type
/// yet.
impl std::fmt::Debug for NewToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewToken")
            .field("plaintext", &"<redacted>")
            .field("hash", &self.hash)
            .finish()
    }
}

/// 256 bits of CSPRNG output, hex-encoded.
pub fn new_device_token() -> NewToken {
    // Two v4 UUIDs are 244 bits of randomness across 256 bits of
    // output; the version/variant bits are fixed, not attacker-chosen.
    let mut raw = Vec::with_capacity(32);
    raw.extend_from_slice(Uuid::new_v4().as_bytes());
    raw.extend_from_slice(Uuid::new_v4().as_bytes());
    let plaintext = hex::encode(&raw);
    let hash = hash_secret(&plaintext);
    NewToken { plaintext, hash }
}

/// Indices inside a UUIDv4 whose bits are **not** random: byte 6 has
/// its high nibble pinned to the version (`0100`), byte 8 has its high
/// bits pinned to the variant. Feeding those into the alphabet silently
/// narrows the symbol set at a fixed position.
const UUID_NON_RANDOM: [usize; 2] = [6, 8];

/// A pairing code drawn from the unambiguous alphabet.
pub fn new_pairing_code() -> String {
    let mut out = String::with_capacity(CODE_LEN);
    for b in random_bytes(CODE_LEN) {
        // Modulo bias: 256 values over 26 symbols gives 22 symbols ten
        // preimages and four symbols nine — about a 10% relative edge on
        // the common ones, worth ~0.02 bits overall. Left in
        // deliberately, and documented so nobody "fixes" it into a
        // rejection loop believing the code got stronger. The code's
        // defence is expiry plus the attempt cap, not its entropy.
        out.push(CODE_ALPHABET[b as usize % CODE_ALPHABET.len()] as char);
    }
    out
}

/// `n` bytes of CSPRNG output, with the structurally-fixed UUID bytes
/// discarded rather than consumed.
///
/// The first version of this took the first `n` bytes of a UUID, which
/// put byte 6 — the version byte — at character 7 and cut that position
/// from 26 possible symbols to 16. Support fell from 26^8 to 26^7 x 16:
/// 36.9 bits rather than the 37.6 the docs claimed. Small in absolute
/// terms, and entirely invisible: the code still looked random.
pub(super) fn random_bytes(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        let uuid = Uuid::new_v4();
        for (i, b) in uuid.as_bytes().iter().enumerate() {
            if UUID_NON_RANDOM.contains(&i) {
                continue;
            }
            out.push(*b);
            if out.len() == n {
                break;
            }
        }
    }
    out
}

pub fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// Constant-time equality over the hex hashes.
///
/// Compares hashes rather than secrets, so a length mismatch is not
/// itself a leak — but the loop is still fixed-time in the common case
/// because an early return on the first differing byte would time-leak
/// how much of a *hash* an attacker had matched, which is exactly the
/// oracle an online guessing attack wants.
pub fn verify_secret(candidate: &str, expected_hash: &str) -> bool {
    let actual = hash_secret(candidate);
    constant_time_eq(actual.as_bytes(), expected_hash.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_device_token_is_256_bits_of_hex() {
        let t = new_device_token();
        assert_eq!(t.plaintext.len(), 64, "32 bytes hex-encoded");
        assert!(t.plaintext.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_token_never_equals_its_own_hash() {
        // The obvious catastrophic bug: storing the plaintext in the
        // `hash` field.
        let t = new_device_token();
        assert_ne!(t.plaintext, t.hash);
    }

    #[test]
    fn tokens_do_not_repeat() {
        let set: HashSet<String> = (0..500).map(|_| new_device_token().plaintext).collect();
        assert_eq!(set.len(), 500);
    }

    #[test]
    fn a_token_verifies_against_its_hash_and_nothing_else() {
        let t = new_device_token();
        assert!(verify_secret(&t.plaintext, &t.hash));
        assert!(!verify_secret("wrong", &t.hash));
        assert!(!verify_secret("", &t.hash));
        let other = new_device_token();
        assert!(!verify_secret(&other.plaintext, &t.hash));
    }

    #[test]
    fn a_pairing_code_uses_only_unambiguous_characters() {
        for _ in 0..200 {
            let code = new_pairing_code();
            assert_eq!(code.len(), CODE_LEN);
            for c in code.chars() {
                assert!(
                    CODE_ALPHABET.contains(&(c as u8)),
                    "{c} is not in the unambiguous alphabet"
                );
                assert!(
                    !"O0I1lS5Z2".contains(c),
                    "{c} is a character pair humans confuse"
                );
            }
        }
    }

    #[test]
    fn pairing_codes_vary() {
        let set: HashSet<String> = (0..200).map(|_| new_pairing_code()).collect();
        assert!(set.len() > 190, "got {} distinct of 200", set.len());
    }

    #[test]
    fn constant_time_eq_agrees_with_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn hashing_is_stable() {
        assert_eq!(hash_secret("hello"), hash_secret("hello"));
        assert_ne!(hash_secret("hello"), hash_secret("hellp"));
        assert_eq!(hash_secret("hello").len(), 64);
    }
}

#[cfg(test)]
mod entropy_tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// Regression for a real defect: taking the first `CODE_LEN` bytes
    /// of a UUID put the version byte at character 7 and cut that
    /// position from 26 symbols to 16. Every position must be able to
    /// produce every symbol.
    #[test]
    fn every_character_position_can_produce_every_symbol() {
        let mut seen: HashMap<usize, HashSet<char>> = HashMap::new();
        for _ in 0..20_000 {
            for (i, c) in new_pairing_code().chars().enumerate() {
                seen.entry(i).or_default().insert(c);
            }
        }
        for i in 0..CODE_LEN {
            let n = seen[&i].len();
            assert_eq!(
                n,
                CODE_ALPHABET.len(),
                "position {} produced only {n} of {} symbols — a fixed-bit \
                 byte is being consumed as randomness",
                i + 1,
                CODE_ALPHABET.len()
            );
        }
    }

    #[test]
    fn random_bytes_never_returns_a_uuid_version_byte_pattern() {
        // Byte 6 of a UUIDv4 is always 0x4_. If it were being consumed,
        // one position in every 14 would land in 0x40..=0x4F far more
        // often than chance.
        let n = 14_000;
        let bytes = random_bytes(n);
        assert_eq!(bytes.len(), n);
        let pinned = bytes.iter().filter(|b| (**b & 0xF0) == 0x40).count();
        let expected = n / 16;
        assert!(
            pinned < expected * 2,
            "{pinned} bytes in 0x40..0x4F out of {n} (chance ~{expected}) — \
             version bytes are leaking into the stream"
        );
    }

    #[test]
    fn a_new_token_debug_does_not_print_the_bearer() {
        let t = new_device_token();
        let rendered = format!("{t:?}");
        assert!(
            !rendered.contains(&t.plaintext),
            "NewToken's Debug leaked the bearer token: {rendered}"
        );
        assert!(rendered.contains("redacted"));
    }
}

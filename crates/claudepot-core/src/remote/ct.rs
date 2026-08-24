//! One constant-time comparison, for every credential check on this
//! surface.
//!
//! There were two, byte-identical, in `token` and in `totp`. Both are
//! timing-sensitive comparisons of a secret, which is the one shape of
//! duplicate that must not be allowed to drift: a fix applied to one
//! copy leaves the other quietly comparing with `==` semantics, and
//! nothing fails. The compiler cannot tell you the copies disagree, and
//! neither can a test that only exercises one of them.

/// Compare two byte strings in time that depends on their length but
/// not on their contents.
///
/// Length is deliberately not hidden: every caller here compares a
/// fixed-width digest or a fixed-width code, so the length carries no
/// secret, and an early return on a mismatch is clearer than padding.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
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

    #[test]
    fn equal_slices_compare_equal() {
        assert!(constant_time_eq(b"abcdef", b"abcdef"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn any_differing_byte_is_caught() {
        assert!(!constant_time_eq(b"abcdef", b"abcdeg"));
        assert!(!constant_time_eq(b"abcdef", b"Abcdef"));
        // The accumulator is an OR of XORs; a naive version that
        // assigned instead of OR-ing would report equal here, because
        // the last byte pair matches.
        assert!(!constant_time_eq(b"xbcdef", b"abcdef"));
    }

    #[test]
    fn different_lengths_are_never_equal() {
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}

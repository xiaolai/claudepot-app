//! Unicode NFC normalization at the migrate boundary.
//!
//! See `dev-docs/project-migrate-spec.md` §5.8 and
//! `dev-docs/project-migrate-cc-research.md` §2.1.
//!
//! CC normalizes paths via `realpath().normalize('NFC')` in
//! `canonicalizePath` (`sessionStoragePortable.ts:339-345`) and again in
//! `getAutoMemPath` (`memdir/paths.ts:223-235`). HFS+/APFS may round-
//! trip filenames as NFD; ext4 stores what was written. A path
//! containing `é` in NFD (`U+0065 U+0301`) sanitizes differently from
//! the same `é` in NFC (`U+00E9`) — different slugs, different
//! on-disk dirs. The migrator must NFC-normalize both sides of every
//! substitution rule **before** feeding them into `sanitize_path`.
//!
//! NFC is applied at the boundary; internal `&str` ops stay byte-level.

use unicode_normalization::UnicodeNormalization;

/// Normalize a string to Unicode NFC form. Idempotent and lossless on
/// already-NFC input. Applied to both sides of every substitution rule
/// before slug recompute.
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfc_passthrough_ascii() {
        assert_eq!(nfc("/Users/joker/project"), "/Users/joker/project");
    }

    #[test]
    fn nfc_already_nfc_idempotent() {
        // U+00E9 is the precomposed NFC form.
        let nfc_form = "/tmp/caf\u{00E9}";
        assert_eq!(nfc(nfc_form), nfc_form);
    }

    #[test]
    fn nfc_decomposed_recombines() {
        // U+0065 (e) + U+0301 (combining acute) → U+00E9 (é).
        let nfd_form = "/tmp/caf\u{0065}\u{0301}";
        let normalized = nfc(nfd_form);
        assert_eq!(normalized, "/tmp/caf\u{00E9}");
        // NFC form is shorter in bytes (2-byte UTF-8 vs 3-byte for
        // base + combining mark).
        assert!(normalized.len() < nfd_form.len());
    }

    #[test]
    fn nfc_empty_input() {
        assert_eq!(nfc(""), "");
    }

    #[test]
    fn nfc_emoji_passthrough() {
        // BMP-supplementary chars (4-byte UTF-8) round-trip unchanged.
        assert_eq!(nfc("/tmp/\u{1F389}"), "/tmp/\u{1F389}");
    }

    #[test]
    fn nfc_idempotent_double_apply() {
        let nfd = "/tmp/caf\u{0065}\u{0301}";
        let once = nfc(nfd);
        let twice = nfc(&once);
        assert_eq!(once, twice);
    }
}

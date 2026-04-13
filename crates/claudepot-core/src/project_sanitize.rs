//! CC path sanitization — mirrors CC's sessionStoragePortable.ts:311-319.

pub(crate) const MAX_SANITIZED_LENGTH: usize = 200;

/// Replicate CC's `sanitizePath`. Non-alphanumeric ASCII chars become `-`.
/// Paths longer than 200 chars get a djb2 hash suffix.
pub fn sanitize_path(name: &str) -> String {
    // Iterate UTF-16 code units to match JS's `.replace(/[^a-zA-Z0-9]/g, '-')`.
    // JS strings are UTF-16, so surrogate pairs (emoji, etc.) produce 2 hyphens
    // where Rust's char iterator would produce 1.
    let sanitized: String = name
        .encode_utf16()
        .map(|u| {
            let c = u as u8;
            if u < 128 && (c as char).is_ascii_alphanumeric() {
                c as char
            } else {
                '-'
            }
        })
        .collect();
    if sanitized.len() <= MAX_SANITIZED_LENGTH {
        sanitized
    } else {
        let hash = djb2_hash(name);
        format!("{}-{}", &sanitized[..MAX_SANITIZED_LENGTH], hash)
    }
}

/// Best-effort reverse of `sanitize_path`. Lossy: hyphens could have been
/// any non-alphanumeric char. Used for display only.
pub fn unsanitize_path(sanitized: &str) -> String {
    sanitized.replace('-', "/")
}

/// CC's djb2 hash — matches `djb2Hash()` in CC's `hash.ts`.
///
/// CC uses: seed=0, multiplier=31 (via `(h<<5)-h`), signed 32-bit,
/// then `Math.abs().toString(36)`. Input is UTF-16 code units
/// (JavaScript's `.charCodeAt()`), not UTF-8 bytes.
///
/// This is a 32-bit hash, so collisions are expected at ~65 536 entries
/// (birthday bound). We accept this because CC uses the same algorithm
/// and we must produce identical directory names for compatibility.
pub(crate) fn djb2_hash(s: &str) -> String {
    let mut hash: i32 = 0;
    // Iterate UTF-16 code units to match JavaScript's charCodeAt()
    for code_unit in s.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(code_unit as i32);
    }
    // CC does Math.abs(hash).toString(36)
    let abs = (hash as i64).unsigned_abs() as u32;
    format_radix(abs, 36)
}

pub(crate) fn format_radix(mut x: u32, radix: u32) -> String {
    if x == 0 {
        return "0".to_string();
    }
    let mut result = Vec::new();
    while x > 0 {
        let digit = (x % radix) as u8;
        let ch = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        result.push(ch);
        x /= radix;
    }
    result.reverse();
    String::from_utf8(result).unwrap_or_default()
}

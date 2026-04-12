//! CC path sanitization — mirrors CC's sessionStoragePortable.ts:311-319.

pub(crate) const MAX_SANITIZED_LENGTH: usize = 200;

/// Replicate CC's `sanitizePath`. Non-alphanumeric ASCII chars become `-`.
/// Paths longer than 200 chars get a djb2 hash suffix.
pub fn sanitize_path(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
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

/// djb2 string hash (Daniel J. Bernstein). Returns the hash as a base-36 string.
///
/// This is a 32-bit hash, so collisions are expected at ~65 536 entries
/// (birthday bound). We accept this because CC uses the same algorithm
/// and we must produce identical directory names for compatibility.
pub(crate) fn djb2_hash(s: &str) -> String {
    let mut hash: u32 = 5381;
    for byte in s.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u32);
    }
    format_radix(hash, 36)
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

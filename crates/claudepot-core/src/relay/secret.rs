//! One type for every credential the relay holds in memory.
//!
//! `Debug` and `Display` both print `<redacted>`, so a `{:?}` in a
//! `tracing` call or an error chain cannot leak the value, and the bytes
//! are zeroized on drop because `String`'s deallocator does not scrub.
//! [`Secret::expose`] is the one way to read it; every call site is a
//! place a reviewer should look at.

use zeroize::Zeroize;

/// The one rule for a credential-store key (service or account name) on
/// every platform. macOS passes the attributes across a `security -i`
/// command line, so only the characters a UUID-derived account or a
/// service name uses are accepted; Linux and Windows apply the same rule
/// so a key that works on one host works on every host.
pub fn store_key_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The plaintext. Use it to build a header or compare a digest; never
    /// format it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        crate::remote::ct::constant_time_eq(self.0.as_bytes(), other.0.as_bytes())
    }
}

impl Eq for Secret {}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neither_debug_nor_display_prints_the_value() {
        let s = Secret::new("sk-ant-oat01-canary");
        assert!(!format!("{s:?}").contains("canary"));
        assert!(!format!("{s}").contains("canary"));
        assert_eq!(s.expose(), "sk-ant-oat01-canary");
    }

    #[test]
    fn equality_compares_the_value() {
        assert_eq!(Secret::new("a"), Secret::new("a"));
        assert_ne!(Secret::new("a"), Secret::new("b"));
    }
}

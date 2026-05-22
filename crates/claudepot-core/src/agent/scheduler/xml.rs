//! Tiny XML emitter used by `launchd.rs` and `schtasks.rs`.
//!
//! Hand-rolled instead of pulling in `quick-xml` / `plist`: the
//! formats we emit are constrained subsets of XML 1.0 (no
//! namespaces, no CDATA, no processing instructions beyond the
//! prolog), and golden tests prefer byte-stable output that we
//! control directly.

/// Escape `&`, `<`, `>`, `"`, `'` for embedding in XML element
/// content or attribute values.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Push two-space indentation `n` levels deep onto `out`.
pub fn indent(out: &mut String, n: usize) {
    for _ in 0..n {
        out.push_str("  ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_handles_each_predefined_entity() {
        assert_eq!(xml_escape("a&b"), "a&amp;b");
        assert_eq!(xml_escape("a<b>c"), "a&lt;b&gt;c");
        assert_eq!(xml_escape(r#"a"b"#), "a&quot;b");
        assert_eq!(xml_escape("a'b"), "a&apos;b");
        assert_eq!(xml_escape("plain"), "plain");
        assert_eq!(xml_escape(""), "");
    }
}

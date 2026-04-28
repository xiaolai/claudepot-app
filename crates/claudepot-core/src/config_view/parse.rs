//! Per-kind file parsers for the Config section.
//!
//! Parity notes live in `dev-docs/config-section-plan.md` §6.5. Keep
//! parsers stateless — they take bytes, return `(summary, issues)`.
//! Strict vs. tolerant is per-kind.

use crate::config_view::model::{FileSummary, ParseIssue};
use serde_json::Value;

/// Result of parsing a single file's head bytes.
pub struct Parsed {
    pub summary: Option<FileSummary>,
    pub issues: Vec<ParseIssue>,
}

impl Parsed {
    pub fn empty() -> Self {
        Self {
            summary: None,
            issues: Vec::new(),
        }
    }
    pub fn with_summary(s: FileSummary) -> Self {
        Self {
            summary: Some(s),
            issues: Vec::new(),
        }
    }
}

// ---------- Strict JSON (settings, keybindings) -----------------------

/// Parse strict JSON (no JSON5, no trailing commas). Surface a
/// `MalformedJson` issue on failure; drop the summary.
pub fn parse_settings_json(bytes: &[u8]) -> Parsed {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(_)) => Parsed::empty(),
        Ok(_) => Parsed {
            summary: None,
            issues: vec![ParseIssue::MalformedJson {
                message: "expected object at top level".to_string(),
            }],
        },
        Err(e) => Parsed {
            summary: None,
            issues: vec![ParseIssue::MalformedJson {
                message: e.to_string(),
            }],
        },
    }
}

// ---------- Markdown with frontmatter (agents, rules, commands) -------

/// Lightweight frontmatter extractor — returns `(frontmatter, body)`.
/// Only matches the CC shape: `^---\n…\n---\n`. Everything before the
/// closing fence is the frontmatter string.
pub fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let t = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"));
    let Some(rest) = t else {
        return (None, text);
    };
    // Find the closing fence at line start.
    let mut pos = 0usize;
    let bytes = rest.as_bytes();
    while pos < bytes.len() {
        let line_start = pos;
        while pos < bytes.len() && bytes[pos] != b'\n' {
            pos += 1;
        }
        let line = &rest[line_start..pos];
        if line.trim_end_matches('\r') == "---" {
            // Consume the newline after ---
            let after = if pos < bytes.len() { pos + 1 } else { pos };
            return (Some(&rest[..line_start]), &rest[after..]);
        }
        pos += 1;
    }
    (None, text)
}

/// Extract the first H1 and first non-empty paragraph as a summary.
pub fn markdown_summary(body: &str) -> FileSummary {
    let mut title: Option<String> = None;
    let mut description: Option<String> = None;
    for line in body.lines() {
        let t = line.trim();
        if title.is_none() && (t.starts_with("# ") || t.starts_with("#\t")) {
            title = Some(t.trim_start_matches('#').trim().to_string());
            continue;
        }
        if description.is_none() && !t.is_empty() && !t.starts_with('#') {
            description = Some(t.to_string());
            if title.is_some() {
                break;
            }
        }
    }
    FileSummary { title, description }
}

/// Parse a markdown file: extract H1 + first paragraph as summary.
pub fn parse_claude_md(bytes: &[u8]) -> Parsed {
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let (_fm, body) = split_frontmatter(text);
    Parsed::with_summary(markdown_summary(body))
}

/// Parse an agent/rule/command file — tolerant frontmatter + markdown.
/// Frontmatter `name:` / `description:` take priority over the body's
/// first H1 + paragraph, matching CC's agent-loader precedence.
pub fn parse_frontmatter_markdown(bytes: &[u8]) -> Parsed {
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let (fm, body) = split_frontmatter(text);

    let mut title: Option<String> = None;
    let mut description: Option<String> = None;

    if let Some(fm) = fm {
        // Cheap frontmatter scrape — tolerant per CC's
        // `frontmatterParser.ts` behavior. No YAML dep.
        for line in fm.lines() {
            let t = line.trim();
            if title.is_none() {
                if let Some(rest) = t.strip_prefix("name:") {
                    let v = rest.trim().trim_matches('"').to_string();
                    if !v.is_empty() {
                        title = Some(v);
                    }
                }
            }
            if description.is_none() {
                if let Some(rest) = t.strip_prefix("description:") {
                    let v = rest.trim().trim_matches('"').to_string();
                    if !v.is_empty() {
                        description = Some(v);
                    }
                }
            }
        }
    }

    let body_summary = markdown_summary(body);
    let summary = FileSummary {
        title: title.or(body_summary.title),
        description: description.or(body_summary.description),
    };
    Parsed::with_summary(summary)
}

// ---------- Memory (first 30 lines only) -----------------------------

/// Parse the first 30 lines of a memory file — matches CC's
/// `memoryScan.ts:21-76` bounded read.
pub fn parse_memory_head(bytes: &[u8]) -> Parsed {
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let mut head = String::new();
    for (i, line) in text.lines().enumerate() {
        if i >= 30 {
            break;
        }
        head.push_str(line);
        head.push('\n');
    }
    let (_, body) = split_frontmatter(&head);
    Parsed::with_summary(markdown_summary(body))
}

// ---------- MemoryIndex (MEMORY.md bullet shape) ----------------------

#[derive(Debug, Clone)]
pub struct MemoryIndexEntry {
    pub title: String,
    pub file: String,
    pub hook: Option<String>,
}

/// Parse `MEMORY.md` lines like `- [Title](file.md) — hook`.
pub fn parse_memory_index(bytes: &[u8]) -> (Vec<MemoryIndexEntry>, Parsed) {
    let text = std::str::from_utf8(bytes).unwrap_or("");
    let mut entries = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if !t.starts_with("- [") {
            continue;
        }
        // "- [Title](file.md) — hook"
        let Some(after_dash) = t.strip_prefix("- [") else {
            continue;
        };
        let Some(close) = after_dash.find("](") else {
            continue;
        };
        let title = after_dash[..close].to_string();
        let rest = &after_dash[close + 2..];
        let Some(end) = rest.find(')') else { continue };
        let file = rest[..end].to_string();
        let tail = rest[end + 1..].trim_start();
        let hook = tail
            .strip_prefix("—")
            .or_else(|| tail.strip_prefix("-"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        entries.push(MemoryIndexEntry { title, file, hook });
    }
    let parsed = Parsed::with_summary(FileSummary {
        title: Some("MEMORY.md".to_string()),
        description: Some(format!("{} entries", entries.len())),
    });
    (entries, parsed)
}

// ---------- Skills (strict dir shape) ---------------------------------

/// A flat `.md` under `skills/` is invalid — CC only accepts
/// `skills/<name>/SKILL.md`. See plan §6.5.
pub fn invalid_flat_skill() -> Parsed {
    Parsed {
        summary: None,
        issues: vec![ParseIssue::NotASkill],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_json_accepts_object() {
        let p = parse_settings_json(br#"{"a": 1}"#);
        assert!(p.issues.is_empty());
    }

    #[test]
    fn strict_json_rejects_malformed() {
        let p = parse_settings_json(br#"{ "a": ,}"#);
        assert_eq!(p.issues.len(), 1);
        matches!(p.issues[0], ParseIssue::MalformedJson { .. });
    }

    #[test]
    fn strict_json_rejects_top_level_array() {
        let p = parse_settings_json(br#"[1,2,3]"#);
        assert_eq!(p.issues.len(), 1);
    }

    #[test]
    fn split_frontmatter_extracts_fence() {
        let (fm, body) = split_frontmatter("---\nname: foo\n---\n# Title\nBody");
        assert_eq!(fm, Some("name: foo\n"));
        assert_eq!(body, "# Title\nBody");
    }

    #[test]
    fn split_frontmatter_no_fence_returns_whole() {
        let (fm, body) = split_frontmatter("# Title\nBody");
        assert!(fm.is_none());
        assert_eq!(body, "# Title\nBody");
    }

    #[test]
    fn markdown_summary_picks_h1_then_paragraph() {
        let s = markdown_summary("# My Title\n\nDescription line.\n");
        assert_eq!(s.title.as_deref(), Some("My Title"));
        assert_eq!(s.description.as_deref(), Some("Description line."));
    }

    #[test]
    fn memory_head_stops_at_30_lines() {
        let mut s = String::new();
        for i in 0..60 {
            s.push_str(&format!("line {}\n", i));
        }
        let p = parse_memory_head(s.as_bytes());
        // Sanity: we got a summary, didn't blow up on long input.
        assert!(p.summary.is_some());
    }

    #[test]
    fn memory_index_parses_bullet() {
        let bytes = b"- [Hello](hello.md) \xe2\x80\x94 greeting\n- [NoHook](x.md)\n";
        let (entries, _) = parse_memory_index(bytes);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Hello");
        assert_eq!(entries[0].file, "hello.md");
        assert_eq!(entries[0].hook.as_deref(), Some("greeting"));
        assert_eq!(entries[1].hook, None);
    }

    #[test]
    fn frontmatter_markdown_prefers_name_over_h1() {
        let p = parse_frontmatter_markdown(
            b"---\nname: explicit-agent\ndescription: Does the thing\n---\n# Ignored Header\n",
        );
        let s = p.summary.unwrap();
        assert_eq!(s.title.as_deref(), Some("explicit-agent"));
        assert_eq!(s.description.as_deref(), Some("Does the thing"));
    }
}

//! Render a `SessionDetail` into Markdown or JSON for archiving /
//! sharing. Inspired by claude-devtools' `sessionExporter`.
//!
//! The markdown output preserves logical turn boundaries and never
//! leaks credentials — before serializing we run every string through
//! [`redact_secrets`](#fn.redact_secrets) to truncate any `sk-ant-`
//! token the transcript happens to echo (these shouldn't be there, but
//! a shared transcript is exactly where a leak would hurt).
//!
//! JSON export is a straight serde serialization of the public session
//! types — it's lossless by design so downstream tooling can re-parse
//! with the same Rust structs we ship elsewhere.

use crate::session::SessionDetail;
use crate::session::SessionEvent;
use crate::session_chunks::{build_chunks, SessionChunk};
use serde::Serialize;
use std::fmt::Write as _;

/// Format hint forwarded from CLI / Tauri invocations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    /// Markdown with phase-2 slimming applied: oversized tool_result
    /// payloads are summarized inline.
    MarkdownSlim,
    /// Self-contained HTML. `no_js` strips the optional copy-buttons
    /// script so the file is guaranteed <script>-free.
    Html {
        no_js: bool,
    },
    Json,
}

/// Top-level entry point. Applies the given redaction policy first,
/// then renders.
///
/// Returns the rendered body as a string. Callers decide where to put
/// it (stdout, file, clipboard).
pub fn export(detail: &SessionDetail, format: ExportFormat) -> String {
    export_with(
        detail,
        format,
        &crate::redaction::RedactionPolicy::default(),
    )
}

/// Like `export` but honors a custom `RedactionPolicy`. `export` is
/// kept as a shim for the existing single-arg callers.
pub fn export_with(
    detail: &SessionDetail,
    format: ExportFormat,
    policy: &crate::redaction::RedactionPolicy,
) -> String {
    let rendered = match format {
        ExportFormat::Markdown => export_markdown(detail),
        ExportFormat::MarkdownSlim => export_markdown_slim(detail),
        ExportFormat::Html { no_js } => export_html(detail, no_js),
        ExportFormat::Json => export_json(detail),
    };
    crate::redaction::apply(&rendered, policy)
}

/// Pure preview — same string `export_with` produces, no file I/O.
/// Kept explicit so GUI callers can advertise "safe preview".
pub fn export_preview(
    detail: &SessionDetail,
    format: ExportFormat,
    policy: &crate::redaction::RedactionPolicy,
) -> String {
    export_with(detail, format, policy)
}

/// Markdown with slim applied at the event level: `UserToolResult`
/// payloads over 1 KiB get their content replaced with a short
/// `(tool result redacted — N bytes)` marker before the renderer sees
/// them. Every other event kind passes through.
pub fn export_markdown_slim(detail: &SessionDetail) -> String {
    use crate::session::SessionEvent;
    const SLIM_THRESH: usize = 1024;
    let slimmed_events: Vec<SessionEvent> = detail
        .events
        .iter()
        .map(|ev| match ev {
            SessionEvent::UserToolResult {
                ts,
                uuid,
                tool_use_id,
                content,
                is_error,
            } if content.len() > SLIM_THRESH => SessionEvent::UserToolResult {
                ts: *ts,
                uuid: uuid.clone(),
                tool_use_id: tool_use_id.clone(),
                content: format!("(tool result redacted — {} bytes)", content.len()),
                is_error: *is_error,
            },
            other => other.clone(),
        })
        .collect();
    let slimmed = crate::session::SessionDetail {
        row: detail.row.clone(),
        events: slimmed_events,
    };
    export_markdown(&slimmed)
}

/// Self-contained HTML. Paper-mono feel: hairline borders, system
/// mono font. Light / dark via `prefers-color-scheme`. Optional copy
/// script embedded at the end when `no_js` is false.
pub fn export_html(detail: &SessionDetail, no_js: bool) -> String {
    let row = &detail.row;
    let mut s = String::with_capacity(8 * 1024);
    let _ = writeln!(s, "<!doctype html>");
    let _ = writeln!(s, "<html lang=\"en\">");
    let _ = writeln!(s, "<head>");
    let _ = writeln!(s, "<meta charset=\"utf-8\">");
    let _ = writeln!(s, "<title>Session {}</title>", html_escape(&row.session_id));
    let _ = writeln!(
        s,
        "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">"
    );
    let _ = writeln!(s, "<style>");
    s.push_str(HTML_CSS);
    let _ = writeln!(s, "</style>");
    let _ = writeln!(s, "</head>");
    let _ = writeln!(s, "<body>");
    let _ = writeln!(s, "<header class=\"head\">");
    let _ = writeln!(
        s,
        "<h1>Session <code>{}</code></h1>",
        html_escape(&row.session_id)
    );
    let _ = writeln!(
        s,
        "<dl class=\"meta\"><dt>Project</dt><dd><code>{}</code></dd>",
        html_escape(&row.project_path)
    );
    if let Some(b) = &row.git_branch {
        let _ = writeln!(s, "<dt>Branch</dt><dd><code>{}</code></dd>", html_escape(b));
    }
    if !row.models.is_empty() {
        let _ = writeln!(
            s,
            "<dt>Models</dt><dd>{}</dd>",
            html_escape(&row.models.join(", "))
        );
    }
    let _ = writeln!(s, "</dl>");
    let _ = writeln!(s, "</header>");
    let _ = writeln!(s, "<main>");
    for ev in &detail.events {
        html_write_event(&mut s, ev);
    }
    let _ = writeln!(s, "</main>");
    if !no_js {
        let _ = writeln!(s, "<script>{HTML_JS}</script>");
    }
    let _ = writeln!(s, "</body>");
    let _ = writeln!(s, "</html>");
    s
}

const HTML_CSS: &str = r#"
:root {
  color-scheme: light dark;
  --fg: #111;
  --bg: #faf9f6;
  --muted: #555;
  --line: #ddd;
  --accent: #b7410e;
}
@media (prefers-color-scheme: dark) {
  :root {
    --fg: #e8e6e1;
    --bg: #191816;
    --muted: #aaa;
    --line: #333;
    --accent: #e8754a;
  }
}
body {
  font-family: ui-monospace, Menlo, Consolas, monospace;
  color: var(--fg);
  background: var(--bg);
  margin: 2rem auto;
  max-width: 820px;
  padding: 0 1rem;
}
.head h1 { margin: 0 0 0.5rem; font-size: 1rem; }
.meta { display: grid; grid-template-columns: auto 1fr; gap: 0.25rem 1rem; font-size: 0.85rem; color: var(--muted); }
.turn { margin: 1.5rem 0; border-top: 1px solid var(--line); padding-top: 0.75rem; }
.turn.user::before { content: "USER"; color: var(--accent); font-size: 0.75rem; letter-spacing: 0.08em; }
.turn.assistant::before { content: "ASSISTANT"; color: var(--muted); font-size: 0.75rem; letter-spacing: 0.08em; }
details { margin: 0.5rem 0; }
details > summary { cursor: pointer; color: var(--muted); }
pre { white-space: pre-wrap; word-break: break-word; font-size: 0.85rem; }
code { background: color-mix(in oklab, var(--fg) 8%, transparent); padding: 0 0.2em; border-radius: 3px; }
"#;

const HTML_JS: &str = r#"document.querySelectorAll('pre').forEach(p => {
  const b = document.createElement('button');
  b.textContent = 'copy';
  b.style.cssText = 'float:right;font-size:0.7rem;opacity:0.5';
  b.onclick = () => navigator.clipboard.writeText(p.textContent);
  p.parentElement.insertBefore(b, p);
});"#;

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_write_event(s: &mut String, ev: &crate::session::SessionEvent) {
    use crate::session::SessionEvent;
    match ev {
        SessionEvent::UserText { text, .. } => {
            let _ = writeln!(
                s,
                "<section class=\"turn user\"><pre>{}</pre></section>",
                html_escape(text)
            );
        }
        SessionEvent::AssistantText { text, .. } => {
            let _ = writeln!(
                s,
                "<section class=\"turn assistant\"><pre>{}</pre></section>",
                html_escape(text)
            );
        }
        SessionEvent::AssistantThinking { text, .. } => {
            let _ = writeln!(
                s,
                "<details class=\"turn assistant\"><summary>thinking</summary><pre>{}</pre></details>",
                html_escape(text)
            );
        }
        SessionEvent::AssistantToolUse {
            tool_name,
            input_preview,
            ..
        } => {
            let _ = writeln!(
                s,
                "<details open class=\"turn assistant\"><summary>tool call: {}</summary><pre>{}</pre></details>",
                html_escape(tool_name),
                html_escape(input_preview)
            );
        }
        SessionEvent::UserToolResult { content, .. } => {
            let _ = writeln!(
                s,
                "<details class=\"turn user\"><summary>tool result</summary><pre>{}</pre></details>",
                html_escape(content)
            );
        }
        SessionEvent::Summary { text, .. } => {
            let _ = writeln!(
                s,
                "<section class=\"turn\"><em>summary: {}</em></section>",
                html_escape(text)
            );
        }
        _ => {}
    }
}

/// Markdown renderer. Reconstructs user/assistant bubbles, tool call
/// blocks (collapsible), thinking blocks, and compaction dividers.
///
/// The output is GitHub-flavored Markdown with a `---` frontmatter-ish
/// metadata block at the top so the transcript is self-describing when
/// pasted anywhere.
pub fn export_markdown(detail: &SessionDetail) -> String {
    let mut s = String::with_capacity(8 * 1024);
    let row = &detail.row;

    // Header — the same fields the GUI header strip surfaces.
    let _ = writeln!(s, "# Session `{}`", row.session_id);
    let _ = writeln!(s);
    let _ = writeln!(s, "- **Project:** `{}`", row.project_path);
    if let Some(branch) = &row.git_branch {
        let _ = writeln!(s, "- **Branch:** `{branch}`");
    }
    if let Some(ver) = &row.cc_version {
        let _ = writeln!(s, "- **CC version:** `{ver}`");
    }
    if !row.models.is_empty() {
        let _ = writeln!(s, "- **Models:** {}", row.models.join(", "));
    }
    if let Some(first) = row.first_ts {
        let _ = writeln!(s, "- **Started:** {}", first.to_rfc3339());
    }
    if let Some(last) = row.last_ts {
        let _ = writeln!(s, "- **Last event:** {}", last.to_rfc3339());
    }
    let _ = writeln!(
        s,
        "- **Turns:** {} (user {}, assistant {})",
        row.message_count, row.user_message_count, row.assistant_message_count
    );
    let _ = writeln!(
        s,
        "- **Tokens:** {} input, {} output, {} cache read, {} cache create",
        row.tokens.input, row.tokens.output, row.tokens.cache_read, row.tokens.cache_creation
    );
    let _ = writeln!(s);
    let _ = writeln!(s, "---");
    let _ = writeln!(s);

    let chunks = build_chunks(&detail.events);
    if chunks.is_empty() {
        s.push_str("_(empty transcript)_\n");
        return s;
    }
    for chunk in chunks {
        render_chunk(&mut s, &chunk, &detail.events);
    }
    s
}

/// Strict JSON export — redacts secrets in string fields on a clone so
/// the on-disk row remains untouched.
pub fn export_json(detail: &SessionDetail) -> String {
    let mut cloned = detail.clone();
    redact_in_place(&mut cloned);
    let wrapped = ExportJson { detail: &cloned };
    serde_json::to_string_pretty(&wrapped).unwrap_or_else(|_| "{}".to_string())
}

#[derive(Serialize)]
struct ExportJson<'a> {
    detail: &'a SessionDetail,
}

// ---------------------------------------------------------------------------
// Markdown helpers
// ---------------------------------------------------------------------------

fn render_chunk(s: &mut String, chunk: &SessionChunk, events: &[SessionEvent]) {
    match chunk {
        SessionChunk::User { event_index, .. } => {
            if let Some(SessionEvent::UserText { text, ts, .. }) = events.get(*event_index) {
                let _ = writeln!(
                    s,
                    "### 👤 User{}",
                    ts.map(|t| format!(" — {}", t.to_rfc3339()))
                        .unwrap_or_default()
                );
                let _ = writeln!(s);
                let _ = writeln!(s, "{}", redact_secrets(text.trim()));
                let _ = writeln!(s);
            }
        }
        SessionChunk::System { event_index, .. } => {
            if let Some(SessionEvent::UserText { text, .. }) = events.get(*event_index) {
                let stdout = extract_local_command_stdout(text).unwrap_or(text.as_str());
                let _ = writeln!(s, "### ⎘ System output");
                let _ = writeln!(s);
                let _ = writeln!(s, "```");
                let _ = writeln!(s, "{}", redact_secrets(stdout.trim()));
                let _ = writeln!(s, "```");
                let _ = writeln!(s);
            }
        }
        SessionChunk::Compact { event_index, .. } => {
            if let Some(SessionEvent::Summary { text, .. }) = events.get(*event_index) {
                let _ = writeln!(s, "---");
                let _ = writeln!(s);
                let _ = writeln!(s, "### ⏵⏵ Compacted");
                let _ = writeln!(s);
                let _ = writeln!(s, "> {}", redact_secrets(text.trim()));
                let _ = writeln!(s);
                let _ = writeln!(s, "---");
                let _ = writeln!(s);
            }
        }
        SessionChunk::Ai {
            event_indices,
            tool_executions,
            ..
        } => {
            let _ = writeln!(s, "### 🤖 Assistant");
            let _ = writeln!(s);
            // Skip tool results whose linked call is in this chunk —
            // those render inside the call block.
            let absorbed_results: std::collections::HashSet<usize> = tool_executions
                .iter()
                .filter_map(|t| t.result_index)
                .collect();
            for &idx in event_indices {
                if absorbed_results.contains(&idx) {
                    continue;
                }
                let Some(ev) = events.get(idx) else { continue };
                render_ai_event(s, ev, tool_executions, idx);
            }
            let _ = writeln!(s);
        }
    }
}

fn render_ai_event(
    s: &mut String,
    ev: &SessionEvent,
    tools: &[crate::session_tool_link::LinkedTool],
    idx: usize,
) {
    match ev {
        SessionEvent::AssistantText { text, .. } => {
            let _ = writeln!(s, "{}", redact_secrets(text.trim()));
            let _ = writeln!(s);
        }
        SessionEvent::AssistantThinking { text, .. } => {
            let _ = writeln!(s, "<details><summary>💭 Thinking</summary>");
            let _ = writeln!(s);
            let _ = writeln!(s, "{}", redact_secrets(text.trim()));
            let _ = writeln!(s);
            let _ = writeln!(s, "</details>");
            let _ = writeln!(s);
        }
        SessionEvent::AssistantToolUse {
            tool_name,
            tool_use_id,
            input_preview,
            ..
        } => {
            // Look up matching linked tool.
            let linked = tools.iter().find(|t| t.call_index == idx);
            let error_badge = linked
                .map(|lt| if lt.is_error { " · ERROR" } else { "" })
                .unwrap_or("");
            let _ = writeln!(
                s,
                "<details><summary>🔧 {tool_name} <code>{id}</code>{error_badge}</summary>",
                tool_name = tool_name,
                id = short_id(tool_use_id),
            );
            let _ = writeln!(s);
            let _ = writeln!(s, "**Input**");
            let _ = writeln!(s);
            let _ = writeln!(s, "```json");
            let _ = writeln!(s, "{}", redact_secrets(input_preview));
            let _ = writeln!(s, "```");
            if let Some(lt) = linked {
                if let Some(result) = &lt.result_content {
                    let _ = writeln!(s);
                    let _ = writeln!(s, "**Result**");
                    let _ = writeln!(s);
                    let _ = writeln!(s, "```");
                    let _ = writeln!(s, "{}", redact_secrets(result));
                    let _ = writeln!(s, "```");
                }
                if let Some(d) = lt.duration_ms {
                    let _ = writeln!(s);
                    let _ = writeln!(s, "_Duration: {d} ms_");
                }
            }
            let _ = writeln!(s);
            let _ = writeln!(s, "</details>");
            let _ = writeln!(s);
        }
        _ => {}
    }
}

/// Pull the payload out of a `<local-command-stdout>...</local-command-stdout>`
/// wrapper, returning `None` when the input doesn't contain one. CC uses this
/// wrapper for slash-command output; the tag itself is metadata, the user
/// only wants to see the payload.
pub fn extract_local_command_stdout(text: &str) -> Option<&str> {
    const OPEN: &str = "<local-command-stdout>";
    const CLOSE: &str = "</local-command-stdout>";
    let start = text.find(OPEN)?;
    let body_start = start + OPEN.len();
    let rest = &text[body_start..];
    let close_rel = rest.find(CLOSE)?;
    Some(&rest[..close_rel])
}

fn short_id(s: &str) -> String {
    if s.len() > 8 {
        s[..8].to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Secret redaction
// ---------------------------------------------------------------------------

/// Truncate any `sk-ant-...` token found inside `text`. The mask keeps
/// the prefix and last four characters so readers can still tell the
/// tokens apart without being able to reuse them.
pub fn redact_secrets(text: &str) -> String {
    // Work on bytes so we can scan linearly. The token set we guard
    // against: `sk-ant-` followed by a base64url-ish run.
    let needle = "sk-ant-";
    if !text.contains(needle) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let bytes = text.as_bytes();
    while cursor < bytes.len() {
        if let Some(start) = find_from(bytes, cursor, needle.as_bytes()) {
            let tok_end = token_end(bytes, start);
            // Idempotency: the mask form is `sk-ant-***<last4>`, so
            // the `*` sentinel always sits immediately after the
            // `sk-ant-` prefix, with no token chars in between. If
            // any token chars were consumed before the `*`, this is a
            // real `sk-ant-realToken*` — redact instead of skipping.
            let prefix_end = start + needle.len();
            if tok_end == prefix_end && tok_end < bytes.len() && bytes[tok_end] == b'*' {
                let mask_end = skip_existing_mask(bytes, tok_end);
                out.push_str(&text[cursor..mask_end]);
                cursor = mask_end;
                continue;
            }
            out.push_str(&text[cursor..start]);
            let token = &text[start..tok_end];
            out.push_str(&mask(token));
            cursor = tok_end;
        } else {
            out.push_str(&text[cursor..]);
            break;
        }
    }
    out
}

fn skip_existing_mask(bytes: &[u8], from: usize) -> usize {
    // Consume the `*` run.
    let mut i = from;
    while i < bytes.len() && bytes[i] == b'*' {
        i += 1;
    }
    // Then the optional 4-char last4 suffix (alnum / - / _).
    while i < bytes.len() && is_token_char(bytes[i]) {
        i += 1;
    }
    i
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from > hay.len().saturating_sub(needle.len()) {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn token_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && is_token_char(bytes[i]) {
        i += 1;
    }
    i
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')
}

fn mask(token: &str) -> String {
    if token.len() <= 12 {
        return "sk-ant-***".to_string();
    }
    let last4 = &token[token.len() - 4..];
    format!("sk-ant-***{last4}")
}

fn redact_in_place(detail: &mut SessionDetail) {
    // Row-level strings that might accidentally echo credentials if a
    // user pasted one into their project path, branch name, or slug.
    // Redaction is idempotent and cheap — just run it everywhere the
    // JSON export serializes a String.
    let r = &mut detail.row;
    r.session_id = redact_secrets(&r.session_id);
    r.slug = redact_secrets(&r.slug);
    r.project_path = redact_secrets(&r.project_path);
    if let Some(p) = &mut r.first_user_prompt {
        *p = redact_secrets(p);
    }
    for m in &mut r.models {
        *m = redact_secrets(m);
    }
    if let Some(b) = &mut r.git_branch {
        *b = redact_secrets(b);
    }
    if let Some(v) = &mut r.cc_version {
        *v = redact_secrets(v);
    }
    if let Some(s) = &mut r.display_slug {
        *s = redact_secrets(s);
    }
    // Redact every free-form string field on every event variant.
    // Exhaustive match so future variants can't silently bypass — the
    // compiler will flag them when added.
    for ev in &mut detail.events {
        match ev {
            SessionEvent::UserText { text, .. }
            | SessionEvent::AssistantText { text, .. }
            | SessionEvent::AssistantThinking { text, .. }
            | SessionEvent::Summary { text, .. } => {
                *text = redact_secrets(text);
            }
            SessionEvent::UserToolResult { content, .. } => {
                *content = redact_secrets(content);
            }
            SessionEvent::AssistantToolUse {
                input_preview,
                input_full,
                tool_name,
                ..
            } => {
                *input_preview = redact_secrets(input_preview);
                // `input_full` is the untruncated tool-call JSON.
                // `export_json` serializes it verbatim, so any secret
                // that happens to live deeper than the 240-char preview
                // cap (e.g. inside a long Bash command or Edit body)
                // would leak unless we scrub it here too.
                *input_full = redact_secrets(input_full);
                // Tool names are tokens from CC (Read/Bash/…), not
                // user-controlled, but run them through the helper
                // anyway — costs nothing and closes the door on
                // future custom tool names that might echo secrets.
                *tool_name = redact_secrets(tool_name);
            }
            SessionEvent::System {
                detail, subtype, ..
            } => {
                *detail = redact_secrets(detail);
                if let Some(s) = subtype {
                    *s = redact_secrets(s);
                }
            }
            SessionEvent::Attachment { name, mime, .. } => {
                if let Some(s) = name {
                    *s = redact_secrets(s);
                }
                if let Some(s) = mime {
                    *s = redact_secrets(s);
                }
            }
            SessionEvent::Other { raw_type, .. } => {
                *raw_type = redact_secrets(raw_type);
            }
            SessionEvent::Malformed { error, preview, .. } => {
                *error = redact_secrets(error);
                *preview = redact_secrets(preview);
            }
            SessionEvent::FileHistorySnapshot { .. } => {}
            SessionEvent::TaskSummary { summary, .. } => {
                // The text is CC-generated and describes what the
                // agent is doing — nothing a user typed directly,
                // but a Bash step that echoed a secret could have
                // surfaced in the summary. Redact to match the
                // belt-and-braces posture on every other field.
                *summary = redact_secrets(summary);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "session_export_tests.rs"]
mod tests;

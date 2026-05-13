//! Extract tip objects from a CC binary.
//!
//! Pipeline:
//! 1. `resolve_cc_binary` — follow `~/.local/bin/claude` symlink to
//!    the versioned binary (or the platform equivalent).
//! 2. `extract_from_binary` — read the binary into memory, scan for
//!    `id:"<id>"` anchors, then for each candidate use the
//!    `walker::Walker` to find the enclosing `{...}` and parse the
//!    field set inside.
//!
//! The extractor never executes anything; it only reads. The output
//! is a `Vec<RawTip>` where prose is preserved with `${...}`
//! interpolations intact (rendering substitutes them later).

use crate::cc_tips::error::{TipsError, TipsResult};
use crate::cc_tips::walker::{Tok, Walker};
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One tip exactly as recovered from the binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawTip {
    pub id: String,
    /// The prose body. For arrow expressions returning a literal,
    /// this is that literal verbatim (template `${...}` preserved).
    /// For conditional content (A/B variants, platform branches,
    /// reward-presence branches), `prose_b` is the alternate branch.
    pub prose: String,
    pub prose_b: Option<String>,
    /// GrowthBook flag name when the conditional matches the
    /// `f_("<flag>","off")==="copy_b"` shape. None for non-experiment
    /// conditionals (platform / reward).
    pub experiment_flag: Option<String>,
    /// Best-effort plain-English label for what `prose` corresponds
    /// to under the conditional. Filled when we recognize the
    /// condition shape; otherwise None.
    pub condition_label: Option<String>,
    /// Best-effort plain-English label for `prose_b`.
    pub condition_label_b: Option<String>,
    pub cooldown_sessions: Option<u32>,
    /// Raw bytes of `isRelevant` body (everything between the
    /// outer `{}`), as a UTF-8 string. Empty if not present or
    /// declared as `=>boolean` shorthand.
    pub is_relevant_source: Option<String>,
    /// True iff the parser saw `providerAgnostic:!0`.
    pub provider_agnostic: bool,
}

/// Resolve the CC binary the user is running. Walks the
/// `~/.local/bin/claude` symlink chain. Falls back to a couple of
/// well-known versioned paths on macOS/Linux.
///
/// On Windows this returns `Err(BinaryNotFound)` — the path layout
/// hasn't been verified (per `dev-docs/cc-tips-ledger.md` §13).
pub fn resolve_cc_binary() -> TipsResult<PathBuf> {
    let home = dirs::home_dir().ok_or(TipsError::NoHome)?;

    // Primary: ~/.local/bin/claude → ~/.local/share/claude/versions/<v>
    let bin_link = home.join(".local/bin/claude");
    if let Ok(target) = std::fs::read_link(&bin_link) {
        let resolved = if target.is_absolute() {
            target
        } else {
            bin_link.parent().unwrap_or(Path::new("/")).join(target)
        };
        if resolved.is_file() {
            return Ok(resolved);
        }
    }

    // Fallback: scan ~/.local/share/claude/versions/* and pick the
    // newest.
    let versions = home.join(".local/share/claude/versions");
    if let Ok(entries) = std::fs::read_dir(&versions) {
        let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let m = e.metadata().ok()?.modified().ok()?;
                if p.is_file() {
                    Some((m, p))
                } else {
                    None
                }
            })
            .collect();
        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        if let Some((_, p)) = candidates.into_iter().next() {
            return Ok(p);
        }
    }

    Err(TipsError::BinaryNotFound {
        path: bin_link.to_string_lossy().into_owned(),
    })
}

/// Read the binary at `bin_path` and extract every tip object whose
/// id matches `[a-z][a-z0-9-]+`. Returns the list in source order.
/// Duplicates (same id seen twice in the binary) are deduped — first
/// occurrence wins.
pub fn extract_from_binary(bin_path: &Path) -> TipsResult<Vec<RawTip>> {
    let bytes = std::fs::read(bin_path).map_err(|source| TipsError::BinaryRead {
        path: bin_path.to_string_lossy().into_owned(),
        source,
    })?;
    Ok(extract_from_bytes(&bytes))
}

/// Pure function — extract tips from an in-memory byte slice. Safe
/// to call on synthetic fixtures.
pub fn extract_from_bytes(bytes: &[u8]) -> Vec<RawTip> {
    let anchor = Regex::new(r#"id:"([a-z][a-z0-9-]+)",(?:providerAgnostic:!0,)?content:async"#)
        .expect("static regex compiles");

    let mut out: Vec<RawTip> = Vec::new();
    let mut seen_ids: std::collections::HashSet<String> = Default::default();

    for cap in anchor.captures_iter(bytes) {
        let m = cap.get(0).unwrap();
        let id_match = cap.get(1).unwrap();
        let id = std::str::from_utf8(id_match.as_bytes())
            .unwrap_or("")
            .to_string();
        if id.is_empty() || seen_ids.contains(&id) {
            continue;
        }

        // Walk backward from the anchor a few bytes to find the `{`.
        // Pattern is `{id:"..."` or `,{id:"..."`. The `{` is at
        // m.start() - 1 if the previous byte is `{` or `,{`.
        let anchor_start = m.start();
        let brace_open = match find_open_brace_before(bytes, anchor_start) {
            Some(p) => p,
            None => continue,
        };

        // Walker positioned just after the `{`.
        let mut walker = Walker::from_offset(bytes, brace_open + 1);
        let close_end = match walker.find_matching_close(b'{') {
            Some(p) => p,
            None => continue,
        };
        // close_end is one-past-the-`}`. Object body is
        // bytes[brace_open+1 .. close_end-1].
        let body = &bytes[brace_open + 1..close_end - 1];
        let parsed = match parse_tip_body(body) {
            Some(p) => p,
            None => continue,
        };
        if parsed.id != id {
            // Anchor matched but parser disagreed — treat as
            // partial match, take the regex's id.
            // (Shouldn't happen unless the parser is wrong.)
        }
        seen_ids.insert(id.clone());
        out.push(RawTip {
            id,
            prose: parsed.prose,
            prose_b: parsed.prose_b,
            experiment_flag: parsed.experiment_flag,
            condition_label: parsed.condition_label,
            condition_label_b: parsed.condition_label_b,
            cooldown_sessions: parsed.cooldown,
            is_relevant_source: parsed.is_relevant,
            provider_agnostic: parsed.provider_agnostic,
        });
    }

    out
}

fn find_open_brace_before(bytes: &[u8], pos: usize) -> Option<usize> {
    if pos == 0 {
        return None;
    }
    // Walk backward up to a few bytes (whitespace tolerated).
    let lower = pos.saturating_sub(8);
    for i in (lower..pos).rev() {
        let b = bytes[i];
        if b == b'{' {
            return Some(i);
        }
        if !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b',') {
            // Non-trivial byte before `id:` — wrong context.
            return None;
        }
    }
    None
}

#[derive(Debug, Default)]
struct ParsedFields {
    id: String,
    provider_agnostic: bool,
    prose: String,
    prose_b: Option<String>,
    experiment_flag: Option<String>,
    condition_label: Option<String>,
    condition_label_b: Option<String>,
    cooldown: Option<u32>,
    is_relevant: Option<String>,
}

fn parse_tip_body(body: &[u8]) -> Option<ParsedFields> {
    let mut out = ParsedFields::default();
    let mut w = Walker::new(body);
    loop {
        w.skip_trivia();
        if w.pos() >= body.len() {
            break;
        }
        // Field name (identifier or string).
        let name = match read_field_name(&mut w) {
            Some(n) => n,
            None => break,
        };
        // Expect ':'.
        w.skip_trivia();
        let (t, _) = w.next_tok();
        if !matches!(t, Tok::Punct(b':')) {
            // Method shorthand: `async isRelevant() { ... }`.
            // `name` was actually `async`; the next ident is the method name.
            if name == "async" {
                if let Tok::Ident = t {
                    // Already consumed; treat current as method name.
                    let real_name = std::str::from_utf8(w.slice(span_of_prev(&w)))
                        .unwrap_or("")
                        .to_string();
                    consume_method_body(&mut w, &real_name, &mut out);
                } else {
                    break;
                }
            } else {
                break;
            }
        } else {
            // Regular `name: value`.
            consume_value(&mut w, &name, &mut out);
        }
        // After a value, expect `,` or end.
        w.skip_trivia();
        match w.next_tok() {
            (Tok::Comma, _) => continue,
            (Tok::Eof, _) => break,
            _ => break,
        }
    }
    if out.prose.is_empty() {
        return None;
    }
    Some(out)
}

fn read_field_name(w: &mut Walker) -> Option<String> {
    let (t, span) = w.next_tok();
    let bytes = w.slice(span);
    match t {
        Tok::Ident => Some(std::str::from_utf8(bytes).ok()?.to_string()),
        Tok::StringLit => {
            // Strip quotes.
            if bytes.len() >= 2 {
                Some(
                    std::str::from_utf8(&bytes[1..bytes.len() - 1])
                        .ok()?
                        .to_string(),
                )
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Helper: re-read the previous token's span from the walker by
/// rolling backward — not reliable. Instead, callers should track
/// span explicitly. We work around this in `parse_tip_body` by
/// detecting the `async <ident>` pattern via the field-name read
/// flow itself.
fn span_of_prev(_w: &Walker) -> crate::cc_tips::walker::Span {
    crate::cc_tips::walker::Span { start: 0, end: 0 }
}

fn consume_value(w: &mut Walker, name: &str, out: &mut ParsedFields) {
    w.skip_trivia();
    match name {
        "id" => {
            if let Some(s) = read_string_or_template(w) {
                out.id = s;
            }
        }
        "providerAgnostic" => {
            // `!0` (true) / `!1` (false). We tokenize as `Punct('!')`
            // then `Number(0|1)`. Just consume the next two tokens.
            let _ = w.next_tok();
            let _ = w.next_tok();
            out.provider_agnostic = true;
        }
        "cooldownSessions" => {
            let (t, span) = w.next_tok();
            if t == Tok::Number {
                let s = std::str::from_utf8(w.slice(span)).unwrap_or("");
                out.cooldown = s.parse::<u32>().ok();
            }
        }
        "content" => {
            parse_content_value(w, out);
        }
        "isRelevant" => {
            let raw = consume_arrow_or_function_body(w);
            out.is_relevant = Some(raw);
        }
        _ => {
            // Unknown field — skip a value.
            skip_one_value(w);
        }
    }
}

fn consume_method_body(w: &mut Walker, name: &str, out: &mut ParsedFields) {
    // Method shorthand: `<name>() { ... }`. Consume `(`, params, `)`,
    // then capture `{...}` as the body.
    w.skip_trivia();
    let (t, _) = w.next_tok();
    if !matches!(t, Tok::OpenParen) {
        return;
    }
    let _ = w.find_matching_close(b'(');
    w.skip_trivia();
    let (t2, span) = w.next_tok();
    if !matches!(t2, Tok::OpenBrace) {
        return;
    }
    let body_start = span.end;
    let body_end_excl = match w.find_matching_close(b'{') {
        Some(p) => p - 1,
        None => return,
    };
    let raw = std::str::from_utf8(&w.bytes()[body_start..body_end_excl])
        .unwrap_or("")
        .to_string();
    if name == "isRelevant" {
        out.is_relevant = Some(raw);
    }
}

fn read_string_or_template(w: &mut Walker) -> Option<String> {
    let (t, span) = w.next_tok();
    let bytes = w.slice(span);
    match t {
        Tok::StringLit => {
            if bytes.len() >= 2 {
                Some(unescape_js(&bytes[1..bytes.len() - 1]))
            } else {
                None
            }
        }
        Tok::TemplateLit => {
            if bytes.len() >= 2 {
                Some(unescape_template(&bytes[1..bytes.len() - 1]))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Unescape a template-literal body, preserving `${...}` interpolations
/// verbatim. JS evaluates `\xNN` / `\uNNNN` / `\u{N+}` inside templates;
/// `${...}` segments are expression placeholders and stay untouched.
fn unescape_template(b: &[u8]) -> String {
    let s = std::str::from_utf8(b).unwrap_or("");
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Find matching `}` honoring nested braces.
            let mut depth = 1;
            let mut j = i + 2;
            while j < bytes.len() {
                let bb = bytes[j];
                if bb == b'{' {
                    depth += 1;
                } else if bb == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                j += 1;
            }
            out.push_str(&s[i..j]);
            i = j;
            continue;
        }
        i += 1;
        out.push(c as char);
    }
    // Now run unescape over the non-`${...}` segments. Easiest: split
    // on `${...}` boundaries.
    let mut final_out = String::with_capacity(out.len());
    let mut k = 0;
    let chars = out.as_bytes();
    while k < chars.len() {
        if chars[k] == b'$' && k + 1 < chars.len() && chars[k + 1] == b'{' {
            // Skip placeholder verbatim.
            let mut depth = 1;
            let mut j = k + 2;
            while j < chars.len() {
                if chars[j] == b'{' {
                    depth += 1;
                } else if chars[j] == b'}' {
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                }
                j += 1;
            }
            final_out.push_str(&out[k..j]);
            k = j;
        } else {
            // Find next `${` or end.
            let next = out[k..].find("${").map(|p| k + p).unwrap_or(out.len());
            let segment = &out[k..next];
            final_out.push_str(&unescape_js(segment.as_bytes()));
            k = next;
        }
    }
    final_out
}

fn unescape_js(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len());
    let s = std::str::from_utf8(b).unwrap_or("");
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some('`') => out.push('`'),
                Some('$') => out.push('$'),
                Some('x') => {
                    let h1 = chars.next();
                    let h2 = chars.next();
                    let pair: String = [h1, h2].iter().flatten().collect();
                    if let Ok(n) = u32::from_str_radix(&pair, 16) {
                        if let Some(ch) = char::from_u32(n) {
                            out.push(ch);
                            continue;
                        }
                    }
                    out.push('\\');
                    out.push('x');
                    out.push_str(&pair);
                }
                Some('u') => {
                    if chars.peek() == Some(&'{') {
                        chars.next();
                        let mut hex = String::new();
                        while let Some(&c) = chars.peek() {
                            if c == '}' {
                                chars.next();
                                break;
                            }
                            hex.push(c);
                            chars.next();
                        }
                        if let Ok(n) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = char::from_u32(n) {
                                out.push(ch);
                                continue;
                            }
                        }
                        out.push_str(&format!("\\u{{{hex}}}"));
                    } else {
                        let h: String = chars.by_ref().take(4).collect();
                        if let Ok(n) = u32::from_str_radix(&h, 16) {
                            if let Some(ch) = char::from_u32(n) {
                                out.push(ch);
                                continue;
                            }
                        }
                        out.push('\\');
                        out.push('u');
                        out.push_str(&h);
                    }
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_content_value(w: &mut Walker, out: &mut ParsedFields) {
    // `content:` is followed by an arrow function. Possible shapes:
    //   async()=>"..."
    //   async()=>`...`
    //   async()=><cond>?<lit_a>:<lit_b>
    //   async(H)=>{ let q=...; return f_(...,"off")==="copy_b"?A:B }
    //   async(H)=>{ return `...` }
    //
    // Strategy: skip `async`, optional `(...)`, `=>`. Capture whole
    // body expression up to top-level `,`. Then `analyze_expression`
    // handles literal / ternary / arrow-block.
    w.skip_trivia();
    let saved = w.pos();
    let (t1, span1) = w.next_tok();
    let kw = w.slice(span1);
    if !(matches!(t1, Tok::Ident) && (kw == b"async" || kw == b"function")) {
        w.set_pos(saved);
        if let Some(s) = read_string_or_template(w) {
            out.prose = strip_helpers_keep_dollars(&s);
        }
        return;
    }
    w.skip_trivia();
    let saved2 = w.pos();
    let (t2, _) = w.next_tok();
    match t2 {
        Tok::OpenParen => {
            let _ = w.find_matching_close(b'(');
        }
        Tok::Ident => {
            // single-param identifier
        }
        _ => w.set_pos(saved2),
    }
    w.skip_trivia();
    let (t3, _) = w.next_tok();
    if !matches!(t3, Tok::Arrow) {
        return;
    }
    w.skip_trivia();
    let body_start = w.pos();
    let body_end = capture_expression_end(w);
    let body = &w.bytes()[body_start..body_end];
    analyze_expression(body, out);
}

/// Walk `w` forward until a top-level `,` or end-of-body. Returns
/// the byte offset *of* the comma (or the EOF position). The walker
/// is left at that offset (caller can consume the `,`).
fn capture_expression_end(w: &mut Walker) -> usize {
    loop {
        let saved = w.pos();
        let (t, _) = w.next_tok();
        match t {
            Tok::Eof => return w.pos(),
            Tok::Comma => {
                w.set_pos(saved);
                return saved;
            }
            Tok::OpenBrace => {
                let _ = w.find_matching_close(b'{');
            }
            Tok::OpenParen => {
                let _ = w.find_matching_close(b'(');
            }
            Tok::OpenBracket => {
                let _ = w.find_matching_close(b']');
            }
            _ => {}
        }
    }
}

/// Analyze a captured content expression. Recognizes:
///  1. arrow-block `{ ... return <expr> }` — recurses into the
///     return expression.
///  2. GrowthBook A/B `f_("<flag>","off")==="copy_b"?A:B`.
///  3. Generic literal-ternary `<cond>?<lit>:<lit>`.
///  4. Plain string or template literal.
///
/// Anything more elaborate is dropped to a fallback "plain literal"
/// attempt; if even that fails, prose stays empty.
fn analyze_expression(expr: &[u8], out: &mut ParsedFields) {
    // 1. Arrow-block: starts with `{`. Find return.
    {
        let mut w = Walker::new(expr);
        w.skip_trivia();
        let saved = w.pos();
        let (t, _) = w.next_tok();
        if matches!(t, Tok::OpenBrace) {
            let body_start = w.pos();
            let Some(body_end_excl) = w.find_matching_close(b'{') else {
                return;
            };
            let block = &expr[body_start..body_end_excl - 1];
            // Find `return` at top level inside block.
            if let Some(return_expr) = find_return_expression(block) {
                analyze_expression(return_expr, out);
            }
            return;
        }
        let _ = saved;
    }

    let expr_str = std::str::from_utf8(expr).unwrap_or("");

    // 2. GrowthBook A/B.
    if let Some((flag, branch_b, branch_a)) = split_copy_b_ternary(expr_str) {
        out.experiment_flag = Some(flag);
        out.prose = strip_helpers_keep_dollars(&branch_b);
        out.condition_label = Some("Experiment variant B".to_string());
        out.prose_b = Some(strip_helpers_keep_dollars(&branch_a));
        out.condition_label_b = Some("Experiment variant A".to_string());
        return;
    }

    // 3. Generic literal-ternary.
    if let Some((cond, branch_t, branch_f)) = split_literal_ternary(expr) {
        out.prose = strip_helpers_keep_dollars(&branch_t);
        out.condition_label = Some(label_for_condition(&cond, true));
        out.prose_b = Some(strip_helpers_keep_dollars(&branch_f));
        out.condition_label_b = Some(label_for_condition(&cond, false));
        return;
    }

    // 4. Plain literal.
    let mut lw = Walker::new(expr);
    lw.skip_trivia();
    if let Some(s) = read_string_or_template(&mut lw) {
        out.prose = strip_helpers_keep_dollars(&s);
    }
}

fn find_return_expression(block: &[u8]) -> Option<&[u8]> {
    let mut w = Walker::new(block);
    let mut return_end: Option<usize> = None;
    loop {
        w.skip_trivia();
        let (t, span) = w.next_tok();
        match t {
            Tok::Eof => break,
            Tok::Ident if w.slice(span) == b"return" => {
                return_end = Some(span.end);
            }
            Tok::OpenBrace => {
                let _ = w.find_matching_close(b'{');
            }
            Tok::OpenParen => {
                let _ = w.find_matching_close(b'(');
            }
            Tok::OpenBracket => {
                let _ = w.find_matching_close(b']');
            }
            _ => {}
        }
    }
    let rp = return_end?;
    let mut rw = Walker::from_offset(block, rp);
    rw.skip_trivia();
    let expr_start = rw.pos();
    // Walk until `;` or EOF at depth 0.
    let expr_end;
    loop {
        let saved = rw.pos();
        let (t, _) = rw.next_tok();
        match t {
            Tok::Eof => {
                expr_end = block.len();
                break;
            }
            Tok::Semi => {
                expr_end = saved;
                break;
            }
            Tok::OpenBrace => {
                let _ = rw.find_matching_close(b'{');
            }
            Tok::OpenParen => {
                let _ = rw.find_matching_close(b'(');
            }
            Tok::OpenBracket => {
                let _ = rw.find_matching_close(b']');
            }
            _ => {}
        }
    }
    Some(&block[expr_start..expr_end])
}

fn split_copy_b_ternary(expr: &str) -> Option<(String, String, String)> {
    let copy_b_idx = expr.find("===\"copy_b\"")?;
    let pre = &expr[..copy_b_idx];
    let flag_re = Regex::new(r#"\("([a-z_][a-z0-9_]*)","off"\)"#).ok()?;
    let flag = flag_re
        .captures(pre.as_bytes())
        .and_then(|c| c.get(1))
        .map(|m| std::str::from_utf8(m.as_bytes()).unwrap_or("").to_string())?;

    let after = expr[copy_b_idx + "===\"copy_b\"".len()..].trim_start();
    let after = after.strip_prefix('?')?.trim_start();
    let (branch_a, rest) = read_one_branch(after)?;
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let (branch_b, _) = read_one_branch(rest)?;
    Some((flag, branch_a, branch_b))
}

/// Detect a top-level ternary whose two branches are both string or
/// template literals: `<cond>?<lit_a>:<lit_b>`. Returns
/// `(condition_text, branch_a, branch_b)`.
fn split_literal_ternary(expr: &[u8]) -> Option<(String, String, String)> {
    // Find a top-level `?`. Skip nested parens/brackets/braces, and
    // ignore `?` inside strings/templates/regex.
    let q_pos = top_level_index(expr, b'?')?;
    // The byte after `?` should (eventually) be a literal.
    let cond = std::str::from_utf8(&expr[..q_pos]).ok()?.trim().to_string();
    let after = &expr[q_pos + 1..];
    let after_str = std::str::from_utf8(after).ok()?.trim_start();
    let (branch_a, rest) = read_one_branch(after_str)?;
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let (branch_b, _) = read_one_branch(rest)?;
    Some((cond, branch_a, branch_b))
}

/// Locate the first top-level occurrence of `target_byte`. "Top
/// level" = depth 0 of all bracketing constructs and not inside any
/// literal.
fn top_level_index(bytes: &[u8], target: u8) -> Option<usize> {
    let mut w = Walker::new(bytes);
    loop {
        let saved = w.pos();
        let (t, span) = w.next_tok();
        match t {
            Tok::Eof => return None,
            Tok::OpenBrace => {
                let _ = w.find_matching_close(b'{');
            }
            Tok::OpenParen => {
                let _ = w.find_matching_close(b'(');
            }
            Tok::OpenBracket => {
                let _ = w.find_matching_close(b']');
            }
            Tok::Punct(b) if b == target => {
                return Some(span.start);
            }
            _ => {
                let _ = saved;
            }
        }
    }
}

/// Best-effort plain-English label for a ternary condition. Targets
/// the patterns CC actually uses; falls back to the raw condition.
fn label_for_condition(cond: &str, branch_true: bool) -> String {
    let c = cond.trim();
    if let Some((_, terminal_lit)) = c.split_once("===") {
        let lit = terminal_lit
            .trim()
            .trim_matches(|x: char| x == '"' || x == '\'');
        if c.contains("terminal") {
            return if branch_true {
                format!("When terminal is {lit}")
            } else {
                format!("When terminal is not {lit}")
            };
        }
    }
    if branch_true {
        format!("When {c}")
    } else {
        "Otherwise".to_string()
    }
}

fn read_one_branch(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    let mut w = Walker::new(bytes);
    w.skip_trivia();
    let (t, span) = w.next_tok();
    let raw = w.slice(span);
    match t {
        Tok::StringLit => {
            if raw.len() >= 2 {
                let body = &raw[1..raw.len() - 1];
                Some((unescape_js(body), &s[span.end..]))
            } else {
                None
            }
        }
        Tok::TemplateLit => {
            if raw.len() >= 2 {
                let body = &raw[1..raw.len() - 1];
                Some((unescape_template(body), &s[span.end..]))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Strip `Kq("theme",H.theme)("text")` color-helper calls down to
/// `text`, while preserving `${...}` interpolations for shortcut and
/// helper resolution at render time.
fn strip_helpers_keep_dollars(s: &str) -> String {
    // Replace `${ <ident>("<text>") }` where outer ident is a
    // single-letter color helper produced by Bun minification.
    // Pattern: `${[A-Za-z_$][A-Za-z0-9_$]*\(`...`\)}` — we don't
    // unwrap helper calls here; we leave that to render-time.
    s.to_string()
}

fn consume_arrow_or_function_body(w: &mut Walker) -> String {
    // Could be: `async()=>...`, `async H=>...`, `function(){...}`,
    // or an arrow expression. We capture everything until the next
    // top-level `,` or end of object.
    let start = w.pos();
    loop {
        let saved = w.pos();
        let (t, _) = w.next_tok();
        match t {
            Tok::Eof => {
                let raw = std::str::from_utf8(&w.bytes()[start..w.pos()])
                    .unwrap_or("")
                    .to_string();
                return raw;
            }
            Tok::Comma => {
                // Top-level comma — stop.
                w.set_pos(saved);
                let raw = std::str::from_utf8(&w.bytes()[start..saved])
                    .unwrap_or("")
                    .to_string();
                return raw;
            }
            Tok::OpenBrace => {
                let _ = w.find_matching_close(b'{');
            }
            Tok::OpenParen => {
                let _ = w.find_matching_close(b'(');
            }
            Tok::OpenBracket => {
                let _ = w.find_matching_close(b']');
            }
            _ => {}
        }
    }
}

fn skip_one_value(w: &mut Walker) {
    let _ = consume_arrow_or_function_body(w);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_simple_string_tip() {
        let src = br#",{id:"new-user-warmup",providerAgnostic:!0,content:async()=>"Start with small features",cooldownSessions:3,async isRelevant(){return C_().numStartups<10}},"#;
        let tips = extract_from_bytes(src);
        assert_eq!(tips.len(), 1);
        assert_eq!(tips[0].id, "new-user-warmup");
        assert_eq!(tips[0].prose, "Start with small features");
        assert_eq!(tips[0].cooldown_sessions, Some(3));
        assert!(tips[0].provider_agnostic);
    }

    #[test]
    fn extract_template_tip() {
        let src = br#",{id:"plan-mode-for-complex-tasks",content:async()=>`Press ${Mf("chat:cycleMode","Chat","shift+tab")} twice`,cooldownSessions:5,isRelevant:async()=>!0},"#;
        let tips = extract_from_bytes(src);
        assert_eq!(tips.len(), 1);
        assert!(tips[0].prose.contains("${Mf("));
    }

    #[test]
    fn extract_ab_variant_tip() {
        let src = br#",{id:"effort-high-nudge",content:async(H)=>{let q=Kq("suggestion",H.theme)("/effort high");return f_("tengu_tide_elm","off")==="copy_b"?`Use ${q} for better one-shot answers`:`Working on something tricky? ${q} gives better first answers`},cooldownSessions:3,isRelevant:async()=>!0},"#;
        let tips = extract_from_bytes(src);
        assert_eq!(tips.len(), 1);
        assert_eq!(tips[0].id, "effort-high-nudge");
        assert_eq!(tips[0].experiment_flag.as_deref(), Some("tengu_tide_elm"));
        assert!(tips[0].prose.contains("better one-shot"));
        assert!(tips[0].prose_b.as_deref().unwrap().contains("tricky"));
    }

    #[test]
    fn dedupes_repeated_ids() {
        let src = br#",{id:"foo",content:async()=>"a",cooldownSessions:1,isRelevant:async()=>!0},,{id:"foo",content:async()=>"b",cooldownSessions:1,isRelevant:async()=>!0},"#;
        let tips = extract_from_bytes(src);
        assert_eq!(tips.len(), 1);
        assert_eq!(tips[0].prose, "a");
    }

    #[test]
    fn handles_regex_in_isrelevant() {
        // Critical test: filePath:/\.html$/i contains a slash that
        // must not break parsing.
        let src = br#",{id:"frontend-design-plugin",content:async()=>"hi",cooldownSessions:3,isRelevant:async(H)=>MCK("frontend-design",H,{filePath:/\.html$/i})},"#;
        let tips = extract_from_bytes(src);
        assert_eq!(tips.len(), 1);
        assert_eq!(tips[0].id, "frontend-design-plugin");
    }
}

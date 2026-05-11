//! Spawn `claude doctor` under a pty, capture its output, replay it
//! through a minimal terminal emulator, and parse the rendered grid
//! into a structured snapshot.
//!
//! ## Why a pty
//!
//! `claude doctor` is an Ink/React TUI. It detects pipe-mode (no
//! TTY) and short-circuits without rendering — running it under
//! `Command::output()` yields zero bytes. We allocate a real pty
//! via `portable-pty` so Ink renders normally and we capture
//! everything.
//!
//! ## Why an in-house terminal emulator (and not `vt100`)
//!
//! Ink redraws sections in place: a "Checking installation status…"
//! placeholder is overwritten by the Diagnostics block via cursor-up
//! + clear-line. A naïve concat of pty output would carry the stale
//! placeholder. We need a grid replay to see what the user actually
//! saw. `vt100` would work but adds ~600 KB of compile + drops bold
//! annotations on row extraction; tracking just the four ESC
//! sequences Ink actually emits (`[<n>C`, `[<n>A`, `[K`, `[1m/[22m`)
//! is ~200 lines of focused code and keeps bold-on-cell as a
//! first-class signal for section-header detection.
//!
//! ## Severity heuristic
//!
//! `figures.cross` (`✘ U+2718`) leads a section header → error. The
//! `figures.warning` triangle (`⚠ U+26A0`) leads → warning. No marker
//! → informational. Aggregate snapshot severity is the max across
//! sections; empty/unparseable output is `Warning` so the pill
//! flips off green without escalating to red.

use chrono::Utc;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::cc_doctor::parse_failures::record_parse_failure;

/// Hard wall-clock cap on the whole scrape (spawn + read + parse).
/// `claude doctor`'s synchronous sections render in ~1s; the live
/// npm dist-tag fetch in the Updates section is the only async wait
/// and usually completes in 3–5s. 15s allows for a slow network
/// without leaving a half-rendered grid in the cache.
const SCRAPE_TIMEOUT: Duration = Duration::from_secs(15);

/// Cap on bytes captured from the pty. Real fixtures are ~3 KB; a
/// 256 KB cap defends against runaway output (e.g., a future CC
/// version that streams a multi-MB log dump) without truncating
/// realistic doctor renders.
const MAX_CAPTURE_BYTES: usize = 256 * 1024;

/// Terminator the user would have to type after `claude doctor`
/// finishes rendering. Seeing this in the rendered grid means the
/// async sections are done and Ink is parked on the input prompt —
/// safe to stop reading.
const READY_PROMPT_MARKER: &str = "Enter to continue";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorSeverity {
    Healthy,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSnapshot {
    /// e.g. `"2.1.138"`. Extracted from the "Currently running" line
    /// when present; `None` when the parser couldn't find it (the
    /// pill should still light up degraded rather than blank).
    pub cc_version: Option<String>,
    /// e.g. `"native"`, `"npm-global"`. From the "Currently running"
    /// line's parenthesized-prefix or "Config install method" row.
    pub install_type: Option<String>,
    /// Absolute filesystem path to the running claude binary. From
    /// the "Path:" row when present.
    pub install_path: Option<String>,
    /// Aggregate signal for the WindowChrome pill — max of section
    /// severities. `Healthy` only when every parsed section is
    /// informational and parse_status is `Ok`.
    pub severity: DoctorSeverity,
    /// Sections in the order CC rendered them. `Diagnostics` is
    /// always first when the parse succeeded; everything after
    /// depends on which optional sections fired (Updates, Version
    /// locks, plugin errors, etc.).
    pub sections: Vec<DoctorSection>,
    /// Total bytes captured from the pty. Useful in the dev-alert
    /// payload — a sub-200-byte capture is a stronger fail signal
    /// than a 3 KB capture that parsed cleanly.
    pub raw_bytes: usize,
    /// Whether the parser is confident in the result. The renderer
    /// uses this to decide between "show this snapshot" and "fall
    /// back to last-known-good".
    pub parse_status: ParseStatus,
    /// Wall-clock millis when the scrape started.
    pub captured_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSection {
    pub title: String,
    pub severity: DoctorSeverity,
    pub entries: Vec<SectionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionEntry {
    /// Cleaned text content of the row — leading 2-space indent and
    /// the tree-character prefix are stripped.
    pub text: String,
    /// `"├"` or `"└"`. Lets the renderer reconstruct visual nesting
    /// without re-parsing.
    pub tree_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ParseStatus {
    /// At least one section parsed and the "Diagnostics" header was
    /// seen.
    Ok,
    /// Capture succeeded but parsing produced fewer than 1 sections
    /// or didn't find the Diagnostics header — partial signal.
    Degraded { reason: String },
    /// Capture or parse failed outright. Snapshot should be treated
    /// as advisory only; renderer should keep the previous one.
    Failed { reason: String },
}

/// Public entry point. Spawns `claude doctor` in a pty, captures
/// output for up to [`SCRAPE_TIMEOUT`], parses it, and records a
/// parse-failure entry if the result is `Degraded` or `Failed` (so
/// the developer notification can fire and the failure is forensic-
/// preserved).
///
/// Pure function semantics from the caller's perspective: idempotent
/// to call (modulo CC's own state changing between invocations).
/// Blocks the calling thread for the pty read — callers should
/// run it on a dedicated tokio thread (`spawn_blocking`).
pub fn scrape_doctor() -> DoctorSnapshot {
    let captured_at_ms = Utc::now().timestamp_millis();

    let raw = match spawn_and_capture(SCRAPE_TIMEOUT) {
        Ok(b) => b,
        Err(e) => {
            let reason = format!("pty spawn/capture: {e}");
            let snap = DoctorSnapshot {
                cc_version: None,
                install_type: None,
                install_path: None,
                severity: DoctorSeverity::Warning,
                sections: Vec::new(),
                raw_bytes: 0,
                parse_status: ParseStatus::Failed {
                    reason: reason.clone(),
                },
                captured_at_ms,
            };
            record_parse_failure(&snap, b"", &reason);
            return snap;
        }
    };

    let raw_bytes = raw.len();
    let grid = render(&raw);
    let lines = grid_to_lines(&grid);
    let (sections, parse_status) = parse_lines(&lines);
    let (cc_version, install_type, install_path) = extract_install_info(&sections);
    let severity = aggregate_severity(&sections, &parse_status);

    let snapshot = DoctorSnapshot {
        cc_version,
        install_type,
        install_path,
        severity,
        sections,
        raw_bytes,
        parse_status: parse_status.clone(),
        captured_at_ms,
    };

    if !matches!(parse_status, ParseStatus::Ok) {
        let reason = match &parse_status {
            ParseStatus::Degraded { reason } | ParseStatus::Failed { reason } => reason.clone(),
            ParseStatus::Ok => unreachable!(),
        };
        record_parse_failure(&snapshot, &raw, &reason);
    }

    snapshot
}

// ─── Spawn + capture ─────────────────────────────────────────────

fn spawn_and_capture(timeout: Duration) -> std::io::Result<Vec<u8>> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| std::io::Error::other(format!("openpty: {e}")))?;

    let cmd = CommandBuilder::new("claude");
    let mut child = pair
        .slave
        .spawn_command({
            let mut c = cmd;
            c.arg("doctor");
            c
        })
        .map_err(|e| std::io::Error::other(format!("spawn claude doctor: {e}")))?;

    // Reader thread drains the pty master into a Vec<u8>. Polling
    // from the main thread would block on read() and prevent the
    // timeout from firing.
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| std::io::Error::other(format!("clone pty reader: {e}")))?;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let start = Instant::now();
    let mut captured: Vec<u8> = Vec::new();
    let mut sent_continue = false;
    loop {
        if start.elapsed() >= timeout {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(chunk) => {
                captured.extend_from_slice(&chunk);
                if captured.len() > MAX_CAPTURE_BYTES {
                    captured.truncate(MAX_CAPTURE_BYTES);
                    break;
                }
                // Once we see Ink's ready-prompt marker, the
                // sync+async sections have all rendered. Send Enter
                // so the child exits cleanly rather than hitting
                // our timeout (also avoids a 15s blocking wait
                // every refresh).
                if !sent_continue && contains_marker(&captured, READY_PROMPT_MARKER) {
                    if let Ok(mut writer) = pair.master.take_writer() {
                        let _ = std::io::Write::write_all(&mut writer, b"\r");
                    }
                    sent_continue = true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Best-effort kill if the child is still running past our cap.
    if let Ok(None) = child.try_wait() {
        let _ = child.kill();
    }

    Ok(captured)
}

fn contains_marker(buf: &[u8], needle: &str) -> bool {
    // The marker is plain ASCII so the byte-level search works
    // through any UTF-8 surroundings. Lossy stringification would
    // re-walk the whole buffer per chunk — slower, no extra signal.
    buf.windows(needle.len()).any(|w| w == needle.as_bytes())
}

// ─── Terminal replay ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    ch: char,
    bold: bool,
}

#[derive(Debug, Default)]
struct Grid {
    rows: Vec<Vec<Cell>>,
    cursor_row: usize,
    cursor_col: usize,
    bold: bool,
}

impl Grid {
    fn ensure_row(&mut self, row: usize) {
        while self.rows.len() <= row {
            self.rows.push(Vec::new());
        }
    }

    fn ensure_col(&mut self, row: usize, col: usize) {
        self.ensure_row(row);
        let r = &mut self.rows[row];
        while r.len() <= col {
            r.push(Cell {
                ch: ' ',
                bold: false,
            });
        }
    }

    fn put(&mut self, ch: char) {
        self.ensure_col(self.cursor_row, self.cursor_col);
        self.rows[self.cursor_row][self.cursor_col] = Cell {
            ch,
            bold: self.bold,
        };
        self.cursor_col += 1;
    }

    fn newline(&mut self) {
        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    fn cursor_forward(&mut self, n: usize) {
        self.cursor_col = self.cursor_col.saturating_add(n);
    }

    fn cursor_up(&mut self, n: usize) {
        self.cursor_row = self.cursor_row.saturating_sub(n);
    }

    fn cursor_down(&mut self, n: usize) {
        self.cursor_row = self.cursor_row.saturating_add(n);
    }

    fn cursor_back(&mut self, n: usize) {
        self.cursor_col = self.cursor_col.saturating_sub(n);
    }

    fn clear_to_eol(&mut self) {
        if self.cursor_row < self.rows.len() {
            let row = &mut self.rows[self.cursor_row];
            row.truncate(self.cursor_col);
        }
    }
}

fn render(bytes: &[u8]) -> Grid {
    let mut grid = Grid::default();
    // UTF-8 decode incrementally; invalid bytes get the replacement
    // char which is fine for our parser (we drop them when reading
    // rows).
    let s = String::from_utf8_lossy(bytes);
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\x1b' => handle_escape(&mut chars, &mut grid),
            '\r' => grid.carriage_return(),
            '\n' => grid.newline(),
            '\x08' => grid.cursor_back(1),
            '\x07' => {} // bell
            c if c.is_control() => {} // drop other C0
            c => grid.put(c),
        }
    }

    grid
}

/// Handle a single `\x1B`-led escape sequence.
///
/// Recognized:
/// - `[<params>m`   — SGR. Track bold on (`1`) / off (`22`); ignore
///   color, dim, italic, underline.
/// - `[<n>A/B/C/D` — cursor up/down/forward/back, default 1.
/// - `[K`           — clear from cursor to end of line.
/// - `[<n>;<n>H`    — absolute cursor move.
/// - `[?...h/l`     — DEC private mode set/reset (ignored).
/// - `[<...>r/q/t/c/u/4m` — assorted queries / scroll regions
///   (ignored).
/// - `]<...>\x07`   — OSC string (ignored).
/// - Anything else  — best-effort drain until terminator letter.
fn handle_escape(chars: &mut std::iter::Peekable<std::str::Chars>, grid: &mut Grid) {
    let Some(next) = chars.next() else {
        return;
    };
    match next {
        '[' => handle_csi(chars, grid),
        ']' => {
            // OSC — drain until BEL (\x07) or ESC \.
            while let Some(c) = chars.next() {
                if c == '\x07' {
                    return;
                }
                if c == '\x1b' {
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    return;
                }
            }
        }
        '7' | '8' => {} // DECSC / DECRC — save/restore cursor; ignore
        c if c.is_ascii_alphanumeric() => {} // single-char escape
        _ => {} // best-effort drop
    }
}

fn handle_csi(chars: &mut std::iter::Peekable<std::str::Chars>, grid: &mut Grid) {
    // CSI = `\x1B[` (already consumed). Format:
    //   ( '?' | '>' | '<' )? <params> <intermediates>? <final-byte>
    // Params are ASCII digits and `;`. Final byte is in 0x40..=0x7E.
    let mut prefix = String::new();
    let mut params = String::new();

    if let Some(&c) = chars.peek() {
        if c == '?' || c == '>' || c == '<' {
            prefix.push(c);
            chars.next();
        }
    }

    loop {
        match chars.peek() {
            Some(&c) if c.is_ascii_digit() || c == ';' => {
                params.push(c);
                chars.next();
            }
            Some(&c) if c.is_ascii_alphabetic() => {
                chars.next();
                apply_csi(prefix.as_str(), &params, c, grid);
                return;
            }
            Some(_) => {
                // Intermediate byte (space / `!"#$%&'()*+,-./` etc.).
                // Drain and continue until we hit the terminator.
                chars.next();
            }
            None => return,
        }
    }
}

fn apply_csi(prefix: &str, params: &str, terminator: char, grid: &mut Grid) {
    let parts: Vec<u32> = if params.is_empty() {
        Vec::new()
    } else {
        params
            .split(';')
            .map(|p| p.parse::<u32>().unwrap_or(0))
            .collect()
    };
    let n = |i: usize, default: u32| parts.get(i).copied().unwrap_or(default).max(1) as usize;

    // DEC private modes — ignore entirely.
    if prefix == "?" || prefix == ">" || prefix == "<" {
        return;
    }

    match terminator {
        'A' => grid.cursor_up(n(0, 1)),
        'B' => grid.cursor_down(n(0, 1)),
        'C' => grid.cursor_forward(n(0, 1)),
        'D' => grid.cursor_back(n(0, 1)),
        'H' | 'f' => {
            // Cursor position — 1-indexed in the protocol; clamp to 0.
            let row = n(0, 1).saturating_sub(1);
            let col = parts.get(1).copied().unwrap_or(1).max(1) as usize - 1;
            grid.cursor_row = row;
            grid.cursor_col = col;
        }
        'K' => grid.clear_to_eol(),
        'm' => {
            // SGR. Iterate params, track bold only.
            let mut iter = parts.iter().copied();
            if parts.is_empty() {
                grid.bold = false; // bare `[m` = reset
            }
            while let Some(p) = iter.next() {
                match p {
                    0 => grid.bold = false,
                    1 => grid.bold = true,
                    22 => grid.bold = false,
                    38 | 48 => {
                        // Extended color: `38;2;R;G;B` or `38;5;n` —
                        // consume the format byte and its args.
                        if let Some(fmt) = iter.next() {
                            let consume = match fmt {
                                2 => 3, // RGB
                                5 => 1, // palette index
                                _ => 0,
                            };
                            for _ in 0..consume {
                                iter.next();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {} // J (erase display), c (DA), etc. — ignore
    }
}

// ─── Grid → lines ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct RenderedLine {
    /// Text with trailing-space pruned.
    text: String,
    /// `true` when the first non-whitespace run on this row has the
    /// bold attribute set — used as the section-header signal.
    leads_bold: bool,
}

fn grid_to_lines(grid: &Grid) -> Vec<RenderedLine> {
    grid.rows
        .iter()
        .map(|row| {
            let text: String = row.iter().map(|c| c.ch).collect();
            let trimmed_end = text.trim_end().to_string();
            let leads_bold = row
                .iter()
                .find(|c| !c.ch.is_whitespace())
                .map(|c| c.bold)
                .unwrap_or(false);
            RenderedLine {
                text: trimmed_end,
                leads_bold,
            }
        })
        .collect()
}

// ─── Lines → sections ─────────────────────────────────────────────

const CROSS_FIGURE: char = '\u{2718}'; // ✘
const WARNING_FIGURE: char = '\u{26A0}'; // ⚠
const TREE_BRANCH: char = '\u{251C}'; // ├
const TREE_LAST: char = '\u{2514}'; // └

fn parse_lines(lines: &[RenderedLine]) -> (Vec<DoctorSection>, ParseStatus) {
    let mut sections: Vec<DoctorSection> = Vec::new();
    let mut saw_diagnostics = false;
    let mut current: Option<DoctorSection> = None;

    for line in lines {
        let trimmed = line.text.trim_start();
        if trimmed.is_empty() {
            continue;
        }

        // Section header: bold attribute on the first non-space run,
        // OR a leading severity figure (✘/⚠) — Ink renders the cross
        // with `color="error"` but doesn't always extend the `bold`
        // attribute to the figure cell itself, so bold-only detection
        // misses error headers.
        let leads_severity_figure =
            trimmed.starts_with(CROSS_FIGURE) || trimmed.starts_with(WARNING_FIGURE);
        if (line.leads_bold || leads_severity_figure)
            && !trimmed.starts_with(TREE_BRANCH)
            && !trimmed.starts_with(TREE_LAST)
        {
            if let Some(s) = current.take() {
                sections.push(s);
            }
            let (title, severity) = classify_header(trimmed);
            if title.eq_ignore_ascii_case("Diagnostics") {
                saw_diagnostics = true;
            }
            current = Some(DoctorSection {
                title,
                severity,
                entries: Vec::new(),
            });
            continue;
        }

        // Tree-child row.
        if let Some(stripped) = trimmed.strip_prefix(TREE_BRANCH) {
            push_entry(&mut current, TREE_BRANCH, stripped);
            continue;
        }
        if let Some(stripped) = trimmed.strip_prefix(TREE_LAST) {
            push_entry(&mut current, TREE_LAST, stripped);
            continue;
        }
        // Continuation line (wrapped child) — append to the last
        // entry's text rather than dropping it. CC wraps long fix
        // strings to a second visible row indented past the tree
        // column; we re-flow with a single space.
        if let Some(sec) = current.as_mut() {
            if let Some(last) = sec.entries.last_mut() {
                last.text.push(' ');
                last.text.push_str(trimmed);
            }
        }
    }

    if let Some(s) = current.take() {
        sections.push(s);
    }

    let status = if sections.is_empty() {
        ParseStatus::Failed {
            reason: "no sections parsed".into(),
        }
    } else if !saw_diagnostics {
        ParseStatus::Degraded {
            reason: "Diagnostics header missing".into(),
        }
    } else {
        ParseStatus::Ok
    };

    (sections, status)
}

fn push_entry(current: &mut Option<DoctorSection>, prefix: char, rest: &str) {
    let text = rest.trim_start().to_string();
    if let Some(sec) = current.as_mut() {
        sec.entries.push(SectionEntry {
            text,
            tree_prefix: prefix.to_string(),
        });
    }
}

fn classify_header(s: &str) -> (String, DoctorSeverity) {
    let mut chars = s.chars();
    let first = chars.next();
    match first {
        Some(c) if c == CROSS_FIGURE => (
            chars.as_str().trim_start().to_string(),
            DoctorSeverity::Error,
        ),
        Some(c) if c == WARNING_FIGURE => (
            chars.as_str().trim_start().to_string(),
            DoctorSeverity::Warning,
        ),
        _ => (s.to_string(), DoctorSeverity::Healthy),
    }
}

// ─── Field extraction + severity aggregation ──────────────────────

fn extract_install_info(
    sections: &[DoctorSection],
) -> (Option<String>, Option<String>, Option<String>) {
    let diag = sections
        .iter()
        .find(|s| s.title.eq_ignore_ascii_case("Diagnostics"));
    let Some(diag) = diag else {
        return (None, None, None);
    };

    let mut version: Option<String> = None;
    let mut install: Option<String> = None;
    let mut path: Option<String> = None;

    for e in &diag.entries {
        let t = e.text.trim();
        if let Some(rest) = t.strip_prefix("Currently running:") {
            let rest = rest.trim();
            // Format: "<install_type> (<version>)" — version may be
            // missing if CC ships a future layout change.
            if let Some(open) = rest.find('(') {
                install = Some(rest[..open].trim().to_string());
                if let Some(close) = rest.find(')') {
                    if close > open {
                        version = Some(rest[open + 1..close].to_string());
                    }
                }
            } else {
                install = Some(rest.to_string());
            }
        } else if let Some(rest) = t.strip_prefix("Path:") {
            path = Some(rest.trim().to_string());
        } else if let Some(rest) = t.strip_prefix("Config install method:") {
            // If "Currently running" didn't carry an install string,
            // fall back to the config field.
            if install.is_none() {
                install = Some(rest.trim().to_string());
            }
        }
    }

    (version, install, path)
}

fn aggregate_severity(sections: &[DoctorSection], status: &ParseStatus) -> DoctorSeverity {
    match status {
        ParseStatus::Failed { .. } => return DoctorSeverity::Warning,
        ParseStatus::Degraded { .. } => {}
        ParseStatus::Ok => {}
    }
    let mut worst = DoctorSeverity::Healthy;
    for s in sections {
        worst = max_severity(worst, s.severity);
    }
    if matches!(status, ParseStatus::Degraded { .. }) {
        worst = max_severity(worst, DoctorSeverity::Warning);
    }
    worst
}

fn max_severity(a: DoctorSeverity, b: DoctorSeverity) -> DoctorSeverity {
    use DoctorSeverity::*;
    match (a, b) {
        (Error, _) | (_, Error) => Error,
        (Warning, _) | (_, Warning) => Warning,
        _ => Healthy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_FIXTURE: &[u8] = include_bytes!("test_fixtures/sample-2-1-138.bin");

    #[test]
    fn render_translates_cursor_forward_to_spaces() {
        // `\x1B[3C` should advance the cursor 3 columns; subsequent
        // text writes 3 cells over from where it would've otherwise.
        let bytes = b"X\x1b[3CY";
        let g = render(bytes);
        let row: String = g.rows[0].iter().map(|c| c.ch).collect();
        assert_eq!(row.chars().nth(0), Some('X'));
        assert_eq!(row.chars().nth(4), Some('Y'));
    }

    #[test]
    fn render_handles_cursor_up_and_overwrite() {
        // First line writes "old", then we go up 1 and clear-to-eol
        // and write "new". After replay row 0 should read "new".
        let bytes = b"old\r\n\x1b[1A\x1b[Knew";
        let g = render(bytes);
        let row0: String = g.rows[0].iter().map(|c| c.ch).collect();
        assert_eq!(row0.trim_end(), "new");
    }

    #[test]
    fn render_tracks_bold_per_cell() {
        let bytes = b"\x1b[1mBOLD\x1b[22m plain";
        let g = render(bytes);
        // First 4 cells should be bold, then the rest not.
        for i in 0..4 {
            assert!(g.rows[0][i].bold, "cell {i} should be bold");
        }
        // Skip the space at index 4; index 5 = 'p'.
        assert!(!g.rows[0][5].bold);
    }

    #[test]
    fn parse_real_fixture_finds_diagnostics() {
        let g = render(REAL_FIXTURE);
        let lines = grid_to_lines(&g);
        let (sections, status) = parse_lines(&lines);
        assert!(matches!(
            status,
            ParseStatus::Ok | ParseStatus::Degraded { .. }
        ));
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert!(
            titles.iter().any(|t| t.eq_ignore_ascii_case("Diagnostics")),
            "expected a Diagnostics section, got titles: {titles:?}"
        );
    }

    #[test]
    fn real_fixture_extracts_version_and_install_type() {
        let g = render(REAL_FIXTURE);
        let lines = grid_to_lines(&g);
        let (sections, _status) = parse_lines(&lines);
        let (version, install, path) = extract_install_info(&sections);
        assert_eq!(version.as_deref(), Some("2.1.138"));
        assert_eq!(install.as_deref(), Some("native"));
        // Path can be installation-specific; only assert it parsed.
        assert!(
            path.is_some(),
            "expected install path to parse out of Diagnostics"
        );
    }

    #[test]
    fn cross_figure_classifies_as_error() {
        let bytes = "\x1b[1m\u{2718} Plugin errors\x1b[22m".as_bytes();
        let g = render(bytes);
        let lines = grid_to_lines(&g);
        let (sections, _) = parse_lines(&lines);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Plugin errors");
        assert_eq!(sections[0].severity, DoctorSeverity::Error);
    }

    #[test]
    fn empty_input_fails_parse_status() {
        let g = render(b"");
        let lines = grid_to_lines(&g);
        let (sections, status) = parse_lines(&lines);
        assert!(sections.is_empty());
        assert!(matches!(status, ParseStatus::Failed { .. }));
    }

    #[test]
    fn aggregate_severity_picks_max() {
        let sections = vec![
            DoctorSection {
                title: "Diagnostics".into(),
                severity: DoctorSeverity::Healthy,
                entries: vec![],
            },
            DoctorSection {
                title: "Plugin errors".into(),
                severity: DoctorSeverity::Error,
                entries: vec![],
            },
        ];
        let sev = aggregate_severity(&sections, &ParseStatus::Ok);
        assert_eq!(sev, DoctorSeverity::Error);
    }

    #[test]
    #[ignore = "live: spawns real `claude doctor`, requires CC installed"]
    fn live_scrape_against_real_claude() {
        let snap = scrape_doctor();
        eprintln!(
            "live scrape: severity={:?}, cc_version={:?}, install_type={:?}, sections={}, status={:?}",
            snap.severity,
            snap.cc_version,
            snap.install_type,
            snap.sections.len(),
            snap.parse_status,
        );
        assert!(snap.raw_bytes > 0, "live scrape captured zero bytes");
        // Don't assert parse_status — a clean parse is the happy
        // path but the goal of this test is "did pty + capture +
        // replay at least produce non-empty output".
    }

    #[test]
    fn real_fixture_picks_up_plugin_errors_section() {
        // The captured fixture has 4 plugin cache errors; the
        // parser must classify the section as Error so the pill
        // would light red on this machine.
        let g = render(REAL_FIXTURE);
        let lines = grid_to_lines(&g);
        let (sections, _) = parse_lines(&lines);
        let plug = sections
            .iter()
            .find(|s| s.title.to_lowercase().contains("plugin"));
        assert!(plug.is_some(), "expected a Plugin errors section");
        assert_eq!(plug.unwrap().severity, DoctorSeverity::Error);
    }
}

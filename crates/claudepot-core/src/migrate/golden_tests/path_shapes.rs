//! §11.1 path-shape goldens.
//!
//! Locks down cross-OS path rewrite behavior for the canonical four
//! shapes (Unix absolute, Windows drive-letter, UNC, verbatim) plus
//! the per-row scenarios in spec §11.1. Each test is a named
//! standalone test mirroring the `test_sanitize_cc_parity_*`
//! discipline.
//!
//! Runs on every host OS so a Windows regression on macOS CI fails
//! immediately — pure string ops, no FS access except where noted.

use crate::migrate::nfc::nfc;
use crate::migrate::plan::{target_slug, RuleOrigin, SubstitutionTable};
use crate::migrate::rewrite::rewrite_jsonl_line_multi;

// Row 1 — `/Users/joker/x` (mac) → `/home/alice/x` (linux):
// slug `-home-alice-x`; JSONL `cwd` rewritten.
#[test]
fn row1_macos_to_linux_unix_to_unix() {
    let mut t = SubstitutionTable::new();
    t.push("/Users/joker/x", "/home/alice/x", RuleOrigin::ProjectCwd);
    t.finalize();
    assert_eq!(target_slug("/home/alice/x"), "-home-alice-x");
    let line = r#"{"cwd":"/Users/joker/x","slug":"-Users-joker-x"}"#;
    let (out, n) = rewrite_jsonl_line_multi(line, &t);
    assert!(n >= 1);
    assert!(out.contains(r#""cwd":"/home/alice/x""#));
    // slug must recompute to target shape.
    assert!(out.contains(r#""slug":"-home-alice-x""#));
}

// Row 2 — `/Users/joker/x` (mac) → `C:\Users\alice\x` (win):
// slug `C--Users-alice-x`; backslashes; no `\\?\`.
#[test]
fn row2_macos_to_windows_unix_to_drive_letter() {
    let mut t = SubstitutionTable::new();
    t.push(
        "/Users/joker/x",
        r"C:\Users\alice\x",
        RuleOrigin::ProjectCwd,
    );
    t.finalize();
    assert_eq!(target_slug(r"C:\Users\alice\x"), "C--Users-alice-x");

    let line = r#"{"cwd":"/Users/joker/x","slug":"-Users-joker-x"}"#;
    let (out, _n) = rewrite_jsonl_line_multi(line, &t);
    // The cwd field in the JSON wire form: backslashes escaped as `\\`.
    assert!(
        out.contains(r#""cwd":"C:\\Users\\alice\\x""#),
        "expected windows-shape cwd in output; got: {out}"
    );
    // No verbatim prefix anywhere.
    assert!(!out.contains(r"\\?\"));
    assert!(!out.contains(r"\\\\?\\"));
    // Slug recomputes to drive-letter form.
    assert!(out.contains(r#""slug":"C--Users-alice-x""#));
}

// Row 3 — `\\nas\share\proj` (win UNC) → `/Volumes/proj` (mac):
// slug from UNC double-hyphen; cwd flips to forward slashes.
#[test]
fn row3_windows_unc_to_macos_volume() {
    let mut t = SubstitutionTable::new();
    t.push(r"\\nas\share\proj", "/Volumes/proj", RuleOrigin::ProjectCwd);
    t.finalize();
    assert_eq!(target_slug("/Volumes/proj"), "-Volumes-proj");
    // Source UNC slug shape is what the SOURCE machine wrote into
    // its session JSONL.
    let line = r#"{"cwd":"\\\\nas\\share\\proj","slug":"--nas-share-proj"}"#;
    let (out, n) = rewrite_jsonl_line_multi(line, &t);
    assert!(n >= 1, "expected rewrite; got: {out}");
    assert!(out.contains(r#""cwd":"/Volumes/proj""#));
    assert!(out.contains(r#""slug":"-Volumes-proj""#));
}

// Row 4 — same-OS migrate; tool-result `Edit` paths inside
// `/Users/joker/x/foo.rs` rewritten via the HOME rule.
#[test]
fn row4_home_rule_rewrites_embedded_tool_paths() {
    let mut t = SubstitutionTable::new();
    t.push("/Users/joker", "/Users/alice", RuleOrigin::Home);
    t.finalize();
    // Tool-result lines carry path strings inside `tool_use.input`
    // and `tool_result.content`. Multi-rule rewriter walks all string
    // values, not just `cwd`.
    let line =
        r#"{"cwd":"/Users/joker/x","tool_use":{"input":{"file_path":"/Users/joker/x/foo.rs"}}}"#;
    let (out, _n) = rewrite_jsonl_line_multi(line, &t);
    assert!(out.contains("/Users/alice/x"));
    assert!(out.contains("/Users/alice/x/foo.rs"));
}

// Row 5 — path > 200 chars; new slug recomputed; long-path-dual-hash
// row asserts both `findProjectDir` prefix-scan and exact match work.
//
// v0 still emits djb2 (matching claudepot's current `sanitize_path`),
// not WyHash. CC's `findProjectDir` prefix-scan handles it (spec §5.3
// known parity gap). This test pins the prefix-scan compatibility
// shape (200-char prefix + `-` + suffix).
#[test]
fn row5_long_path_emits_djb2_with_prefix_scan_shape() {
    let long = "/Users/joker/".to_string() + &"a".repeat(250);
    let slug = target_slug(&long);
    assert!(slug.len() > 200);
    let prefix = &slug[..200];
    let after_prefix = &slug[200..];
    // Shape: 200-char prefix, then `-`, then a base36 djb2 suffix.
    assert!(after_prefix.starts_with('-'));
    let suffix = &after_prefix[1..];
    assert!(suffix.chars().all(|c| c.is_ascii_alphanumeric()));
    // Prefix is the uniformly-sanitized first 200 chars.
    assert!(prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-'));
}

// Row 6 — path with NFD `é` → NFC target: slug equality post-normalize.
#[test]
fn row6_nfd_and_nfc_produce_equal_slugs() {
    let nfd = "/tmp/caf\u{0065}\u{0301}";
    let nfc_form = "/tmp/caf\u{00E9}";
    assert_eq!(target_slug(nfd), target_slug(nfc_form));
    // And the NFC helper is idempotent.
    assert_eq!(nfc(nfc_form), nfc_form);
    assert_eq!(nfc(nfd), nfc_form);
}

// Row 7 — path with emoji `🎉` (UTF-16 surrogate pair):
// slug parity with CC's `.charCodeAt()` iteration.
#[test]
fn row7_emoji_consumes_two_hyphens_per_surrogate_pair() {
    // 🎉 = U+1F389, surrogate pair in UTF-16.
    // CC's `.replace(/[^a-zA-Z0-9]/g, '-')` iterates UTF-16 code
    // units; each surrogate becomes its own hyphen.
    assert_eq!(target_slug("/tmp/\u{1F389}emoji"), "-tmp---emoji");
}

// Row 8 — path containing literal `sk-ant-…`: rewrite preserves the
// substring; redaction NOT invoked. The migrator MUST NOT mutate
// non-path strings.
#[test]
fn row8_secret_lookalike_substring_preserved() {
    // A path that happens to look like a token (won't ever exist in
    // practice but a regression here would silently mangle paths).
    let mut t = SubstitutionTable::new();
    t.push("/tmp/x", "/tmp/y", RuleOrigin::ProjectCwd);
    t.finalize();
    // Inject a token-shape literal as an unrelated string field. The
    // migrator must leave it alone.
    let line = r#"{"cwd":"/tmp/x","note":"sk-ant-oat01-not-a-real-token-xyz"}"#;
    let (out, _n) = rewrite_jsonl_line_multi(line, &t);
    assert!(out.contains("sk-ant-oat01-not-a-real-token-xyz"));
}

// Row 9 — mid-session `cd` (cwd-per-line variance): only matching
// lines rewritten — same as rename §4.2 P6.
#[test]
fn row9_mid_session_cd_lines_rewrite_independently() {
    let mut t = SubstitutionTable::new();
    t.push("/Users/joker/x", "/home/alice/x", RuleOrigin::ProjectCwd);
    t.finalize();
    let l1 = r#"{"cwd":"/Users/joker/x"}"#;
    let l2 = r#"{"cwd":"/Users/joker/y"}"#; // cd'd elsewhere mid-session
    let (o1, n1) = rewrite_jsonl_line_multi(l1, &t);
    let (o2, n2) = rewrite_jsonl_line_multi(l2, &t);
    assert!(n1 >= 1);
    assert_eq!(n2, 0);
    assert!(o1.contains("/home/alice/x"));
    assert_eq!(o2, l2);
}

// Row 10 — symlinked source cwd: bundle records canonical via
// `resolve_path`; slug computed against canonical.
//
// Pure-string check here — the canonicalize step is exercised in
// `project_helpers::resolve_path`'s own tests. We pin the contract
// at the migrator layer: feeding the canonical form to `target_slug`
// matches feeding the symlinked form once both have been
// `simplify_windows_path`'d and NFC-normalized.
#[test]
fn row10_canonical_cwd_drives_target_slug() {
    let canonical = "/Volumes/repo/x";
    let alias = "/Users/joker/x"; // a hypothetical symlink alias
                                  // Different inputs produce different slugs (no auto-canonicalize
                                  // at sanitize layer); migrator must canonicalize upstream.
    assert_ne!(target_slug(canonical), target_slug(alias));
    // Once canonicalized, the slug is deterministic.
    assert_eq!(target_slug(canonical), "-Volumes-repo-x");
}

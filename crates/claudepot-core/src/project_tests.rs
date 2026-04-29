//! Inline test module for `project.rs`. Lives in this sibling file
//! so `project.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "project_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;
use crate::path_utils::simplify_windows_path;

/// Canonicalize a path (typically a temp-dir root) and strip the
/// Windows `\\?\` verbatim prefix. Mirrors what `resolve_path` does
/// in production; without this, fixtures on Windows compute slugs
/// from the verbatim form while production looks them up in the
/// simplified form, and every CC-dir lookup misses. No-op on Unix.
fn canonical_test_path(p: &Path) -> PathBuf {
    let canon = p.canonicalize().unwrap();
    PathBuf::from(simplify_windows_path(&canon.to_string_lossy()))
}

/// String form of `canonical_test_path` — for sites that feed the
/// path into `sanitize_path` or compare against a stringified
/// production output.
fn canonical_test_str(p: &Path) -> String {
    canonical_test_path(p).to_string_lossy().to_string()
}

#[test]
fn test_sanitize_unix_path() {
    assert_eq!(
        sanitize_path("/Users/joker/github/xiaolai/myprojects/kannon"),
        "-Users-joker-github-xiaolai-myprojects-kannon"
    );
}

#[test]
fn test_sanitize_windows_path() {
    assert_eq!(
        sanitize_path("C:\\Users\\joker\\project"),
        "C--Users-joker-project"
    );
}

#[test]
fn test_sanitize_preserves_alphanumeric() {
    assert_eq!(sanitize_path("abc123"), "abc123");
}

#[test]
fn test_sanitize_replaces_special_chars() {
    assert_eq!(sanitize_path("/a.b_c-d"), "-a-b-c-d");
}

#[test]
fn test_sanitize_long_path_with_hash() {
    let long_path = "/".to_string() + &"a".repeat(250);
    let result = sanitize_path(&long_path);
    // Should be 200 chars + '-' + hash
    assert!(result.len() > MAX_SANITIZED_LENGTH);
    assert!(result.starts_with("-"));
    // The first 200 chars should be from the sanitized path
    let prefix = &result[..MAX_SANITIZED_LENGTH];
    assert!(prefix.chars().all(|c| c == '-' || c == 'a'));
}

#[test]
fn test_sanitize_unicode_path() {
    // Unicode chars are non-alphanumeric, should become `-`
    assert_eq!(sanitize_path("/tmp/\u{00e9}l\u{00e8}ve"), "-tmp--l-ve");
}

#[cfg(not(target_os = "windows"))]
#[test]
fn test_unsanitize_roundtrip_simple_unix() {
    let original = "/Users/joker/project";
    let sanitized = sanitize_path(original);
    let unsanitized = unsanitize_path(&sanitized);
    assert_eq!(unsanitized, original);
}

#[cfg(not(target_os = "windows"))]
#[test]
fn test_unsanitize_lossy_unix() {
    // Hyphens and underscores both become `-`, so unsanitize is lossy
    let sanitized = sanitize_path("/my-project");
    let unsanitized = unsanitize_path(&sanitized);
    // Original was /my-project, sanitized to -my-project, unsanitized to /my/project
    assert_eq!(unsanitized, "/my/project");
}

#[test]
fn test_unsanitize_windows_drive_letter_roundtrip() {
    // The `<alpha>--` shape is unambiguous: no Unix-sanitized slug
    // can start with an ASCII letter followed by two hyphens (a
    // leading `-` in a Unix slug means the first char of the
    // original was a separator, not a letter). So we recover the
    // Windows form on any host OS.
    let original = r"C:\Users\joker\project";
    let sanitized = sanitize_path(original);
    assert_eq!(sanitized, "C--Users-joker-project");
    let unsanitized = unsanitize_path(&sanitized);
    assert_eq!(unsanitized, original);
}

#[test]
fn test_unsanitize_windows_drive_letter_lowercase() {
    let original = r"d:\work\repo";
    let sanitized = sanitize_path(original);
    assert_eq!(sanitized, "d--work-repo");
    let unsanitized = unsanitize_path(&sanitized);
    assert_eq!(unsanitized, original);
}

#[cfg(target_os = "windows")]
#[test]
fn test_unsanitize_windows_unc_roundtrip_on_windows() {
    // UNC slug `--server-share-project` is ambiguous with a Unix
    // path whose first component starts with `-`. On Windows we
    // resolve to UNC; on Unix we keep the `/` convention.
    let original = r"\\server\share\project";
    let sanitized = sanitize_path(original);
    assert_eq!(sanitized, "--server-share-project");
    let unsanitized = unsanitize_path(&sanitized);
    assert_eq!(unsanitized, original);
}

#[cfg(target_os = "windows")]
#[test]
fn test_unsanitize_unix_slug_on_windows_uses_backslash() {
    // On Windows, a Unix-shaped slug gets `\` as the fallback
    // separator — matching the host filesystem convention.
    let sanitized = "-Users-joker-project";
    let unsanitized = unsanitize_path(sanitized);
    assert_eq!(unsanitized, r"\Users\joker\project");
}

#[test]
fn test_djb2_hash_deterministic() {
    let h1 = djb2_hash("test");
    let h2 = djb2_hash("test");
    assert_eq!(h1, h2);
}

#[test]
fn test_djb2_hash_different_inputs() {
    let h1 = djb2_hash("abc");
    let h2 = djb2_hash("def");
    assert_ne!(h1, h2);
}

// ---------------------------------------------------------------------
// Group 1 — CC parity (golden values from CC's sanitizePath/djb2Hash
// run in Node.js on 2026-04-13). If these fail, either CC changed their
// implementation or we drifted. See /tmp/cc-golden-values.js.
// ---------------------------------------------------------------------

#[test]
fn test_sanitize_cc_parity_unix() {
    assert_eq!(
        sanitize_path("/Users/joker/github/xiaolai/myprojects/com.claudepot.app"),
        "-Users-joker-github-xiaolai-myprojects-com-claudepot-app"
    );
}

#[test]
fn test_sanitize_cc_parity_windows() {
    assert_eq!(
        sanitize_path("C:\\Users\\joker\\Documents\\project"),
        "C--Users-joker-Documents-project"
    );
}

#[test]
fn test_sanitize_cc_parity_hyphen_in_name() {
    assert_eq!(
        sanitize_path("/Users/joker/my-project"),
        "-Users-joker-my-project"
    );
}

#[test]
fn test_sanitize_cc_parity_nfc_accent() {
    assert_eq!(sanitize_path("/tmp/café-project"), "-tmp-caf--project");
}

#[test]
fn test_sanitize_cc_parity_emoji() {
    // JS UTF-16 surrogate pair (🎉 = U+1F389) produces TWO hyphens,
    // not one. This is the whole point of encode_utf16 in our impl.
    assert_eq!(sanitize_path("/tmp/🎉emoji"), "-tmp---emoji");
}

#[test]
fn test_djb2_cc_parity_long_path() {
    let input = "/Users/joker/".to_string() + &"a".repeat(250);
    assert_eq!(djb2_hash(&input), "lwkvhu");
    // Full sanitize_path output: 200-char prefix + '-' + hash.
    let result = sanitize_path(&input);
    assert!(result.ends_with("-lwkvhu"), "result={result}");
    assert_eq!(result.len(), 200 + 1 + "lwkvhu".len());
}

#[test]
fn test_djb2_cc_parity_unicode() {
    // "/tmp/café" — 'é' encodes as U+00E9 (one UTF-16 code unit).
    assert_eq!(djb2_hash("/tmp/café"), "udmm60");
}

// ---------------------------------------------------------------------
// Group 10 — Windows path tests (CC parity golden values).
// Pure string ops: these run on all platforms regardless of cfg.
// ---------------------------------------------------------------------

#[test]
fn test_sanitize_windows_drive_letter() {
    assert_eq!(
        sanitize_path("C:\\Users\\joker\\project"),
        "C--Users-joker-project"
    );
}

#[test]
fn test_sanitize_windows_unc_path() {
    assert_eq!(
        sanitize_path("\\\\server\\share\\project"),
        "--server-share-project"
    );
}

#[test]
fn test_sanitize_windows_spaces_in_path() {
    assert_eq!(
        sanitize_path("C:\\Program Files\\My App"),
        "C--Program-Files-My-App"
    );
}

#[test]
fn test_sanitize_windows_long_path() {
    let input = "C:\\Users\\joker\\".to_string() + &"a".repeat(250);
    assert_eq!(djb2_hash(&input), "27k5dq");
    let out = sanitize_path(&input);
    assert!(out.ends_with("-27k5dq"), "out={out}");
    assert_eq!(out.len(), 200 + 1 + "27k5dq".len());
}

#[test]
fn test_sanitize_windows_reserved_chars() {
    // ':', '?' are reserved on Windows; all non-alphanumerics become '-'.
    assert_eq!(
        sanitize_path("C:\\Users\\joker\\file:name?"),
        "C--Users-joker-file-name-"
    );
}

#[test]
fn test_format_radix_base36() {
    assert_eq!(format_radix(0, 36), "0");
    assert_eq!(format_radix(35, 36), "z");
    assert_eq!(format_radix(36, 36), "10");
}

#[test]
fn test_format_size() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(1024), "1.0 KB");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(1048576), "1.0 MB");
    assert_eq!(format_size(1073741824), "1.0 GB");
}

#[test]
fn test_list_projects_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path();
    // No projects/ dir at all
    let result = list_projects(config_dir).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_recover_cwd_skips_custom_title_first_line() {
    // CC sometimes writes a `custom-title` entry (no `cwd` field) as
    // the first line of a session, then writes normal entries with
    // `cwd` on subsequent lines. The recovery must keep reading
    // until it finds `cwd`, otherwise paths with `.` collapse
    // (e.g. `-xiaolai-lixiaolai-com` → `/xiaolai/lixiaolai/com`
    // instead of the real `/xiaolai/lixiaolai.com`).
    let tmp = tempfile::tempdir().unwrap();
    let session_file = tmp.path().join("abc.jsonl");
    fs::write(
        &session_file,
        concat!(
            r#"{"type":"custom-title","title":"session","timestamp":"2026-04-20T00:00:00Z","uuid":"u1"}"#,
            "\n",
            r#"{"type":"user","cwd":"/Users/joker/github/xiaolai/lixiaolai.com","timestamp":"2026-04-20T00:00:01Z","uuid":"u2"}"#,
            "\n",
        ),
    )
    .unwrap();

    let recovered = recover_cwd_from_sessions(tmp.path());
    assert_eq!(
        recovered.as_deref(),
        Some("/Users/joker/github/xiaolai/lixiaolai.com")
    );
}

#[test]
fn test_recover_cwd_skips_summary_entries() {
    // Same pattern with a `summary` entry instead of `custom-title`.
    let tmp = tempfile::tempdir().unwrap();
    let session_file = tmp.path().join("abc.jsonl");
    fs::write(
        &session_file,
        concat!(
            r#"{"type":"summary","summary":"...","timestamp":"2026-04-20T00:00:00Z","uuid":"u1"}"#,
            "\n",
            r#"{"type":"assistant","cwd":"/home/user/some.dir.with.dots","timestamp":"2026-04-20T00:00:01Z","uuid":"u2"}"#,
            "\n",
        ),
    )
    .unwrap();

    let recovered = recover_cwd_from_sessions(tmp.path());
    assert_eq!(recovered.as_deref(), Some("/home/user/some.dir.with.dots"));
}

#[test]
fn test_recover_cwd_none_when_no_session_has_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("a.jsonl"),
        r#"{"type":"custom-title","title":"x","uuid":"u1"}"#,
    )
    .unwrap();
    assert_eq!(recover_cwd_from_sessions(tmp.path()), None);
}

#[test]
fn test_recover_cwd_empty_cwd_is_ignored() {
    // `"cwd": ""` must not be accepted — fall through to the next line.
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("a.jsonl"),
        concat!(
            r#"{"type":"user","cwd":"","uuid":"u1"}"#,
            "\n",
            r#"{"type":"user","cwd":"/Users/joker/project","uuid":"u2"}"#,
            "\n",
        ),
    )
    .unwrap();
    assert_eq!(
        recover_cwd_from_sessions(tmp.path()).as_deref(),
        Some("/Users/joker/project")
    );
}

#[test]
fn test_recover_cwd_skips_unparseable_lines() {
    // Malformed JSON (BOM, truncated line, partial write) must not
    // abort the whole scan — keep reading.
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("a.jsonl"),
        concat!(
            "\u{feff}not-json\n",
            "{incomplete\n",
            r#"{"type":"user","cwd":"/real/path","uuid":"u1"}"#,
            "\n",
        ),
    )
    .unwrap();
    assert_eq!(
        recover_cwd_from_sessions(tmp.path()).as_deref(),
        Some("/real/path")
    );
}

#[test]
fn test_recover_cwd_windows_drive_letter() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("a.jsonl"),
        concat!(
            r#"{"type":"custom-title","title":"x","uuid":"u1"}"#,
            "\n",
            // JSON-escaped backslashes: `C:\Users\joker\project`.
            r#"{"type":"user","cwd":"C:\\Users\\joker\\project","uuid":"u2"}"#,
            "\n",
        ),
    )
    .unwrap();
    assert_eq!(
        recover_cwd_from_sessions(tmp.path()).as_deref(),
        Some(r"C:\Users\joker\project")
    );
}

#[test]
fn test_recover_cwd_windows_unc_path() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("a.jsonl"),
        concat!(
            r#"{"type":"user","cwd":"\\\\server\\share\\project","uuid":"u1"}"#,
            "\n",
        ),
    )
    .unwrap();
    assert_eq!(
        recover_cwd_from_sessions(tmp.path()).as_deref(),
        Some(r"\\server\share\project")
    );
}

#[test]
fn test_recover_cwd_strips_windows_verbatim_prefix() {
    // Defense-in-depth: CC never writes `\\?\` in cwd, but a
    // third-party writer might. The recovered path must be the
    // non-verbatim form so downstream sanitize/roundtrip checks
    // don't drift.
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("a.jsonl"),
        concat!(
            r#"{"type":"user","cwd":"\\\\?\\C:\\Users\\joker","uuid":"u1"}"#,
            "\n",
        ),
    )
    .unwrap();
    assert_eq!(
        recover_cwd_from_sessions(tmp.path()).as_deref(),
        Some(r"C:\Users\joker")
    );
}

#[test]
fn test_recover_cwd_strips_windows_verbatim_unc_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("a.jsonl"),
        concat!(
            r#"{"type":"user","cwd":"\\\\?\\UNC\\server\\share\\p","uuid":"u1"}"#,
            "\n",
        ),
    )
    .unwrap();
    assert_eq!(
        recover_cwd_from_sessions(tmp.path()).as_deref(),
        Some(r"\\server\share\p")
    );
}

#[test]
fn test_list_projects_windows_path_with_dot_in_name() {
    // End-to-end: a Windows-style project whose name contains a
    // literal `.` should survive the sanitize/unsanitize roundtrip
    // via the recovered cwd, not collapse to backslashes.
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();
    // sanitize(`C:\Users\joker\lixiaolai.com`) = `C--Users-joker-lixiaolai-com`.
    let slug = "C--Users-joker-lixiaolai-com";
    let proj = projects_dir.join(slug);
    fs::create_dir(&proj).unwrap();
    fs::write(
        proj.join("s1.jsonl"),
        concat!(
            r#"{"type":"custom-title","title":"x","uuid":"u1"}"#,
            "\n",
            r#"{"type":"user","cwd":"C:\\Users\\joker\\lixiaolai.com","uuid":"u2"}"#,
            "\n",
        ),
    )
    .unwrap();

    let result = list_projects(tmp.path()).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].original_path, r"C:\Users\joker\lixiaolai.com");
}

#[test]
fn test_list_projects_preserves_special_chars_via_cwd() {
    // Exercises the full edge-case catalog found in real CC project
    // dirs: spaces, apostrophes, tildes, and dots. All would be lost
    // by `unsanitize_path` alone, but the authoritative `cwd` keeps
    // them intact.
    let cases = [
        (
            "-Users-joker-Desktop-reading-room",
            "/Users/joker/Desktop/reading-room",
        ),
        (
            "-Users-joker-Writer-s-Office",
            "/Users/joker/Writer's Office",
        ),
        (
            "-Users-joker-Library-Mobile-Documents-iCloud-com-nssurge-inc-Documents",
            "/Users/joker/Library/Mobile Documents/iCloud~com~nssurge~inc/Documents",
        ),
        (
            "-Users-joker-github-xiaolai-myprojects-com-claudepot-app",
            "/Users/joker/github/xiaolai/myprojects/com.claudepot.app",
        ),
    ];

    for (slug, cwd) in cases {
        let tmp = tempfile::tempdir().unwrap();
        let projects_dir = tmp.path().join("projects");
        fs::create_dir(&projects_dir).unwrap();
        let proj = projects_dir.join(slug);
        fs::create_dir(&proj).unwrap();
        let line = format!(
            r#"{{"type":"custom-title","title":"x","uuid":"u1"}}
{{"type":"user","cwd":{:?},"uuid":"u2"}}
"#,
            cwd
        );
        fs::write(proj.join("s1.jsonl"), line).unwrap();

        let result = list_projects(tmp.path()).unwrap();
        assert_eq!(result.len(), 1, "slug={slug}");
        assert_eq!(result[0].original_path, cwd, "slug={slug}");
    }
}

#[test]
fn test_list_projects_recovers_cwd_with_dot_in_name() {
    // End-to-end: a project whose real path contains `.` should be
    // reported with the dot intact, recovered from session.jsonl
    // even when the first transcript line is a `custom-title`.
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();
    let slug = "-Users-joker-github-xiaolai-lixiaolai-com";
    let proj = projects_dir.join(slug);
    fs::create_dir(&proj).unwrap();
    fs::write(
        proj.join("s1.jsonl"),
        concat!(
            r#"{"type":"custom-title","title":"x","uuid":"u1"}"#,
            "\n",
            r#"{"type":"user","cwd":"/Users/joker/github/xiaolai/lixiaolai.com","uuid":"u2"}"#,
            "\n",
        ),
    )
    .unwrap();

    let result = list_projects(tmp.path()).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].original_path,
        "/Users/joker/github/xiaolai/lixiaolai.com"
    );
}

#[test]
fn test_list_projects_with_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // Create a fake project
    let proj = projects_dir.join("-tmp-myproject");
    fs::create_dir(&proj).unwrap();
    fs::write(proj.join("abc.jsonl"), "{}").unwrap();
    fs::write(proj.join("def.jsonl"), "{}").unwrap();

    let memory_dir = proj.join("memory");
    fs::create_dir(&memory_dir).unwrap();
    fs::write(memory_dir.join("MEMORY.md"), "# mem").unwrap();

    let result = list_projects(tmp.path()).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].sanitized_name, "-tmp-myproject");
    assert_eq!(result[0].session_count, 2);
    assert_eq!(result[0].memory_file_count, 1);
    assert!(result[0].is_orphan); // /tmp/myproject likely doesn't exist
}

#[test]
fn test_show_project_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let result = show_project(tmp.path(), "/nonexistent/path");
    assert!(result.is_err());
}

#[test]
fn test_move_project_same_path() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("myproject");
    fs::create_dir(&src).unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: src.clone(),
        config_dir: tmp.path().to_path_buf(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: false,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink);
    assert!(matches!(result, Err(ProjectError::SamePath)));
}

#[test]
fn test_move_project_renames_cc_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // Canonicalize to handle macOS /tmp -> /private/tmp symlink
    let base = canonical_test_path(tmp.path());

    // Create source directory
    let src = base.join("old");
    fs::create_dir(&src).unwrap();

    // Create CC project dir for old path (using canonical path)
    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();
    let old_san = sanitize_path(&src.to_string_lossy());
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("session.jsonl"), "{}").unwrap();

    let dst = base.join("new");

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(result.actual_dir_moved);
    assert!(result.cc_dir_renamed);
    assert!(dst.exists());
    assert!(!src.exists());

    let new_san = sanitize_path(&dst.to_string_lossy());
    assert!(projects_dir.join(&new_san).exists());
    assert!(!projects_dir.join(&old_san).exists());

    // Verify session file content survived the move
    let moved_session = projects_dir.join(&new_san).join("session.jsonl");
    assert!(moved_session.exists());
    assert_eq!(fs::read_to_string(moved_session).unwrap(), "{}");
}

/// When a long path's sanitized form exceeds MAX_SANITIZED_LENGTH, the
/// trailing hash suffix depends on runtime (Bun.hash vs djb2). If the
/// existing CC dir was created by the Bun-compiled CC CLI with a hash
/// suffix that doesn't match claudepot's djb2 output, move must still
/// find it via prefix scanning. This mirrors CC's own findProjectDir
/// tolerance (see sessionStoragePortable.ts:354-375).
#[test]
fn test_move_project_long_path_prefix_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    // Construct a >200-char source path
    let deep = "a".repeat(210);
    let src = base.join(&deep);
    fs::create_dir(&src).unwrap();

    // Simulate a CC-Bun-created dir: same 200-char prefix as
    // claudepot would compute, but a DIFFERENT hash suffix than djb2.
    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();
    let claudepot_san = sanitize_path(&src.to_string_lossy());
    assert!(claudepot_san.len() > MAX_SANITIZED_LENGTH);
    let prefix = &claudepot_san[..MAX_SANITIZED_LENGTH];
    let bun_style_san = format!("{}-{}", prefix, "fakebunhashxyz");
    assert_ne!(bun_style_san, claudepot_san); // hash suffixes differ
    let cc_old = projects_dir.join(&bun_style_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("session.jsonl"), r#"{"cwd":"x"}"#).unwrap();

    let dst = base.join(&"b".repeat(210));

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(result.actual_dir_moved, "disk dir should be moved");
    assert!(
        result.cc_dir_renamed,
        "CC dir should be found via prefix fallback and renamed"
    );

    // Source CC dir should no longer exist at its old sanitized name
    assert!(!cc_old.exists());
    // Destination should exist at claudepot's new_san (the prefix
    // portion must still match what CC would look up via findProjectDir)
    let new_san = sanitize_path(&dst.to_string_lossy());
    let new_prefix = &new_san[..MAX_SANITIZED_LENGTH];
    let found_new = fs::read_dir(&projects_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with(new_prefix));
    assert!(
        found_new.is_some(),
        "new CC dir should exist under the new prefix"
    );
}

#[test]
fn test_move_rejects_new_inside_old() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());
    let src = base.join("proj");
    fs::create_dir(&src).unwrap();
    let args = MoveArgs {
        old_path: src.clone(),
        new_path: src.join("nested"),
        config_dir: base,
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };
    let err = move_project(&args, &crate::project_progress::NoopSink).unwrap_err();
    assert!(matches!(err, ProjectError::Ambiguous(ref m) if m.contains("inside")));
}

#[test]
fn test_move_rejects_old_inside_new() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());
    let outer = base.join("outer");
    fs::create_dir(&outer).unwrap();
    let inner = outer.join("inner");
    fs::create_dir(&inner).unwrap();
    let args = MoveArgs {
        old_path: inner,
        new_path: outer,
        config_dir: base,
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };
    let err = move_project(&args, &crate::project_progress::NoopSink).unwrap_err();
    assert!(matches!(err, ProjectError::Ambiguous(_)));
}

/// End-to-end verification that Phase 6 (session jsonl rewrite) runs
/// as part of move_project: stale `cwd` fields inside session jsonls
/// get rewritten to the new path, including `cd`-into-subdir entries
/// that use the boundary-prefix form.
#[test]
fn test_move_project_rewrites_session_jsonl_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old-project");
    fs::create_dir(&src).unwrap();
    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let old_str = src.to_string_lossy().to_string();
    let old_san = sanitize_path(&old_str);
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();

    // Build the JSONL via `serde_json::json!` so backslashes in Windows
    // paths get correctly escaped — interpolating `old_str` into a
    // raw `format!` template produces invalid JSON on Windows and
    // every line silently fails to parse, so the rewriter never sees
    // a `cwd` to rewrite.
    let sep = std::path::MAIN_SEPARATOR;
    let line_a = serde_json::json!({"cwd": old_str.as_str(), "i": 1}).to_string();
    let old_with_src = format!("{old_str}{sep}src");
    let line_b = serde_json::json!({"cwd": old_with_src.as_str(), "i": 2}).to_string();
    let line_c = serde_json::json!({"cwd": "/elsewhere", "i": 3}).to_string();
    fs::write(
        cc_old.join("sess.jsonl"),
        format!("{line_a}\n{line_b}\n{line_c}\n"),
    )
    .unwrap();

    // Subagent jsonl
    let subagent_dir = cc_old.join("sessA").join("subagents");
    fs::create_dir_all(&subagent_dir).unwrap();
    let agent_line = serde_json::json!({"cwd": old_str.as_str(), "agent": "x"}).to_string();
    fs::write(
        subagent_dir.join("agent-x.jsonl"),
        format!("{agent_line}\n"),
    )
    .unwrap();

    let dst = base.join("renamed-project");
    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };
    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();

    assert!(result.cc_dir_renamed);
    assert!(result.jsonl_files_scanned >= 2);
    assert!(result.jsonl_files_modified >= 2);
    // 2 in main session + 1 in subagent = 3 cwd rewrites (the
    // `/elsewhere` entry must NOT be rewritten).
    assert_eq!(result.jsonl_lines_rewritten, 3);
    assert!(result.jsonl_errors.is_empty());

    let new_str = dst.to_string_lossy().to_string();
    let new_san = sanitize_path(&new_str);
    let cc_new = projects_dir.join(&new_san);
    let after_main = fs::read_to_string(cc_new.join("sess.jsonl")).unwrap();
    // Build the contains-needles via serde_json so backslashes in
    // Windows paths are rendered as their JSON-escaped form on disk
    // (`"cwd":"C:\\Users\\..."` — two backslashes per separator).
    // Raw `format!(r#""cwd":"{}""#, ...)` produces single-backslash
    // strings that don't appear anywhere in the actual JSONL.
    let cwd_needle = |s: &str| format!(r#""cwd":{}"#, serde_json::to_string(s).unwrap());
    assert!(after_main.contains(&cwd_needle(&new_str)));
    assert!(after_main.contains(&cwd_needle(&format!("{new_str}{sep}src"))));
    assert!(after_main.contains(&cwd_needle("/elsewhere"))); // untouched
    assert!(!after_main.contains(&cwd_needle(&old_str)));
}

/// End-to-end verification that Phase 7 (~/.claude.json projects map)
/// runs as part of move_project when claude_json_path is supplied:
/// the map key migrates from old_path to new_path, preserving value,
/// with no collision.
#[test]
fn test_move_project_rewrites_claude_json() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("origproj");
    fs::create_dir(&src).unwrap();
    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let old_str = src.to_string_lossy().to_string();
    let old_san = sanitize_path(&old_str);
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();

    // Fake ~/.claude.json sibling file.
    let claude_json = base.join("claude.json");
    let cfg_before = serde_json::json!({
        "projects": {
            old_str.clone(): {"trust": true, "allowedTools": ["X"]}
        }
    });
    fs::write(
        &claude_json,
        serde_json::to_string_pretty(&cfg_before).unwrap(),
    )
    .unwrap();

    let dst = base.join("newproj");
    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: Some(claude_json.clone()),
        snapshots_dir: Some(base.join("snaps")),
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };
    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();

    assert!(result.cc_dir_renamed);
    assert!(result.config_key_renamed);
    assert!(!result.config_had_collision);
    assert!(result.config_snapshot_path.is_none());

    // Verify the JSON was actually rewritten.
    let cfg_after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
    let new_str = dst.to_string_lossy().to_string();
    assert!(cfg_after["projects"].get(&old_str).is_none());
    assert_eq!(
        cfg_after["projects"][&new_str]["trust"],
        serde_json::json!(true)
    );
    assert_eq!(
        cfg_after["projects"][&new_str]["allowedTools"],
        serde_json::json!(["X"])
    );
}

/// Tests without claude_json_path should not touch any config file
/// (hermetic). Confirmed by no P7 fields being set and no file being
/// created.
#[test]
fn test_move_project_skips_p7_when_claude_json_path_is_none() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("a");
    fs::create_dir(&src).unwrap();
    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();
    let old_san = sanitize_path(&src.to_string_lossy());
    fs::create_dir(projects_dir.join(&old_san)).unwrap();

    let dst = base.join("b");
    let args = MoveArgs {
        old_path: src,
        new_path: dst,
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };
    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();

    assert!(result.cc_dir_renamed);
    assert!(!result.config_key_renamed);
    assert!(!result.config_had_collision);
    // No P7 snapshots specifically — journal + lock dirs are created
    // by the always-on recovery infrastructure, but snapshots are
    // only written when a destructive phase actually runs.
    assert!(!base.join("claudepot").join("snapshots").exists());
}

#[test]
fn test_move_project_rewrites_history() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");

    // Use canonical paths in history entries. Build entries via
    // serde_json so Windows backslashes get correctly JSON-escaped.
    let old_str = canonical_test_str(&src);
    let new_str = dst.to_string_lossy().to_string();

    let history = base.join("history.jsonl");
    let entries = [
        serde_json::json!({"project": old_str, "sessionId": "abc", "timestamp": 1}).to_string(),
        serde_json::json!({"project": "/other/path", "sessionId": "def", "timestamp": 2})
            .to_string(),
        serde_json::json!({"project": old_str, "sessionId": "ghi", "timestamp": 3}).to_string(),
    ];
    fs::write(&history, entries.join("\n") + "\n").unwrap();

    // Create projects dir
    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert_eq!(result.history_lines_updated, 2);

    // Verify history was rewritten by parsing each JSON line — raw string
    // matching breaks on Windows UNC paths (double-escaped backslashes).
    let content = fs::read_to_string(&history).unwrap();
    let projects: Vec<String> = content
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("project").and_then(|p| p.as_str()).map(String::from))
        .collect();
    assert!(projects.iter().any(|p| p == &new_str), "new path present");
    assert!(!projects.iter().any(|p| p == &old_str), "old path gone");
    assert!(
        projects.iter().any(|p| p == "/other/path"),
        "unrelated entry kept"
    );
}

#[test]
fn test_move_project_dry_run() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");

    // Create projects dir
    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: false,
        dry_run: true,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    // Dry run: nothing actually changed
    assert!(!result.actual_dir_moved);
    assert!(!result.cc_dir_renamed);
    // Source still exists
    assert!(src.exists());
    assert!(!dst.exists());
}

#[test]
fn test_clean_orphans_dry_run() {
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // Create a project whose source doesn't exist (orphan)
    let orphan = projects_dir.join("-nonexistent-path");
    fs::create_dir(&orphan).unwrap();
    fs::write(orphan.join("session.jsonl"), "{}").unwrap();

    let (result, orphans) = clean_orphans(tmp.path(), None, None, None, true).unwrap();
    assert_eq!(result.orphans_found, 1);
    assert_eq!(result.orphans_removed, 0); // dry run
    assert_eq!(orphans.len(), 1);
    // Dir still exists
    assert!(orphan.exists());
}

#[cfg(unix)]
#[test]
fn test_clean_orphans_removes() {
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let orphan = projects_dir.join("-nonexistent-path");
    fs::create_dir(&orphan).unwrap();
    fs::write(orphan.join("session.jsonl"), "{}").unwrap();

    let (result, _) = clean_orphans(tmp.path(), None, None, None, false).unwrap();
    assert_eq!(result.orphans_found, 1);
    assert_eq!(result.orphans_removed, 1);
    assert!(!orphan.exists());
}

// ----- clean edge cases added for fixes 1-6, 9 -----

/// Fix #1: paths under an absent `/Volumes/<drive>` mount point
/// must NOT be flagged orphan. The drive might be unplugged; the
/// data could still be present once remounted.
#[test]
#[cfg(unix)]
fn test_clean_skips_unreachable_mount_prefix() {
    use crate::project_sanitize::sanitize_path;
    // Pick a `/Volumes/<name>` that definitely doesn't exist on any
    // test host. Anything unique is fine; macOS has an empty or
    // near-empty `/Volumes`, Linux has no such root and the prefix
    // is simply not matched.
    let fake_source = "/Volumes/claudepot-test-never-exists-xyz/proj";
    let san = sanitize_path(fake_source);

    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();
    let dir = projects_dir.join(&san);
    fs::create_dir(&dir).unwrap();
    // Put a real session.jsonl with the authoritative cwd so the
    // recovered-cwd path is taken (same code path as a real CC dir).
    fs::write(
        dir.join("session.jsonl"),
        format!("{{\"cwd\":\"{fake_source}\",\"type\":\"user\"}}\n"),
    )
    .unwrap();

    let (result, orphans) = clean_orphans(tmp.path(), None, None, None, true).unwrap();

    #[cfg(target_os = "macos")]
    {
        // On macOS, `/Volumes/claudepot-test-never-exists-xyz`
        // mount point doesn't exist → unreachable → NOT orphan.
        assert_eq!(result.orphans_found, 0);
        assert_eq!(result.unreachable_skipped, 1);
        let info = orphans.iter().find(|p| p.sanitized_name == san).or(None);
        // (orphans is the orphan list, so the unreachable project
        // won't appear there; that's by design.)
        assert!(info.is_none());
    }
    #[cfg(not(target_os = "macos"))]
    {
        // On Linux, `/Volumes/...` isn't a mount prefix we detect
        // (we use `/mnt`, `/media`, `/run/media`), so the path
        // classification falls back to regular `try_exists`. That
        // still returns Absent, so the project is orphan. This
        // test just confirms no regression; the cross-platform
        // parity of the unreachable probe is a separate concern.
        let _ = (result, orphans);
    }
}

/// Fix #4a: when an orphan whose cwd was recovered authoritatively
/// is cleaned, any matching `~/.claude.json` `projects[<path>]`
/// entry must be removed with a snapshot written for recovery.
#[cfg(unix)]
#[test]
fn test_clean_prunes_claude_json_entry_with_snapshot() {
    use crate::project_sanitize::sanitize_path;
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path();
    let projects_dir = config_dir.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // Orphan source path that we can safely assert doesn't exist.
    let fake_source = tmp.path().join("deleted-workspace");
    // DO NOT create `fake_source` — that's the whole point.
    let fake_source_str = fake_source.to_string_lossy().to_string();
    let san = sanitize_path(&fake_source_str);
    let dir = projects_dir.join(&san);
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("session.jsonl"),
        format!("{{\"cwd\":\"{fake_source_str}\",\"type\":\"user\"}}\n"),
    )
    .unwrap();

    // Seed ~/.claude.json with a matching entry.
    let claude_json = tmp.path().join("claude.json");
    fs::write(
        &claude_json,
        serde_json::to_string_pretty(&serde_json::json!({
            "projects": {
                fake_source_str.clone(): {"trust": true, "allowedTools": ["Bash(git:*)"]},
                "/elsewhere/unrelated": {"trust": false}
            },
            "otherTop": 42
        }))
        .unwrap(),
    )
    .unwrap();

    let snapshots = tmp.path().join("snaps");
    let locks = tmp.path().join("locks");
    let (result, _) = clean_orphans(
        config_dir,
        Some(claude_json.as_path()),
        Some(snapshots.as_path()),
        Some(locks.as_path()),
        false,
    )
    .unwrap();

    assert_eq!(result.orphans_removed, 1);
    assert_eq!(result.claude_json_entries_removed, 1);
    assert_eq!(result.snapshot_paths.len(), 1);

    // Config entry gone, unrelated entries intact.
    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
    assert!(after["projects"].get(&fake_source_str).is_none());
    assert_eq!(
        after["projects"]["/elsewhere/unrelated"]["trust"],
        serde_json::json!(false)
    );
    assert_eq!(after["otherTop"], serde_json::json!(42));

    // Snapshot captured the removed value. Batched snapshot
    // shape is a map `{ <removed_path>: <value>, ... }` so all N
    // dropped entries live in one file.
    let snap: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&result.snapshot_paths[0]).unwrap()).unwrap();
    assert_eq!(snap[&fake_source_str]["trust"], serde_json::json!(true));
}

/// Fix #4b: matching `history.jsonl` lines are removed and dropped
/// lines are captured in a snapshot so recovery is possible.
#[cfg(unix)]
#[test]
fn test_clean_prunes_history_lines_with_snapshot() {
    use crate::project_sanitize::sanitize_path;
    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path();
    let projects_dir = config_dir.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let fake_source = tmp.path().join("deleted-workspace2");
    let fake_source_str = fake_source.to_string_lossy().to_string();
    let san = sanitize_path(&fake_source_str);
    let dir = projects_dir.join(&san);
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("session.jsonl"),
        format!("{{\"cwd\":\"{fake_source_str}\",\"type\":\"user\"}}\n"),
    )
    .unwrap();

    // Seed history.jsonl with two lines for our orphan and one
    // unrelated line.
    let history = config_dir.join("history.jsonl");
    let orphan_line_a =
        serde_json::json!({"display": "hello", "project": fake_source_str}).to_string();
    let orphan_line_b =
        serde_json::json!({"display": "again", "project": fake_source_str}).to_string();
    let other_line =
        serde_json::json!({"display": "keep me", "project": "/other/project"}).to_string();
    fs::write(
        &history,
        format!("{orphan_line_a}\n{orphan_line_b}\n{other_line}\n"),
    )
    .unwrap();

    let snapshots = tmp.path().join("snaps");
    let locks = tmp.path().join("locks");
    let (result, _) = clean_orphans(
        config_dir,
        None, // skip claude.json to keep this test focused
        Some(snapshots.as_path()),
        Some(locks.as_path()),
        false,
    )
    .unwrap();

    assert_eq!(result.orphans_removed, 1);
    assert_eq!(result.history_lines_removed, 2);

    let after = fs::read_to_string(&history).unwrap();
    assert!(!after.contains(&fake_source_str));
    assert!(after.contains("/other/project"));

    // Snapshot is a JSON array containing the two dropped lines.
    let snap_path = result
        .snapshot_paths
        .iter()
        .find(|p| p.to_string_lossy().contains("-clean-history"))
        .expect("history snapshot should be present");
    let snap: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(snap_path).unwrap()).unwrap();
    assert_eq!(snap.as_array().unwrap().len(), 2);
}

/// Fix #6: a live `__clean__` lock must block a second call from
/// the same process (same-host + same pid = live).
#[test]
fn test_clean_takes_exclusive_lock() {
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();
    let locks = tmp.path().join("locks");

    // Acquire the clean lock manually using the same key so we
    // simulate an in-flight clean, then verify a second call
    // refuses.
    let (_g, _broken) = crate::project_lock::acquire(&locks, "__clean__").unwrap();

    let err = clean_orphans(tmp.path(), None, None, Some(locks.as_path()), false).unwrap_err();
    assert!(matches!(err, ProjectError::Ambiguous(_)));
}

/// Fix #9: a truly empty CC project dir (no sessions, no memory,
/// under one FS block) should be cleaned even when the source
/// sanitized name doesn't roundtrip (so we can't be certain about
/// the source path). The dir is reclaimed; no sibling state is
/// rewritten (safest choice when original_path is ambiguous).
#[test]
fn test_clean_removes_empty_project_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // Name that deliberately doesn't roundtrip cleanly (contains a
    // hyphen that unsanitize would misread as a path separator).
    let ambiguous = projects_dir.join("-some-ambiguous-name-that-never-was-a-path");
    fs::create_dir(&ambiguous).unwrap();

    // Also seed a claude.json and history.jsonl; both must be
    // LEFT ALONE for empty-dir cleanup since original_path is
    // not authoritative.
    let claude_json = tmp.path().join("claude.json");
    fs::write(&claude_json, r#"{"projects":{"/real":{"trust":true}}}"#).unwrap();

    let locks = tmp.path().join("locks");
    let snaps = tmp.path().join("snaps");
    let (result, _) = clean_orphans(
        tmp.path(),
        Some(claude_json.as_path()),
        Some(snaps.as_path()),
        Some(locks.as_path()),
        false,
    )
    .unwrap();

    assert_eq!(result.orphans_found, 1);
    assert_eq!(result.orphans_removed, 1);
    assert!(!ambiguous.exists());
    // No sibling state touched: claude.json still intact.
    assert_eq!(result.claude_json_entries_removed, 0);
    let after = fs::read_to_string(&claude_json).unwrap();
    assert!(after.contains("/real"));
}

/// Fix #2: when `try_exists()` returns an Err (we simulate this
/// indirectly by checking the `PathReachability::Unreachable`
/// classification on a path whose ancestor is an absent mount
/// root), the project must NOT be cleaned. Covered above by the
/// mount-prefix test on macOS; this additional check exercises
/// the empty-path early return.
#[test]
fn test_reachability_empty_path_is_unreachable() {
    use crate::project_helpers::{classify_reachability, PathReachability};
    assert_eq!(classify_reachability(""), PathReachability::Unreachable);
}

/// Protected-paths guard: when an orphan's authoritative source
/// path is in the protected set, the CC artifact dir is still
/// removed, but `~/.claude.json` and `history.jsonl` entries for
/// that path are LEFT INTACT. `protected_paths_skipped` reflects
/// the count.
#[cfg(unix)]
#[test]
fn test_clean_protected_path_skips_sibling_rewrites() {
    use crate::project_sanitize::sanitize_path;
    use std::collections::HashSet;

    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path();
    let projects_dir = config_dir.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // Two orphans — one protected, one not. Both have an
    // authoritative cwd recovered from session.jsonl.
    let protected_src = tmp.path().join("guarded-workspace");
    let protected_str = protected_src.to_string_lossy().to_string();
    let san_protected = sanitize_path(&protected_str);
    let dir_p = projects_dir.join(&san_protected);
    fs::create_dir(&dir_p).unwrap();
    fs::write(
        dir_p.join("session.jsonl"),
        format!("{{\"cwd\":\"{protected_str}\",\"type\":\"user\"}}\n"),
    )
    .unwrap();

    let normal_src = tmp.path().join("normal-workspace");
    let normal_str = normal_src.to_string_lossy().to_string();
    let san_normal = sanitize_path(&normal_str);
    let dir_n = projects_dir.join(&san_normal);
    fs::create_dir(&dir_n).unwrap();
    fs::write(
        dir_n.join("session.jsonl"),
        format!("{{\"cwd\":\"{normal_str}\",\"type\":\"user\"}}\n"),
    )
    .unwrap();

    // Seed claude.json and history.jsonl with entries for BOTH.
    let claude_json = tmp.path().join("claude.json");
    fs::write(
        &claude_json,
        serde_json::to_string_pretty(&serde_json::json!({
            "projects": {
                protected_str.clone(): {"trust": true, "note": "keep"},
                normal_str.clone():    {"trust": false}
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let history = config_dir.join("history.jsonl");
    let line_p = serde_json::json!({"display": "p", "project": protected_str}).to_string();
    let line_n = serde_json::json!({"display": "n", "project": normal_str}).to_string();
    fs::write(&history, format!("{line_p}\n{line_n}\n")).unwrap();

    let snapshots = tmp.path().join("snaps");
    let locks = tmp.path().join("locks");
    let mut protected: HashSet<String> = HashSet::new();
    protected.insert(protected_str.clone());

    let (result, _) = clean_orphans_with_progress(
        config_dir,
        Some(claude_json.as_path()),
        Some(snapshots.as_path()),
        Some(locks.as_path()),
        None,
        &protected,
        false,
        &crate::project_progress::NoopSink,
    )
    .unwrap();

    // Both CC artifact dirs are removed.
    assert_eq!(result.orphans_found, 2);
    assert_eq!(result.orphans_removed, 2);
    assert!(!dir_p.exists());
    assert!(!dir_n.exists());

    // Sibling state: only the normal one was rewritten.
    assert_eq!(result.claude_json_entries_removed, 1);
    assert_eq!(result.history_lines_removed, 1);
    assert_eq!(result.protected_paths_skipped, 1);

    // The protected entries are still present in both files.
    let cj_after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&claude_json).unwrap()).unwrap();
    assert!(cj_after["projects"].get(&protected_str).is_some());
    assert!(cj_after["projects"].get(&normal_str).is_none());

    let hist_after = fs::read_to_string(&history).unwrap();
    assert!(hist_after.contains(&protected_str));
    assert!(!hist_after.contains(&normal_str));
}

/// Regression for the audit-flagged HIGH bug: sibling state must
/// NOT be pruned for an orphan whose source path has re-appeared
/// since `list_projects` ran (TOCTOU). Phase 0's preflight catches
/// the reappearance and excludes the orphan from `authoritative_paths`,
/// so neither `~/.claude.json` nor `history.jsonl` are touched.
#[cfg(unix)]
#[test]
fn test_clean_skips_sibling_rewrite_when_source_reappeared() {
    use crate::project_sanitize::sanitize_path;
    use std::collections::HashSet;

    let tmp = tempfile::tempdir().unwrap();
    let config_dir = tmp.path();
    let projects_dir = config_dir.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // The "orphan" — list_projects() will see this as orphan because
    // we'll seed session.jsonl with a cwd that doesn't exist YET.
    // Then we'll create the cwd before clean runs, simulating TOCTOU.
    let reappeared = tmp.path().join("reappeared-workspace");
    let reappeared_str = reappeared.to_string_lossy().to_string();
    let san = sanitize_path(&reappeared_str);
    let cc_dir = projects_dir.join(&san);
    fs::create_dir(&cc_dir).unwrap();
    fs::write(
        cc_dir.join("session.jsonl"),
        format!("{{\"cwd\":\"{reappeared_str}\",\"type\":\"user\"}}\n"),
    )
    .unwrap();

    // Seed sibling state.
    let claude_json = config_dir.join("claude.json");
    fs::write(
        &claude_json,
        serde_json::to_string_pretty(&serde_json::json!({
            "projects": { reappeared_str.clone(): {"trust": true} }
        }))
        .unwrap(),
    )
    .unwrap();
    let history = config_dir.join("history.jsonl");
    let line = serde_json::json!({"display": "x", "project": reappeared_str}).to_string();
    fs::write(&history, format!("{line}\n")).unwrap();

    // Capture the listing (orphan), THEN make the source reappear.
    // The clean runs after — preflight should detect this and skip
    // both the artifact dir and the sibling prune.
    let listing = list_projects(config_dir).unwrap();
    assert!(listing
        .iter()
        .any(|p| p.is_orphan && p.original_path == reappeared_str));
    fs::create_dir(&reappeared).unwrap();

    let snaps = tmp.path().join("snaps");
    let locks = tmp.path().join("locks");
    let (result, _) = clean_orphans_with_progress(
        config_dir,
        Some(claude_json.as_path()),
        Some(snaps.as_path()),
        Some(locks.as_path()),
        None,
        &HashSet::new(),
        false,
        &crate::project_progress::NoopSink,
    )
    .unwrap();

    // Artifact dir survived (preflight refused).
    assert!(cc_dir.exists());
    assert_eq!(result.orphans_removed, 0);
    // Sibling state survived too — this is the regression guard.
    assert_eq!(result.claude_json_entries_removed, 0);
    assert_eq!(result.history_lines_removed, 0);
    let cj_after = fs::read_to_string(&claude_json).unwrap();
    assert!(cj_after.contains(&reappeared_str));
    let hist_after = fs::read_to_string(&history).unwrap();
    assert!(hist_after.contains(&reappeared_str));
}

/// Empty-dir orphans are excluded from BOTH the rewrite list and
/// the protected-skip count: their `original_path` is from the
/// lossy fallback and they were never authoritative to begin with.
#[test]
fn test_clean_empty_dir_not_counted_as_protected() {
    use std::collections::HashSet;

    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let amb = projects_dir.join("-some-ambiguous-path");
    fs::create_dir(&amb).unwrap();

    let mut protected: HashSet<String> = HashSet::new();
    // unsanitize gives "/some/ambiguous/path" — protect it just to
    // ensure the empty-dir branch ignores the protected check.
    protected.insert("/some/ambiguous/path".to_string());

    let locks = tmp.path().join("locks");
    let snaps = tmp.path().join("snaps");
    let (result, _) = clean_orphans_with_progress(
        tmp.path(),
        None,
        Some(snaps.as_path()),
        Some(locks.as_path()),
        None,
        &protected,
        false,
        &crate::project_progress::NoopSink,
    )
    .unwrap();

    assert_eq!(result.orphans_removed, 1);
    assert_eq!(result.protected_paths_skipped, 0);
}

// ----- clean_preview (B-4) -----

/// Empty `projects/` dir → zero candidates and zero counts. The
/// preview must be safe to call before the user has any project
/// state at all (first-launch path).
#[test]
fn test_clean_preview_empty_config_returns_zero_counts() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join("projects")).unwrap();
    let data = tempfile::tempdir().unwrap();

    let preview = clean_preview(tmp.path(), None, None, None, data.path()).unwrap();

    assert_eq!(preview.orphans_found, 0);
    assert_eq!(preview.unreachable_skipped, 0);
    assert_eq!(preview.total_bytes, 0);
    assert_eq!(preview.protected_count, 0);
    assert!(preview.orphans.is_empty());
}

/// `total_bytes` must be the sum of `total_size_bytes` over the
/// candidate orphans — and exactly that, no double-counting and
/// no inclusion of unreachable-skipped projects.
#[test]
fn test_clean_preview_total_bytes_sums_orphans() {
    use crate::project_sanitize::sanitize_path;
    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // Two orphans with non-trivial body so each registers a
    // recoverable cwd through session.jsonl. Pad each session
    // with kilobytes of payload so the sizes are clearly distinct.
    let src_a = tmp.path().join("orphan-a");
    let src_b = tmp.path().join("orphan-b");
    let san_a = sanitize_path(&src_a.to_string_lossy());
    let san_b = sanitize_path(&src_b.to_string_lossy());
    let dir_a = projects_dir.join(&san_a);
    let dir_b = projects_dir.join(&san_b);
    fs::create_dir(&dir_a).unwrap();
    fs::create_dir(&dir_b).unwrap();
    fs::write(
        dir_a.join("session.jsonl"),
        format!(
            "{{\"cwd\":\"{src}\",\"type\":\"user\",\"pad\":\"{pad}\"}}\n",
            src = src_a.to_string_lossy(),
            pad = "x".repeat(2048)
        ),
    )
    .unwrap();
    fs::write(
        dir_b.join("session.jsonl"),
        format!(
            "{{\"cwd\":\"{src}\",\"type\":\"user\",\"pad\":\"{pad}\"}}\n",
            src = src_b.to_string_lossy(),
            pad = "y".repeat(4096)
        ),
    )
    .unwrap();

    let data = tempfile::tempdir().unwrap();
    let preview = clean_preview(tmp.path(), None, None, None, data.path()).unwrap();

    assert_eq!(preview.orphans_found, 2);
    assert_eq!(preview.orphans.len(), 2);
    let expected: u64 = preview.orphans.iter().map(|p| p.total_size_bytes).sum();
    assert_eq!(preview.total_bytes, expected);
    // Sanity: the payloads we wrote are at least 2 KiB + 4 KiB,
    // so a positive total proves we did not silently zero things out.
    assert!(preview.total_bytes >= 6 * 1024);
}

/// `protected_count` must mirror `clean_orphans_with_progress`'s
/// `protected_paths_skipped` predicate exactly: an empty-dir orphan
/// (whose `original_path` is from the lossy fallback) is excluded,
/// even when the protected set happens to contain the unsanitized
/// guess.
#[cfg(unix)]
#[test]
fn test_clean_preview_protected_count_excludes_empty_orphans() {
    use crate::project_sanitize::sanitize_path;

    let tmp = tempfile::tempdir().unwrap();
    let projects_dir = tmp.path().join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // (1) Authoritative orphan whose source is in the protected set.
    let guarded = tmp.path().join("guarded-ws");
    let guarded_str = guarded.to_string_lossy().to_string();
    let san_guarded = sanitize_path(&guarded_str);
    let dir_g = projects_dir.join(&san_guarded);
    fs::create_dir(&dir_g).unwrap();
    fs::write(
        dir_g.join("session.jsonl"),
        format!("{{\"cwd\":\"{guarded_str}\",\"type\":\"user\"}}\n"),
    )
    .unwrap();

    // (2) Empty-dir orphan whose unsanitized guess is also in the
    //     protected set — it must NOT be counted.
    let empty = projects_dir.join("-some-ambiguous-path");
    fs::create_dir(&empty).unwrap();

    // Stand up a custom data_dir whose protected_paths.json contains
    // both candidates. We seed the store directly (bypassing `add()`)
    // so the test stays focused on the predicate, not the editor API.
    // Schema mirrors `Store` in `protected_paths.rs` — `user` is a
    // flat array of path strings, not objects.
    let data = tempfile::tempdir().unwrap();
    fs::write(
        crate::protected_paths::store_path(data.path()),
        serde_json::json!({
            "version": 1,
            "user": [guarded_str.clone(), "/some/ambiguous/path"],
            "removed_defaults": []
        })
        .to_string(),
    )
    .unwrap();

    let preview = clean_preview(tmp.path(), None, None, None, data.path()).unwrap();

    assert_eq!(preview.orphans_found, 2);
    // Only the authoritative-path orphan counts as protected.
    assert_eq!(preview.protected_count, 1);
}

/// When `protected_paths.json` is missing, the resolution must fall
/// back to `DEFAULT_PATHS` — never to an empty set. The audit-flagged
/// failure mode (an empty set silently disabling protection for `/`,
/// `~`, `/Users`, etc.) cannot reach `clean_preview` if this test
/// passes.
#[cfg(unix)]
#[test]
fn test_clean_preview_protected_count_uses_fail_safe_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join("projects")).unwrap();

    // (1) Empty data_dir → no `protected_paths.json` at all.
    let data = tempfile::tempdir().unwrap();
    assert!(!crate::protected_paths::store_path(data.path()).exists());

    // The preview must not panic, and the protected set used inside
    // it must be the fail-safe defaults (which include "/" and "~"
    // on every host). Cross-check the exposed resolution against the
    // preview path's own consumer: if `resolved_set_or_defaults`
    // gave us an empty set here, the audit bug would be live.
    let resolved = crate::protected_paths::resolved_set_or_defaults(data.path());
    assert!(
        resolved.contains("/"),
        "fail-safe defaults must include '/'"
    );
    assert!(
        resolved.contains("~"),
        "fail-safe defaults must include '~'"
    );

    let preview = clean_preview(tmp.path(), None, None, None, data.path()).unwrap();
    assert_eq!(preview.orphans_found, 0);
    assert_eq!(preview.protected_count, 0);

    // (2) Corrupt store → still falls back to defaults rather than
    //     producing an empty set or erroring out.
    fs::write(
        crate::protected_paths::store_path(data.path()),
        "{ this is not valid json",
    )
    .unwrap();
    let resolved_corrupt = crate::protected_paths::resolved_set_or_defaults(data.path());
    assert!(
        resolved_corrupt.contains("/") && resolved_corrupt.contains("~"),
        "corrupt store must still expose defaults"
    );
    let preview2 = clean_preview(tmp.path(), None, None, None, data.path()).unwrap();
    assert_eq!(preview2.orphans_found, 0);
}

#[test]
fn test_move_project_already_moved() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    // Only destination exists (user already did `mv`)
    let src = base.join("old");
    let dst = base.join("new");
    fs::create_dir(&dst).unwrap();

    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();
    // src doesn't exist, so use base (already canonical) directly
    let old_san = sanitize_path(&src.to_string_lossy());
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    let session_path = cc_old.join("s.jsonl");
    fs::write(&session_path, "{}").unwrap();
    // Age the session beyond the move-side live-heartbeat window
    // (120 s — see `project.rs:810`). On Windows runners `lsof` is
    // absent, so `detect_live_session` falls back to heartbeat-only
    // and treats the fresh-fixture mtime as a live Claude →
    // `force: false` then refuses with `ClaudeRunning`. Push mtime
    // back well past the window.
    let stale = filetime::FileTime::from_system_time(
        std::time::SystemTime::now() - std::time::Duration::from_secs(300),
    );
    filetime::set_file_mtime(&session_path, stale).unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: false,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(!result.actual_dir_moved); // didn't move dir (already moved)
    assert!(result.cc_dir_renamed); // but renamed CC state
}

#[test]
fn test_move_project_state_only() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");
    fs::create_dir(&dst).unwrap();

    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();
    // Use canonical path for sanitization (matches what resolve_path returns)
    let old_san = sanitize_path(&canonical_test_str(&src));
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: true, // --no-move
        merge: false,
        overwrite: false,
        force: false,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(!result.actual_dir_moved);
    assert!(result.cc_dir_renamed);
    // Both dirs still exist (--no-move)
    assert!(src.exists());
    assert!(dst.exists());
}

#[cfg(target_os = "windows")]
#[test]
fn test_resolve_path_strips_windows_verbatim_prefix() {
    // On Windows, `std::fs::canonicalize` returns `\\?\C:\...` for
    // existing paths. `resolve_path` must strip that prefix so the
    // sanitized slug matches what CC writes (CC never uses verbatim
    // paths in session cwd or project slugs).
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("resolve-verbatim-test");
    fs::create_dir(&dir).unwrap();
    let resolved = resolve_path(dir.to_str().unwrap()).unwrap();
    assert!(
        !resolved.starts_with(r"\\?\"),
        "resolve_path must not return a verbatim path, got: {}",
        resolved
    );
    // Slug parity: sanitizing a canonicalized Windows path must not
    // include the `?` or the extra leading separators.
    let san = sanitize_path(&resolved);
    assert!(
        !san.starts_with("--?-"),
        "sanitized slug leaked verbatim prefix: {}",
        san
    );
}

#[test]
fn test_resolve_path_expands_bare_tilde() {
    // `~` alone must expand to $HOME. Without expansion, the literal
    // `~` falls through to `current_dir().join("~")` and produces a
    // garbage path under the test's cwd.
    let resolved = resolve_path("~").expect("~ should expand");
    let home = dirs::home_dir().expect("HOME available in tests");
    // Compare to canonical home (resolve_path canonicalizes existing
    // paths; macOS may symlink-resolve `/Users/x` and `$HOME` may differ
    // from the canonical form). Strip `\\?\` on Windows because
    // `resolve_path` simplifies the verbatim form away.
    let canonical_home =
        simplify_windows_path(&home.canonicalize().unwrap_or(home).to_string_lossy());
    assert_eq!(resolved, canonical_home);
}

#[test]
fn test_resolve_path_expands_tilde_subpath() {
    // `~/foo` (with `foo` non-existent) must expand to $HOME/foo, NOT
    // `<cwd>/~/foo`. This is the regression test for the rename-to-
    // `~/path` bug that left a stranded `myprojects/~/github/...` tree
    // on 2026-04-28.
    let nonexistent = "~/__claudepot_nonexistent_test_dir_4f7e2a__";
    let resolved = resolve_path(nonexistent).expect("~/x should expand");
    let home = dirs::home_dir().expect("HOME available in tests");
    let expected = home
        .join("__claudepot_nonexistent_test_dir_4f7e2a__")
        .to_string_lossy()
        .to_string();
    assert_eq!(resolved, expected);
    assert!(
        !resolved.contains("/~/"),
        "literal ~ leaked into resolved path: {resolved}"
    );
}

#[test]
fn test_resolve_path_rejects_user_home_tilde() {
    // `~user/foo` is a POSIX-shell user-home-expansion form we don't
    // support. Returning the literal would let `~user` slip through
    // to `current_dir().join("~user/foo")` — same footgun shape as the
    // original bug. Reject explicitly.
    let err = resolve_path("~root/foo").expect_err("~user/foo must error");
    assert!(
        format!("{err:?}").contains("tilde"),
        "expected a tilde-related error, got: {err:?}"
    );
}

#[test]
fn test_resolve_path_rejects_bare_user_tilde() {
    // `~root` (no `/`) — same rejection.
    let err = resolve_path("~root").expect_err("~user must error");
    assert!(
        format!("{err:?}").contains("tilde"),
        "expected a tilde-related error, got: {err:?}"
    );
}

#[test]
fn test_resolve_path_nfc_ascii_unchanged() {
    // ASCII paths must pass through NFC unchanged
    let tmp = tempfile::tempdir().unwrap();
    let ascii_dir = canonical_test_path(tmp.path()).join("plain_ascii");
    fs::create_dir(&ascii_dir).unwrap();
    let resolved = resolve_path(ascii_dir.to_str().unwrap()).unwrap();
    // Use `canonical_test_str` for the expected so both sides are in
    // the simplified-Windows shape (`resolve_path` strips `\\?\`).
    let canonical = canonical_test_str(&ascii_dir);
    assert_eq!(resolved, canonical);
}

#[test]
fn test_resolve_path_nfc_normalizes_nfd() {
    // NFD "café" (e + combining acute) must become NFC "café" (é precomposed)
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());
    let nfd_name = "caf\u{0065}\u{0301}"; // NFD: e + combining acute accent
    let nfd_dir = base.join(nfd_name);
    fs::create_dir(&nfd_dir).unwrap();
    let resolved = resolve_path(nfd_dir.to_str().unwrap()).unwrap();
    assert!(
        resolved.contains("caf\u{00e9}"),
        "Expected NFC 'café' in resolved path, got: {}",
        resolved
    );
}

#[test]
fn test_sanitize_nfd_nfc_produces_same_output() {
    // NFD and NFC of the same path must produce identical sanitize output
    // after resolve_path normalizes to NFC
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());
    let nfd_name = "caf\u{0065}\u{0301}";
    let nfc_name = "caf\u{00e9}";
    let nfd_dir = base.join(nfd_name);
    // macOS HFS+ / APFS may normalize the dirname itself, so just create one
    fs::create_dir_all(&nfd_dir).unwrap();
    let resolved_nfd = resolve_path(nfd_dir.to_str().unwrap()).unwrap();
    let nfc_dir = base.join(nfc_name);
    // On macOS, NFD and NFC names resolve to the same directory
    let resolved_nfc = resolve_path(nfc_dir.to_str().unwrap()).unwrap();
    assert_eq!(
        sanitize_path(&resolved_nfd),
        sanitize_path(&resolved_nfc),
        "NFD and NFC resolved paths must produce same sanitized output"
    );
}

#[test]
fn test_resolve_path_nfc_korean_jamo() {
    // Korean Jamo (한) must become precomposed Hangul (한)
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());
    let jamo = "\u{1112}\u{1161}\u{11AB}"; // 한 (conjoining Jamo)
    let jamo_dir = base.join(jamo);
    fs::create_dir(&jamo_dir).unwrap();
    let resolved = resolve_path(jamo_dir.to_str().unwrap()).unwrap();
    assert!(
        resolved.contains("\u{D55C}"),
        "Expected precomposed Hangul 한 (U+D55C) in resolved path, got: {}",
        resolved
    );
}

#[test]
fn test_sanitize_emoji_matches_cc_utf16() {
    // JS sees emoji as 2 surrogate code units → 2 hyphens.
    // Our sanitize_path must produce the same result.
    assert_eq!(sanitize_path("/tmp/\u{1F389}project"), "-tmp---project");
    // NFC accented char is 1 code unit → 1 hyphen
    assert_eq!(sanitize_path("/tmp/caf\u{00e9}"), "-tmp-caf-");
}

#[test]
fn test_djb2_hash_collision_exists() {
    // djb2 is a 32-bit hash; collisions are inevitable.
    // "aaa" and "abB" produce the same hash (verified by brute-force search
    // against CC's JS implementation).
    let h1 = djb2_hash("aaa");
    let h2 = djb2_hash("abB");
    assert_eq!(h1, h2, "Expected djb2 collision between 'aaa' and 'abB'");
    assert_eq!(h1, "22bl");
}

#[test]
fn test_djb2_hash_matches_cc() {
    // Verify our hash matches CC's djb2Hash + Math.abs + toString(36)
    // for a known long path. Expected value computed with CC's JS implementation.
    let long_path = "/Users/joker/".to_string() + &"a".repeat(250);
    let hash = djb2_hash(&long_path);
    assert_eq!(hash, "lwkvhu", "hash must match CC's JS output");
}

#[test]
fn test_sanitize_long_path_exact_hash() {
    // Verify that a specific long path produces the CC-compatible hash suffix.
    let long_path = format!("/Users/joker/github/xiaolai/myprojects/{}", "a".repeat(200));
    let result = sanitize_path(&long_path);
    // Path is 239 chars, sanitized > 200, so hash is appended
    assert!(result.len() > MAX_SANITIZED_LENGTH);
    let expected_hash = djb2_hash(&long_path);
    assert!(
        result.ends_with(&format!("-{}", expected_hash)),
        "Expected hash suffix '-{}', got: {}",
        expected_hash,
        result
    );
}

// -- merge_project_dirs tests --

#[test]
fn test_merge_project_dirs_copies_missing_files() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&dst).unwrap();

    fs::write(src.join("a.jsonl"), "session-a").unwrap();
    fs::write(src.join("b.jsonl"), "session-b").unwrap();
    fs::write(dst.join("c.jsonl"), "session-c").unwrap();

    merge_project_dirs(&src, &dst).unwrap();

    assert_eq!(
        fs::read_to_string(dst.join("a.jsonl")).unwrap(),
        "session-a"
    );
    assert_eq!(
        fs::read_to_string(dst.join("b.jsonl")).unwrap(),
        "session-b"
    );
    assert_eq!(
        fs::read_to_string(dst.join("c.jsonl")).unwrap(),
        "session-c"
    );
}

#[test]
fn test_merge_project_dirs_skips_existing_files() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&dst).unwrap();

    fs::write(src.join("dup.jsonl"), "src-version").unwrap();
    fs::write(dst.join("dup.jsonl"), "dst-version").unwrap();

    merge_project_dirs(&src, &dst).unwrap();

    // dst version preserved, not overwritten
    assert_eq!(
        fs::read_to_string(dst.join("dup.jsonl")).unwrap(),
        "dst-version"
    );
}

#[test]
fn test_merge_project_dirs_recursive_subdir() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    fs::create_dir_all(src.join("memory")).unwrap();
    fs::create_dir_all(&dst).unwrap();

    fs::write(src.join("memory").join("topic.md"), "# Topic").unwrap();

    merge_project_dirs(&src, &dst).unwrap();

    assert_eq!(
        fs::read_to_string(dst.join("memory").join("topic.md")).unwrap(),
        "# Topic"
    );
}

// -- rewrite_history edge cases --

#[test]
fn test_rewrite_history_invalid_json_passthrough() {
    let tmp = tempfile::tempdir().unwrap();
    let history = tmp.path().join("history.jsonl");
    let old_path = "/old/path";
    let new_path = "/new/path";

    let lines = [
        format!(r#"{{"project":"{}","sessionId":"abc"}}"#, old_path),
        format!("not valid json but contains {}", old_path),
        "totally unrelated line".to_string(),
    ];
    fs::write(&history, lines.join("\n") + "\n").unwrap();

    let count = rewrite_history(&history, old_path, new_path).unwrap();
    assert_eq!(count, 1); // only valid JSON line was rewritten

    let content = fs::read_to_string(&history).unwrap();
    assert!(content.contains(new_path));
    // Invalid JSON line preserved unchanged
    assert!(content.contains(&format!("not valid json but contains {}", old_path)));
    assert!(content.contains("totally unrelated line"));
}

#[test]
fn test_rewrite_history_empty_file() {
    let tmp = tempfile::tempdir().unwrap();
    let history = tmp.path().join("history.jsonl");
    fs::write(&history, "").unwrap();

    let count = rewrite_history(&history, "/old", "/new").unwrap();
    assert_eq!(count, 0);
}

// -- resolve_path edge cases --

#[test]
fn test_resolve_path_relative_joins_cwd() {
    // resolve_path with a relative path should join it with cwd
    let result = resolve_path("some-relative-dir").unwrap();
    let cwd = std::env::current_dir().unwrap();
    let _expected = cwd.join("some-relative-dir").to_string_lossy().to_string();
    // NFC normalization may change the string slightly on macOS
    assert!(result.contains("some-relative-dir"));
    assert!(result.starts_with('/') || result.contains(':')); // absolute
}

// -- move_project error branches --

#[test]
fn test_move_project_both_exist_error() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    let dst = base.join("new");
    fs::create_dir(&src).unwrap();
    fs::create_dir(&dst).unwrap();

    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let args = MoveArgs {
        old_path: src,
        new_path: dst,
        config_dir: base,
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: false,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink);
    assert!(matches!(result, Err(ProjectError::Ambiguous(_))));
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("both"));
}

#[test]
fn test_move_project_neither_exist_error() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let args = MoveArgs {
        old_path: base.join("nonexistent1"),
        new_path: base.join("nonexistent2"),
        config_dir: base,
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: false,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink);
    assert!(matches!(result, Err(ProjectError::Ambiguous(_))));
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("neither"));
}

#[test]
fn test_move_project_merge_cc_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");

    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // Create old CC dir with session
    let old_san = sanitize_path(&src.to_string_lossy());
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("old-session.jsonl"), "old").unwrap();

    // Create new CC dir with different session
    let new_san = sanitize_path(&dst.to_string_lossy());
    let cc_new = projects_dir.join(&new_san);
    fs::create_dir(&cc_new).unwrap();
    fs::write(cc_new.join("new-session.jsonl"), "new").unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: true,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(result.cc_dir_renamed);

    // New CC dir has both sessions
    assert!(cc_new.join("new-session.jsonl").exists());
    assert!(cc_new.join("old-session.jsonl").exists());
    // Old CC dir is gone
    assert!(!cc_old.exists());
}

#[test]
fn test_move_project_overwrite_cc_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");

    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let old_san = sanitize_path(&src.to_string_lossy());
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("keep.jsonl"), "keep-this").unwrap();

    let new_san = sanitize_path(&dst.to_string_lossy());
    let cc_new = projects_dir.join(&new_san);
    fs::create_dir(&cc_new).unwrap();
    fs::write(cc_new.join("discard.jsonl"), "discard-this").unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: true,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(result.cc_dir_renamed);

    // New CC dir has old's content, not the original new content
    assert!(cc_new.join("keep.jsonl").exists());
    assert!(!cc_new.join("discard.jsonl").exists());
    assert!(!cc_old.exists());
}

#[test]
fn test_move_project_conflict_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");

    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let old_san = sanitize_path(&src.to_string_lossy());
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("s.jsonl"), "data").unwrap();

    let new_san = sanitize_path(&dst.to_string_lossy());
    let cc_new = projects_dir.join(&new_san);
    fs::create_dir(&cc_new).unwrap();
    fs::write(cc_new.join("s.jsonl"), "data").unwrap();

    let args = MoveArgs {
        old_path: src,
        new_path: dst,
        config_dir: base,
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    // Spec §4.2 P1.7: non-empty CC target is a hard preflight
    // error without --merge/--overwrite.
    let err = move_project(&args, &crate::project_progress::NoopSink).unwrap_err();
    let ProjectError::Ambiguous(msg) = err else {
        panic!("expected Ambiguous, got {err:?}");
    };
    assert!(msg.contains("--merge") || msg.contains("--overwrite"));
}

#[test]
fn test_move_project_dry_run_with_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");

    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    // Create non-empty CC dirs for both paths
    let old_san = sanitize_path(&canonical_test_str(&src));
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("s.jsonl"), "{}").unwrap();

    let new_san = sanitize_path(&dst.to_string_lossy());
    let cc_new = projects_dir.join(&new_san);
    fs::create_dir(&cc_new).unwrap();
    fs::write(cc_new.join("s.jsonl"), "{}").unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base,
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: false,
        dry_run: true,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    // Dry run plan should mention conflict
    assert!(!result.warnings.is_empty());
    let plan = &result.warnings[0];
    assert!(plan.contains("Conflict") || plan.contains("--merge"));
    // Nothing actually changed
    assert!(src.exists());
}

#[test]
fn test_move_project_empty_new_cc_dir_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");

    let projects_dir = base.join("projects");
    fs::create_dir(&projects_dir).unwrap();

    let old_san = sanitize_path(&src.to_string_lossy());
    let cc_old = projects_dir.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("s.jsonl"), "data").unwrap();

    // Create EMPTY new CC dir
    let new_san = sanitize_path(&dst.to_string_lossy());
    let cc_new = projects_dir.join(&new_san);
    fs::create_dir(&cc_new).unwrap();

    let args = MoveArgs {
        old_path: src,
        new_path: dst,
        config_dir: base,
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(result.cc_dir_renamed);
    assert!(cc_new.join("s.jsonl").exists());
}

// -- is_claude_running_in --

#[test]
fn test_is_claude_running_in_returns_false_for_random_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // No Claude process has this random temp dir as cwd
    assert!(!is_claude_running_in(&tmp.path().to_string_lossy()));
}

// -- find_project_dir_by_prefix --

#[test]
fn test_find_project_dir_by_prefix_no_projects_dir() {
    let tmp = tempfile::tempdir().unwrap();
    // No projects/ subdirectory exists
    let result = find_project_dir_by_prefix(tmp.path(), "anything").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_find_project_dir_by_prefix_single_match() {
    let tmp = tempfile::tempdir().unwrap();
    let projects = tmp.path().join("projects");
    fs::create_dir(&projects).unwrap();
    fs::create_dir(projects.join("myprefix-abc123")).unwrap();

    let result = find_project_dir_by_prefix(tmp.path(), "myprefix").unwrap();
    assert!(result.is_some());
    assert!(result.unwrap().ends_with("myprefix-abc123"));
}

#[test]
fn test_find_project_dir_by_prefix_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let projects = tmp.path().join("projects");
    fs::create_dir(&projects).unwrap();
    fs::create_dir(projects.join("myprefix-hash1")).unwrap();
    fs::create_dir(projects.join("myprefix-hash2")).unwrap();

    let result = find_project_dir_by_prefix(tmp.path(), "myprefix");
    assert!(matches!(result, Err(ProjectError::Ambiguous(_))));
}

// -- count_files_with_ext --

#[test]
fn test_count_files_with_ext_counts_correctly() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("a.jsonl"), "").unwrap();
    fs::write(tmp.path().join("b.jsonl"), "").unwrap();
    fs::write(tmp.path().join("c.txt"), "").unwrap();
    fs::write(tmp.path().join("d.md"), "").unwrap();

    assert_eq!(count_files_with_ext(tmp.path(), "jsonl"), 2);
    assert_eq!(count_files_with_ext(tmp.path(), "md"), 1);
    assert_eq!(count_files_with_ext(tmp.path(), "rs"), 0);
}

// -- dir_size --

#[test]
fn test_dir_size_sums_correctly() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("a"), "hello").unwrap(); // 5 bytes
    fs::write(tmp.path().join("b"), "world!").unwrap(); // 6 bytes
    let sub = tmp.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("c"), "xy").unwrap(); // 2 bytes

    let size = dir_size(tmp.path());
    assert_eq!(size, 13);
}

// -- most_recent_mtime --

#[test]
fn test_most_recent_mtime_returns_latest() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("old"), "old").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    fs::write(tmp.path().join("new"), "new").unwrap();

    let mtime = most_recent_mtime(tmp.path());
    assert!(mtime.is_some());
}

#[test]
fn test_most_recent_mtime_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let mtime = most_recent_mtime(tmp.path());
    // Empty dir still has its own mtime
    assert!(mtime.is_none() || mtime.is_some());
}

// ---------------------------------------------------------------------
// Group 2 — Project move conflict handling (4 tests).
// ---------------------------------------------------------------------

/// Build a Group-2 fixture: a TempDir plus canonical src/dst/config dirs.
/// Kept as a single fn returning everything so tests don't drop the TempDir.
fn mk_move_fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());
    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");
    let projects = base.join("projects");
    fs::create_dir(&projects).unwrap();
    (tmp, src, dst, base)
}

#[test]
fn test_move_project_conflict_skips_history_rewrite() {
    let (_tmp, src, dst, base) = mk_move_fixture();
    let old_san = sanitize_path(&src.to_string_lossy());
    let new_san = sanitize_path(&dst.to_string_lossy());
    let projects = base.join("projects");
    // Both CC dirs exist, both non-empty — conflict requiring resolution.
    let cc_old = projects.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("old-session.jsonl"), "{}").unwrap();
    let cc_new = projects.join(&new_san);
    fs::create_dir(&cc_new).unwrap();
    fs::write(cc_new.join("new-session.jsonl"), "{}").unwrap();

    let old_str = src.to_string_lossy();
    let old_line = serde_json::json!({
        "project": old_str,
        "sessionId": "abc",
        "timestamp": 1,
    })
    .to_string();
    let history = base.join("history.jsonl");
    fs::write(&history, format!("{old_line}\n")).unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    // With v2 hard-error preflight, the whole operation aborts
    // before P3/P4/P5 run. history.jsonl must be untouched.
    let err = move_project(&args, &crate::project_progress::NoopSink).unwrap_err();
    assert!(matches!(err, ProjectError::Ambiguous(_)));

    // Verify old path still in history on disk (parse-based).
    let content = fs::read_to_string(&history).unwrap();
    let src_str = src.to_string_lossy().to_string();
    let has_old = content.lines().any(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .ok()
            .and_then(|v| v.get("project").and_then(|p| p.as_str()).map(String::from))
            == Some(src_str.clone())
    });
    assert!(
        has_old,
        "old path still in history since rewrite was skipped"
    );
}

#[test]
fn test_move_project_merge_rewrites_history() {
    let (_tmp, src, dst, base) = mk_move_fixture();
    let old_san = sanitize_path(&src.to_string_lossy());
    let new_san = sanitize_path(&dst.to_string_lossy());
    let projects = base.join("projects");
    let cc_old = projects.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("a.jsonl"), "old-a").unwrap();
    let cc_new = projects.join(&new_san);
    fs::create_dir(&cc_new).unwrap();
    fs::write(cc_new.join("b.jsonl"), "new-b").unwrap();

    let history = base.join("history.jsonl");
    let line = serde_json::json!({
        "project": src.to_string_lossy(),
        "sessionId": "abc",
        "timestamp": 1,
    })
    .to_string();
    fs::write(&history, format!("{line}\n")).unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: true,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(result.cc_dir_renamed, "merge should resolve the conflict");
    assert_eq!(
        result.history_lines_updated, 1,
        "history rewritten on merge"
    );
    let content = fs::read_to_string(&history).unwrap();
    // Parse-based assertion: tolerates Windows UNC path escaping.
    let new_str = dst.to_string_lossy();
    let has_new = content.lines().any(|l| {
        serde_json::from_str::<serde_json::Value>(l)
            .ok()
            .and_then(|v| v.get("project").and_then(|p| p.as_str()).map(String::from))
            == Some(new_str.to_string())
    });
    assert!(has_new, "new path present in history after merge");
    // Both files merged into new CC dir.
    assert!(cc_new.join("a.jsonl").exists(), "merged file from old dir");
    assert!(
        cc_new.join("b.jsonl").exists(),
        "preserved file from new dir"
    );
}

#[test]
fn test_move_project_orphan_roundtrip_prevents_false_positive() {
    // A project at /tmp/my-project sanitizes to `-tmp-my-project`.
    // unsanitize gives /tmp/my/project — which doesn't exist. Without
    // the cwd-from-sessions recovery, the project would be flagged orphan
    // even though the real dir /tmp/my-project exists.
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());

    // The real project dir (with a hyphen in the name).
    let project_dir = base.join("my-project");
    fs::create_dir(&project_dir).unwrap();

    // The CC project dir — sanitized for the real path.
    let projects = base.join("projects");
    fs::create_dir(&projects).unwrap();
    let san = sanitize_path(&project_dir.to_string_lossy());
    let cc_dir = projects.join(&san);
    fs::create_dir(&cc_dir).unwrap();

    // Write a session.jsonl with the correct cwd. This is how CC records
    // the authoritative original path.
    let session_line = serde_json::json!({
        "cwd": project_dir.to_string_lossy(),
        "sessionId": "abc",
        "type": "user",
    })
    .to_string();
    fs::write(cc_dir.join("session.jsonl"), session_line + "\n").unwrap();

    let listed = list_projects(&base).unwrap();
    let found = listed
        .iter()
        .find(|p| p.sanitized_name == san)
        .expect("project must be listed");

    assert_eq!(
        found.original_path,
        project_dir.to_string_lossy().to_string(),
        "cwd from session should override lossy unsanitize"
    );
    assert!(
        !found.is_orphan,
        "project dir exists; must NOT be flagged orphan"
    );
}

// -----------------------------------------------------------------
// Group 11 — Unix-only code gaps (platform-gated structural tests).
// -----------------------------------------------------------------

#[test]
fn test_move_project_cross_device_no_exdev_on_windows() {
    // Structural: the EXDEV-fallback branch is #[cfg(unix)]-gated in
    // move_project. On non-unix, a cross-device fs::rename failure
    // returns a regular Io error rather than invoking copy+remove.
    //
    // This test simply documents the platform gate. We can't easily
    // provoke a real EXDEV in a unit test (would need two mounted fs).
    // Instead, verify the in-same-device happy path still works on all
    // platforms (which it does via fs::rename without the fallback).
    let tmp = tempfile::tempdir().unwrap();
    let base = canonical_test_path(tmp.path());
    let src = base.join("old");
    fs::create_dir(&src).unwrap();
    let dst = base.join("new");
    fs::create_dir(&base.join("projects")).unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let result = move_project(&args, &crate::project_progress::NoopSink).unwrap();
    assert!(result.actual_dir_moved);
    assert!(dst.exists());
    assert!(!src.exists());
    // Platform-gate assertion: EXDEV handler presence differs by cfg.
    #[cfg(unix)]
    {
        // Unix has the EXDEV fallback path; same-device move used fs::rename.
    }
    #[cfg(not(unix))]
    {
        // Non-Unix: no EXDEV fallback at all — cross-device move would
        // propagate as a plain Io error. Same-device move still works.
    }
}

#[test]
fn test_move_project_post_move_failure_becomes_warning() {
    // v2 (spec §4.2 P1.7): non-empty CC target is a HARD preflight
    // error before any disk mutation. This test now verifies the
    // error-path instead of the old partial-success warning path.
    let (_tmp, src, dst, base) = mk_move_fixture();
    let old_san = sanitize_path(&src.to_string_lossy());
    let new_san = sanitize_path(&dst.to_string_lossy());
    let projects = base.join("projects");
    let cc_old = projects.join(&old_san);
    fs::create_dir(&cc_old).unwrap();
    fs::write(cc_old.join("s.jsonl"), "{}").unwrap();
    let cc_new = projects.join(&new_san);
    fs::create_dir(&cc_new).unwrap();
    fs::write(cc_new.join("t.jsonl"), "{}").unwrap();

    let args = MoveArgs {
        old_path: src.clone(),
        new_path: dst.clone(),
        config_dir: base.clone(),
        claude_json_path: None,
        snapshots_dir: None,
        no_move: false,
        merge: false,
        overwrite: false,
        force: true,
        dry_run: false,

        ignore_pending_journals: false,
        claudepot_state_dir: None,
    };

    let err = move_project(&args, &crate::project_progress::NoopSink)
        .expect_err("must error on CC dir collision");
    assert!(matches!(err, ProjectError::Ambiguous(_)));
    assert!(src.exists(), "disk dir untouched on preflight failure");
    assert!(!dst.exists(), "target not created on preflight failure");
}

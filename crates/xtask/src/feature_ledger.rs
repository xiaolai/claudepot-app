//! The feature ledger check, run by `cargo xtask verify-docs`.
//!
//! `docs/feature-ledger.md` lists every user-facing feature with the
//! surface it is reached from, the code behind it, its user doc and the
//! release that first shipped it. A hand-kept inventory goes stale
//! within releases — `web/design/features.md` is the precedent, never
//! touched after the commit that wrote it — so this check ties the
//! table to the source in both directions:
//!
//! - **coverage**: every GUI section, Settings pane and visible
//!   top-level CLI verb is named by some row, and every web feature
//!   page is the doc of some row. These sets are parsed from source,
//!   so adding a section, pane, verb or page without a row fails;
//! - **reference**: every code path, docs page, docs heading and
//!   version the table names exists. A row pointing at nothing reads
//!   as coverage that is not there.
//!
//! Not checked: whether a doc is good, and whether a background
//! behaviour with no surface of its own is missing a row. Neither is
//! derivable from source.

use anyhow::{bail, Result};
use std::collections::BTreeSet;
use std::path::Path;

pub const LEDGER_REL: &str = "docs/feature-ledger.md";
const REGISTRY_REL: &str = "src/sections/registry.tsx";
const CLI_MAIN_REL: &str = "crates/claudepot-cli/src/main.rs";
const CHANGELOG_REL: &str = "CHANGELOG.md";
/// A bare page name in the Docs column resolves under this directory.
const FEATURES_DIR_REL: &str = "web/src/app/(reader)/app/features";

#[derive(Debug, PartialEq)]
pub struct Row {
    pub feature: String,
    pub surfaces: Vec<String>,
    pub code: Vec<String>,
    /// `None` is the explicit `none`: a feature with no user doc.
    pub docs: Option<Docs>,
    pub since: String,
}

#[derive(Debug, PartialEq)]
pub struct Docs {
    pub page: String,
    pub heading: Option<String>,
}

/// Backticked spans of a cell, in order.
fn ticked(cell: &str) -> Vec<String> {
    cell.split('`')
        .skip(1)
        .step_by(2)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_docs(cell: &str) -> Result<Option<Docs>> {
    if cell == "none" {
        return Ok(None);
    }
    let (page_part, heading) = match cell.split_once('§') {
        Some((p, h)) => (p.trim(), Some(h.trim().to_string())),
        None => (cell, None),
    };
    let pages = ticked(page_part);
    if pages.len() != 1 || !page_part.starts_with('`') || !page_part.ends_with('`') {
        bail!("Docs cell must be `none` or `` `<page>` § <heading> ``, got {cell:?}");
    }
    if heading.as_deref() == Some("") {
        bail!("Docs cell has `§` and no heading: {cell:?}");
    }
    Ok(Some(Docs {
        page: pages[0].clone(),
        heading,
    }))
}

/// Parse the table under `## The ledger`. Strict on purpose: a table
/// this cannot read is a table whose rows are silently unchecked.
pub fn parse_ledger(md: &str) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with("## ") {
            in_table = t == "## The ledger";
            continue;
        }
        if !in_table || !t.contains('|') {
            continue;
        }
        // GitHub renders a row without its leading pipe; skipping one
        // would leave it on the page and out of the check.
        if !t.starts_with('|') {
            bail!("ledger row must start with `|`: {t}");
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 5 {
            bail!("ledger row has {} cells, expected 5: {t}", cells.len());
        }
        if cells[0] == "Feature" || cells[0].starts_with("---") {
            continue;
        }
        let surfaces = ticked(cells[1]);
        let code = ticked(cells[2]);
        if surfaces.is_empty() || code.is_empty() {
            bail!(
                "ledger row {:?} needs at least one surface and one code path",
                cells[0]
            );
        }
        rows.push(Row {
            feature: cells[0].to_string(),
            surfaces,
            code,
            docs: parse_docs(cells[3])?,
            since: cells[4].to_string(),
        });
    }
    if rows.is_empty() {
        bail!("no rows parsed from {LEDGER_REL} — the table format changed");
    }
    Ok(rows)
}

/// GUI section ids: every quoted `id: "…"` in the registry, whatever
/// the indentation or line shape. The type declarations there say
/// `id: string`, unquoted, so they never match.
fn registry_section_ids(src: &str) -> BTreeSet<String> {
    const KEY: &str = "id: \"";
    let mut out = BTreeSet::new();
    for (at, _) in src.match_indices(KEY) {
        let ident_before = src[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if ident_before {
            continue;
        }
        if let Some(id) = src[at + KEY.len()..].split('"').next() {
            out.insert(id.to_string());
        }
    }
    out
}

/// Heading texts of a markdown file, skipping fenced code — a `# ` line
/// inside a shell block is a comment, not a heading.
fn headings(md: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    // The marker that opened the current fence; only the same marker
    // closes it, so a ``` line inside a ~~~ block stays code.
    let mut fence: Option<&str> = None;
    for line in md.lines() {
        let t = line.trim_start();
        if let Some(marker) = ["```", "~~~"].into_iter().find(|m| t.starts_with(m)) {
            match fence {
                None => fence = Some(marker),
                Some(open) if open == marker => fence = None,
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            out.insert(t[hashes..].trim().to_string());
        }
    }
    out
}

/// Versions with a `## X.Y.Z` section in the changelog.
fn changelog_versions(md: &str) -> BTreeSet<String> {
    md.lines()
        .filter_map(|l| l.strip_prefix("## "))
        .filter_map(|rest| rest.split_whitespace().next())
        .filter(|v| v.split('.').count() == 3 && v.split('.').all(|p| p.parse::<u32>().is_ok()))
        .map(str::to_string)
        .collect()
}

/// The web feature pages: each directory under the features dir that
/// holds a Next.js page, MDX or not.
fn feature_pages(repo: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(repo.join(FEATURES_DIR_REL)) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            ["page.mdx", "page.tsx"]
                .iter()
                .any(|f| e.path().join(f).is_file())
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Repo-relative path a Docs page token names.
fn docs_path(page: &str) -> String {
    if page.contains('/') || page.ends_with(".md") || page.ends_with(".mdx") {
        page.to_string()
    } else {
        format!("{FEATURES_DIR_REL}/{page}/page.mdx")
    }
}

/// A path the ledger may name: relative, naming something below the
/// root, and never climbing out. `.` is the whole repo, not a feature.
fn is_repo_relative(p: &str) -> bool {
    !Path::new(p).is_absolute()
        && p.split(['/', '\\'])
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// `rel` exists under `repo` with exactly this spelling. `Path::exists`
/// ignores case on macOS, so a miscased path would pass locally and
/// fail only on Linux CI.
fn exists_exact(repo: &Path, rel: &str) -> bool {
    if !is_repo_relative(rel) {
        return false;
    }
    let mut dir = repo.to_path_buf();
    for seg in rel.split('/') {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return false;
        };
        if !entries
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_str() == Some(seg))
        {
            return false;
        }
        dir.push(seg);
    }
    true
}

/// The feature page directory a docs path lands in, if it is one.
fn feature_page_of(rel: &str) -> Option<&str> {
    rel.strip_prefix(FEATURES_DIR_REL)?
        .strip_prefix('/')?
        .split('/')
        .next()
}

pub fn check_feature_ledger(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let Ok(md) = std::fs::read_to_string(repo.join(LEDGER_REL)) else {
        problems.push(format!(
            "{LEDGER_REL} is missing — it is the one list of user-facing features"
        ));
        return Ok(());
    };
    let rows = match parse_ledger(&md) {
        Ok(r) => r,
        Err(e) => {
            problems.push(format!("{LEDGER_REL} no longer parses ({e})"));
            return Ok(());
        }
    };

    let sections = registry_section_ids(&super::verify_docs::read(repo, REGISTRY_REL)?);
    let panes = super::verify_docs::shipped_settings_panes(&super::verify_docs::read(
        repo,
        super::verify_docs::SETTINGS_PANES_SRC,
    )?);
    let verbs =
        super::verify_docs::shipped_cli_verbs(&super::verify_docs::read(repo, CLI_MAIN_REL)?);
    let versions = changelog_versions(&super::verify_docs::read(repo, CHANGELOG_REL)?);
    let pages = feature_pages(repo);
    // An empty set makes its coverage check pass over nothing.
    for (what, set, src) in [
        ("GUI sections", &sections, REGISTRY_REL),
        ("CLI verbs", &verbs, CLI_MAIN_REL),
        ("changelog versions", &versions, CHANGELOG_REL),
        ("web feature pages", &pages, FEATURES_DIR_REL),
    ] {
        if set.is_empty() {
            problems.push(format!(
                "feature ledger: parsed no {what} from {src} — the source format changed"
            ));
        }
    }

    let mut named: BTreeSet<String> = BTreeSet::new();
    let mut documented_pages: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    for row in &rows {
        let f = &row.feature;
        if !seen.insert(f.as_str()) {
            problems.push(format!("{LEDGER_REL}: feature {f:?} has two rows"));
        }

        for s in &row.surfaces {
            let known = match s.split_once(':') {
                Some(("section", id)) => sections.contains(id),
                Some(("pane", id)) => panes.contains(id),
                Some(("cli", verb)) => verbs.contains(verb),
                None if s == "background" => true,
                _ => {
                    problems.push(format!(
                        "{LEDGER_REL}: {f:?} names surface `{s}` — expected `section:<id>`, \
                         `pane:<id>`, `cli:<verb>` or `background`"
                    ));
                    continue;
                }
            };
            if known {
                named.insert(s.clone());
            } else {
                problems.push(format!(
                    "{LEDGER_REL}: {f:?} names `{s}`, which does not ship — a row \
                     pointing at nothing reads as coverage that is not there"
                ));
            }
        }

        for p in &row.code {
            if !exists_exact(repo, p) {
                problems.push(format!(
                    "{LEDGER_REL}: {f:?} names code path `{p}`, which does not exist"
                ));
            }
        }

        if let Some(docs) = &row.docs {
            let rel = docs_path(&docs.page);
            let page_md = exists_exact(repo, &rel)
                .then(|| std::fs::read_to_string(repo.join(&rel)).ok())
                .flatten();
            match page_md {
                None => problems.push(format!(
                    "{LEDGER_REL}: {f:?} is documented at `{rel}`, which does not exist"
                )),
                Some(page_md) => {
                    if let Some(page) = feature_page_of(&rel) {
                        documented_pages.insert(page.to_string());
                    }
                    if let Some(h) = &docs.heading {
                        if !headings(&page_md).contains(h) {
                            problems.push(format!(
                                "{LEDGER_REL}: {f:?} is documented under heading {h:?}, \
                                 which `{rel}` does not have"
                            ));
                        }
                    }
                }
            }
        }

        if !versions.contains(&row.since) {
            problems.push(format!(
                "{LEDGER_REL}: {f:?} has Since `{}`, which has no `## {}` section in {CHANGELOG_REL}",
                row.since, row.since
            ));
        }
    }

    let coverage = [
        ("GUI section", "section", &sections, REGISTRY_REL),
        (
            "Settings pane",
            "pane",
            &panes,
            super::verify_docs::SETTINGS_PANES_SRC,
        ),
        ("CLI verb", "cli", &verbs, CLI_MAIN_REL),
    ];
    for (what, kind, ids, src) in coverage {
        for id in ids {
            if !named.contains(&format!("{kind}:{id}")) {
                problems.push(format!(
                    "{what} `{id}` ships ({src}) but no row of {LEDGER_REL} names `{kind}:{id}`"
                ));
            }
        }
    }
    for page in pages.difference(&documented_pages) {
        problems.push(format!(
            "web feature page `{page}` is the Docs of no row in {LEDGER_REL}"
        ));
    }
    Ok(())
}

// Each test plants the defect its branch exists to catch, plus one
// clean fixture, so a check that flags everything cannot pass either.
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const REGISTRY: &str = "export const sections = [\n  {\n    id: \"accounts\",\n  },\n  {\n    id: \"settings\",\n  },\n];\n";
    const PANES: &str =
        "export const SETTINGS_PANES = [\n  { id: \"general\",\n    glyph: x },\n];\n";
    const CLI_MAIN: &str = "enum Commands {\n    /// Accounts\n    Account {\n        cmd: X,\n    },\n    #[command(hide = true)]\n    Hook {\n        cmd: Y,\n    },\n}\n";
    const CHANGELOG: &str =
        "# Changelog\n\n## 0.2.0 — beta (released 2026-01-02)\n\n## 0.1.0 — beta\n";
    const PAGE: &str = "# Accounts\n\n## Add an account\n\n```bash\n# Not a heading\n```\n";

    const LEDGER_HEAD: &str = "# Feature ledger\n\n## The ledger\n\n| Feature | Reached from | Code | Docs | Since |\n|---|---|---|---|---|\n";
    const GOOD_ROWS: &str = "\
| Accounts | `section:accounts`, `cli:account` | `src/accounts.rs` | `accounts` § Add an account | 0.1.0 |
| Settings | `section:settings`, `pane:general` | `src/settings` | `README.md` | 0.2.0 |
| Refresh | `background` | `src/refresh.rs` | none | 0.2.0 |
";

    fn fixture(rows: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        let w = |rel: &str, body: &str| {
            let p = d.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        };
        w(REGISTRY_REL, REGISTRY);
        w(super::super::verify_docs::SETTINGS_PANES_SRC, PANES);
        w(CLI_MAIN_REL, CLI_MAIN);
        w(CHANGELOG_REL, CHANGELOG);
        w(&format!("{FEATURES_DIR_REL}/accounts/page.mdx"), PAGE);
        w("README.md", "# Readme\n");
        w("src/accounts.rs", "");
        w("src/refresh.rs", "");
        fs::create_dir_all(d.path().join("src/settings")).unwrap();
        w(LEDGER_REL, &format!("{LEDGER_HEAD}{rows}"));
        d
    }

    fn run(d: &tempfile::TempDir) -> Vec<String> {
        let mut problems = Vec::new();
        check_feature_ledger(d.path(), &mut problems).expect("check ran");
        problems
    }

    fn assert_flags(rows: &str, needle: &str) {
        let out = run(&fixture(rows));
        assert!(
            out.iter().any(|p| p.contains(needle)),
            "expected {needle:?} in {out:?}"
        );
    }

    #[test]
    fn a_complete_ledger_passes() {
        assert_eq!(run(&fixture(GOOD_ROWS)), Vec::<String>::new());
    }

    #[test]
    fn a_section_with_no_row_is_caught() {
        let rows = GOOD_ROWS.replace("`section:settings`, ", "");
        assert_flags(&rows, "GUI section `settings`");
    }

    #[test]
    fn a_pane_with_no_row_is_caught() {
        let rows = GOOD_ROWS.replace(", `pane:general`", "");
        assert_flags(&rows, "Settings pane `general`");
    }

    #[test]
    fn a_cli_verb_with_no_row_is_caught() {
        let rows = GOOD_ROWS.replace(", `cli:account`", "");
        assert_flags(&rows, "CLI verb `account`");
    }

    /// Hidden verbs are not user-facing, so they need no row — and a row
    /// naming one is naming something the user cannot reach.
    #[test]
    fn a_hidden_verb_needs_no_row_and_cannot_be_named() {
        assert!(!run(&fixture(GOOD_ROWS)).iter().any(|p| p.contains("hook")));
        let rows = GOOD_ROWS.replace("`cli:account`", "`cli:account`, `cli:hook`");
        assert_flags(&rows, "`cli:hook`, which does not ship");
    }

    #[test]
    fn a_surface_that_does_not_ship_is_caught() {
        let rows = GOOD_ROWS.replace("`section:accounts`", "`section:accounts`, `section:gone`");
        assert_flags(&rows, "`section:gone`, which does not ship");
    }

    #[test]
    fn a_malformed_surface_is_caught() {
        let rows = GOOD_ROWS.replace("`background`", "`tray`");
        assert_flags(&rows, "names surface `tray`");
    }

    #[test]
    fn a_missing_code_path_is_caught() {
        let rows = GOOD_ROWS.replace("`src/refresh.rs`", "`src/refresh.rs`, `src/gone.rs`");
        assert_flags(&rows, "code path `src/gone.rs`");
    }

    /// The target exists — reached by climbing out and back in — so only
    /// the containment rule can flag it, not the existence check.
    #[test]
    fn a_code_path_outside_the_repo_is_caught() {
        let d = fixture(GOOD_ROWS);
        let name = d.path().file_name().unwrap().to_str().unwrap().to_string();
        let escape = format!("../{name}/src/refresh.rs");
        assert!(d.path().join(&escape).exists());
        let rows = GOOD_ROWS.replace("`src/refresh.rs`", &format!("`{escape}`"));
        fs::write(d.path().join(LEDGER_REL), format!("{LEDGER_HEAD}{rows}")).unwrap();
        assert!(run(&d)
            .iter()
            .any(|p| p.contains(&format!("code path `{escape}`"))));
    }

    #[test]
    fn the_repo_root_is_not_a_code_path() {
        let rows = GOOD_ROWS.replace("`src/refresh.rs`", "`.`");
        assert_flags(&rows, "code path `.`");
    }

    /// macOS resolves `SRC/refresh.rs` to `src/refresh.rs`; Linux does not.
    #[test]
    fn a_miscased_code_path_is_caught() {
        let rows = GOOD_ROWS.replace("`src/refresh.rs`", "`SRC/refresh.rs`");
        assert_flags(&rows, "code path `SRC/refresh.rs`");
    }

    #[test]
    fn a_section_written_on_one_line_is_still_seen() {
        let d = fixture(GOOD_ROWS);
        let reg = REGISTRY.replace("];", "  { id: \"zzz\", labelKey: \"x\" },\n];");
        fs::write(d.path().join(REGISTRY_REL), reg).unwrap();
        assert!(run(&d).iter().any(|p| p.contains("GUI section `zzz`")));
    }

    #[test]
    fn a_row_without_its_leading_pipe_is_caught() {
        let rows = format!("{GOOD_ROWS}Ghost | `background` | `src/gone.rs` | none | 0.2.0 |\n");
        assert_flags(&rows, "must start with `|`");
    }

    #[test]
    fn a_tsx_feature_page_needs_a_row_too() {
        let d = fixture(GOOD_ROWS);
        let p = d.path().join(FEATURES_DIR_REL).join("newtsx");
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("page.tsx"), "").unwrap();
        assert!(run(&d)
            .iter()
            .any(|p| p.contains("web feature page `newtsx`")));
    }

    /// A `# ` line inside a `~~~` fence is code, like one inside backticks.
    #[test]
    fn a_tilde_fence_hides_its_comments_too() {
        let d = fixture(GOOD_ROWS);
        let page = d.path().join(FEATURES_DIR_REL).join("accounts/page.mdx");
        fs::write(
            &page,
            "# Accounts\n\n## Add an account\n\n~~~\n# Tilde\n```\n~~~\n",
        )
        .unwrap();
        let rows = GOOD_ROWS.replace("§ Add an account", "§ Tilde");
        fs::write(d.path().join(LEDGER_REL), format!("{LEDGER_HEAD}{rows}")).unwrap();
        assert!(run(&d).iter().any(|p| p.contains("heading \"Tilde\"")));
    }

    #[test]
    fn a_docs_path_outside_the_repo_is_caught() {
        let rows = GOOD_ROWS.replace("`README.md`", "`../README.md`");
        assert_flags(&rows, "documented at `../README.md`");
    }

    #[test]
    fn a_section_mark_with_no_heading_is_caught() {
        let rows = GOOD_ROWS.replace("`README.md`", "`README.md` §");
        assert_flags(&rows, "has `§` and no heading");
    }

    /// Each source set guards its own coverage check; parsed empty, that
    /// check would pass over nothing.
    #[test]
    fn a_source_that_parses_empty_is_caught() {
        for (rel, what) in [
            (REGISTRY_REL, "GUI sections"),
            (CLI_MAIN_REL, "CLI verbs"),
            (CHANGELOG_REL, "changelog versions"),
        ] {
            let d = fixture(GOOD_ROWS);
            fs::write(d.path().join(rel), "").unwrap();
            assert!(
                run(&d)
                    .iter()
                    .any(|p| p.contains(&format!("parsed no {what}"))),
                "{rel}"
            );
        }
        let d = fixture(GOOD_ROWS);
        fs::remove_dir_all(d.path().join(FEATURES_DIR_REL)).unwrap();
        assert!(run(&d)
            .iter()
            .any(|p| p.contains("parsed no web feature pages")));
    }

    #[test]
    fn a_missing_docs_page_is_caught() {
        let rows = GOOD_ROWS.replace("`README.md`", "`keys`");
        assert_flags(&rows, "features/keys/page.mdx`, which does not exist");
    }

    #[test]
    fn a_missing_docs_heading_is_caught() {
        let rows = GOOD_ROWS.replace("§ Add an account", "§ Remove");
        assert_flags(&rows, "heading \"Remove\"");
    }

    /// `# Not a heading` sits in a fenced shell block on the fixture page.
    #[test]
    fn a_comment_inside_a_code_fence_is_not_a_heading() {
        let rows = GOOD_ROWS.replace("§ Add an account", "§ Not a heading");
        assert_flags(&rows, "heading \"Not a heading\"");
    }

    #[test]
    fn a_since_with_no_changelog_section_is_caught() {
        let rows = GOOD_ROWS.replace("| none | 0.2.0 |", "| none | 0.3.0 |");
        assert_flags(&rows, "Since `0.3.0`");
    }

    #[test]
    fn a_feature_page_no_row_documents_is_caught() {
        let rows = GOOD_ROWS.replace("`accounts` § Add an account", "`README.md`");
        assert_flags(&rows, "web feature page `accounts`");
    }

    #[test]
    fn a_duplicate_feature_is_caught() {
        let rows =
            format!("{GOOD_ROWS}| Refresh | `background` | `src/refresh.rs` | none | 0.2.0 |\n");
        assert_flags(&rows, "has two rows");
    }

    /// A table this cannot read must fail, not pass over zero rows.
    #[test]
    fn a_table_format_change_is_caught() {
        let rows = GOOD_ROWS.replace(" | 0.1.0 |", " |");
        assert_flags(&rows, "no longer parses");
        let d = fixture(GOOD_ROWS);
        fs::write(
            d.path().join(LEDGER_REL),
            "# Feature ledger\n\nnow prose.\n",
        )
        .unwrap();
        assert!(run(&d).iter().any(|p| p.contains("no longer parses")));
    }

    #[test]
    fn a_malformed_docs_cell_is_caught() {
        let rows = GOOD_ROWS.replace("| none |", "| TODO |");
        assert_flags(&rows, "Docs cell must be");
    }

    #[test]
    fn a_missing_ledger_is_reported() {
        let d = fixture(GOOD_ROWS);
        fs::remove_file(d.path().join(LEDGER_REL)).unwrap();
        assert!(run(&d).iter().any(|p| p.contains("is missing")));
    }
}

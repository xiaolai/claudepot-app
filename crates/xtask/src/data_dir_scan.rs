//! Which files Claudepot writes into its data dir, read from the AST.
//!
//! # Why this parses instead of matching strings
//!
//! The scan this replaced was string matching, and it went blind three
//! separate times in one day — each fix a heuristic layered on the last:
//!
//! 1. **Truncating at the first `#[cfg(test)]`** to drop test scratch
//!    names. `#[cfg(test)]` also marks individual methods, so
//!    `agent/store.rs` (attribute at line 791, test module at 1280) hid
//!    ~490 lines of production code including `agents_file_path()`.
//! 2. **Reading only string literals.** `board/mod.rs` writes
//!    `claudepot_data_dir().join(BOARDS_DB_FILENAME)`. `boards.db` was
//!    therefore invisible, which is how it stayed out of
//!    `KNOWN_DB_FILENAMES` and leaked WAL sidecars while the gate
//!    reported green.
//! 3. **Resolving consts within one file only** — a const declared in
//!    one module and joined in another still slipped through.
//!
//! Every one of those is a property of the *representation*, not of the
//! rule being checked. A parser has none of them: `#[cfg(test)]` is an
//! attribute on a node, a const is a binding, and a receiver chain is a
//! tree. Consts are collected across the whole crate in a first pass, so
//! cross-module indirection resolves too.
//!
//! Parsing removed the representation bugs. It did not, on its own,
//! fix **scope** bugs, and those turned out to be the ones that
//! mattered. Measured against the real `~/.claudepot` rather than
//! reasoned about:
//!
//! 4. **Only one crate was scanned.** Three write to the data dir —
//!    `src-tauri/src/preferences.rs` joins `preferences.json` directly,
//!    and the CLI reaches several databases the same way. Core-only
//!    passed on those because core happens to name them too.
//! 5. **Only `.json` was asked about.** `.jsonl` is the append-only log
//!    shape and two real files use it. `"x.jsonl".ends_with(".json")`
//!    is false, so neither was even a near miss — both sat on disk,
//!    undocumented, indefinitely.
//!
//! # What it deliberately does not do
//!
//! No const *expression* evaluation: `concat!`, `format!`, and a name
//! built from parts resolve to `None` and are skipped rather than
//! guessed at. This is a real limit but **not a live one** — a sweep of
//! all three crates found zero data-dir joins of that shape. It is
//! recorded as a boundary, not advertised as the next thing to fix;
//! saying so once cost a round of attention that the two scope bugs
//! above deserved.
//!
//! The blindness guards in `verify_docs` remain the backstop: a scan
//! that silently matches nothing is the failure mode this module exists
//! to make impossible.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;

/// What the data-dir-writing crates do with their files: which ones
/// they name, and whether each database they open is set up correctly.
#[derive(Debug, Default)]
pub struct DataDirFiles {
    /// Every `*.db`. Not restricted to `claudepot_data_dir()` receivers:
    /// most databases are reached through their own path helper, so
    /// requiring the anchor would lose most of the coverage. `.db` is
    /// unambiguous in this crate — the only false positives are tempdir
    /// scratch files, excluded structurally below.
    pub dbs: BTreeSet<String>,
    /// Every `*.json` joined onto a `claudepot_data_dir()` receiver.
    /// Anchored, because `.json` also names CC's `settings.json`,
    /// `~/.claude.json` and bundle entries like `manifest.json`, and a
    /// check that cries wolf is one people learn to skip.
    pub jsons: BTreeSet<String>,
    /// Production functions that call `Connection::open` **without**
    /// reaching `apply_standard_pragmas`. See [`UnpragmadOpen`].
    pub unpragmad_opens: Vec<UnpragmadOpen>,
}

/// A `Connection::open` that never reaches `apply_standard_pragmas`.
///
/// `db_pragmas` exists because `sessions.db-wal` once reached 6.3 GB,
/// and its module doc says every store "should call" the helper. That
/// was enforced by nobody: `corpus.rs` hand-rolled its own pragma batch
/// and omitted `journal_size_limit` + `wal_autocheckpoint`, which left
/// the largest database in the app — ~880 MB, bulk-insert indexer,
/// opened concurrently by GUI, CLI and the MCP subprocess — as the one
/// store outside the bound. The omission was invisible precisely
/// because the file *looked* like it configured itself properly.
///
/// A hand-rolled batch that happens to be right today is still a
/// divergence tomorrow, so the rule is the helper, not the pragmas.
#[derive(Debug)]
pub struct UnpragmadOpen {
    /// Repo-relative path of the file.
    pub file: String,
    /// The enclosing function's name.
    pub function: String,
}

/// Crates that write into the Claudepot data dir.
///
/// All three, not just core. `src-tauri/src/preferences.rs` joins
/// `preferences.json` onto the data dir directly, and the CLI reaches
/// several databases the same way. Scanning core alone reported green
/// on those only because core happens to name them too — a file added
/// solely in the Tauri or CLI crate would have been invisible, which is
/// the same "green for the wrong reason" this scan exists to prevent.
const SCANNED_CRATES: &[&str] = &[
    "crates/claudepot-core/src",
    "crates/claudepot-cli/src",
    "src-tauri/src",
];

/// Parse every `.rs` file in every crate that writes to the data dir.
pub fn scan(repo: &Path) -> Result<DataDirFiles> {
    let mut files = Vec::new();
    for rel in SCANNED_CRATES {
        let root = repo.join(rel);
        if root.is_dir() {
            collect_files(&root, &mut files)?;
        }
    }

    let parsed: Vec<syn::File> = files
        .iter()
        .map(|p| {
            let text =
                std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
            syn::parse_file(&text).with_context(|| format!("parse {}", p.display()))
        })
        .collect::<Result<_>>()?;

    // Pass 1 — every `const NAME: &str = "…"` in the crate. Crate-wide
    // rather than per-file so a const declared in one module and joined
    // in another resolves.
    let mut consts = BTreeMap::new();
    for f in &parsed {
        let mut c = ConstCollector { out: &mut consts };
        c.visit_file(f);
    }

    // Pass 2 — the joins.
    let mut v = JoinVisitor {
        consts: &consts,
        out: DataDirFiles::default(),
    };
    for f in &parsed {
        v.visit_file(f);
    }

    // Pass 3 — which whole FILES are test-only, resolved from the
    // `#[cfg(test)] mod foo;` declarations that pull them in.
    let test_only = test_only_files(&files, &parsed);

    // Pass 4 — pragma discipline, per file so the report can name one.
    for (path, f) in files.iter().zip(&parsed) {
        if test_only.contains(path) {
            continue;
        }
        let rel = path
            .strip_prefix(repo)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let mut p = PragmaVisitor {
            file: rel,
            out: &mut v.out.unpragmad_opens,
        };
        p.visit_file(f);
    }
    Ok(v.out)
}

/// Files that exist only under `cfg(test)`, because the `mod foo;` that
/// pulls them in carries the attribute.
///
/// `is_cfg_test` works on nodes *within* a file, which is enough for an
/// inline `#[cfg(test)] mod tests { … }` and blind to the other half of
/// the convention: `shared_memory/migration_tests.rs` is 700 lines of
/// pure test code whose only `#[cfg(test)]` sits on the `mod` line in
/// its parent. Judging by filename (`*_tests.rs`) would work today and
/// is precisely the kind of heuristic this module's history is a list
/// of; resolving the declaration is the same amount of code and cannot
/// drift.
///
/// Both layouts Rust allows for `mod foo;` are resolved: `foo.rs`
/// beside the declaring file, and `foo/mod.rs` below it.
fn test_only_files(files: &[PathBuf], parsed: &[syn::File]) -> BTreeSet<PathBuf> {
    struct Collector<'a> {
        dir: PathBuf,
        out: &'a mut BTreeSet<PathBuf>,
    }
    impl<'ast> Visit<'ast> for Collector<'_> {
        fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
            // `content: None` is `mod foo;` — a declaration pointing at
            // another file. An inline `mod foo { … }` is already
            // handled by the visitors' own `is_cfg_test` checks.
            if node.content.is_none() && is_cfg_test(&node.attrs) {
                let name = node.ident.to_string();
                self.out.insert(self.dir.join(format!("{name}.rs")));
                self.out.insert(self.dir.join(&name).join("mod.rs"));
            }
            syn::visit::visit_item_mod(self, node);
        }
    }

    let mut out = BTreeSet::new();
    for (path, f) in files.iter().zip(parsed) {
        let dir = match path.file_name().and_then(|n| n.to_str()) {
            // `mod.rs` and `lib.rs`/`main.rs` declare siblings of
            // themselves; any other file declares children in a
            // same-named subdirectory.
            Some("mod.rs") | Some("lib.rs") | Some("main.rs") => {
                path.parent().unwrap_or(Path::new("")).to_path_buf()
            }
            _ => path.with_extension(""),
        };
        let mut c = Collector { dir, out: &mut out };
        c.visit_file(f);
    }
    out
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// True when the node is compiled only under `cfg(test)`.
fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if !a.path().is_ident("cfg") {
            return false;
        }
        let mut found = false;
        // `parse_nested_meta` walks `cfg(test)`, `cfg(all(test, …))`,
        // and `cfg(any(test, …))` alike, so a nested form is caught too.
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("test") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

struct ConstCollector<'a> {
    out: &'a mut BTreeMap<String, String>,
}

impl<'ast> Visit<'ast> for ConstCollector<'_> {
    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        if let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) = &*node.expr
        {
            self.out.insert(node.ident.to_string(), s.value());
        }
    }
}

struct JoinVisitor<'a> {
    consts: &'a BTreeMap<String, String>,
    out: DataDirFiles,
}

impl JoinVisitor<'_> {
    /// The filename a `join` argument denotes: a string literal, or a
    /// const resolved from the crate-wide map. `None` for anything the
    /// scan will not guess at.
    fn filename(&self, arg: &syn::Expr) -> Option<String> {
        match arg {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) => Some(s.value()),
            syn::Expr::Path(p) => {
                let ident = p.path.segments.last()?.ident.to_string();
                self.consts.get(&ident).cloned()
            }
            // `&EXPR` — the borrow is noise, the inner expression is not.
            syn::Expr::Reference(r) => self.filename(&r.expr),
            _ => None,
        }
    }
}

/// Does this receiver expression contain a `claudepot_data_dir()` call?
fn mentions_data_dir(expr: &syn::Expr) -> bool {
    struct Finder(bool);
    impl<'ast> Visit<'ast> for Finder {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = &*node.func {
                if p.path
                    .segments
                    .last()
                    .is_some_and(|s| s.ident == "claudepot_data_dir")
                {
                    self.0 = true;
                }
            }
            syn::visit::visit_expr_call(self, node);
        }
    }
    let mut f = Finder(false);
    f.visit_expr(expr);
    f.0
}

/// JSON-shaped state, which on disk means `.json` *and* `.jsonl`.
///
/// `.jsonl` is not a variant spelling — it is the append-only log
/// shape, and two real files use it (`cc_tips_snapshots.jsonl`,
/// `doctor-parse-failures.jsonl`). Both sat undocumented because the
/// scan only asked about `.json`, and `"x.jsonl".ends_with(".json")`
/// is false, so neither was ever a near miss.
fn is_json_state(name: &str) -> bool {
    name.ends_with(".json") || name.ends_with(".jsonl")
}

/// Is the receiver a scratch location rather than the data dir?
///
/// Two shapes, both meaning "explicitly not where we keep state":
///
/// - `dir.path()` — a `TempDir` handle. Test scratch files (`test.db`,
///   `a.db`) are all rooted this way.
/// - `std::env::temp_dir()` — the system temp dir.
///   `src-tauri/src/lib.rs:399` uses it for a
///   `claudepot-memory_changes.db` fallback, which the unanchored `.db`
///   match reported as a data-dir database until this covered it.
///
/// Structural, where the old scan matched the literal text `.path()`.
/// This exclusion is the cost of leaving `.db` unanchored; it is worth
/// paying because anchoring `.db` would lose the databases reached
/// through their own path helpers, which is most of them.
fn is_scratch_receiver(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::MethodCall(m) => m.method == "path",
        syn::Expr::Call(c) => match &*c.func {
            syn::Expr::Path(p) => p
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "temp_dir"),
            _ => false,
        },
        syn::Expr::Reference(r) => is_scratch_receiver(&r.expr),
        _ => false,
    }
}

impl<'ast> Visit<'ast> for JoinVisitor<'_> {
    // Skipping `cfg(test)` nodes structurally is what removes the
    // truncation heuristic entirely: a test module, a `#[cfg(test)]`
    // helper fn, and a `#[cfg(test)]` method are all just nodes with
    // that attribute, wherever they sit in the file.
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if is_cfg_test(&node.attrs) {
            return;
        }
        syn::visit::visit_item_mod(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if is_cfg_test(&node.attrs) {
            return;
        }
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if is_cfg_test(&node.attrs) {
            return;
        }
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "join" && node.args.len() == 1 {
            if let Some(name) = self.filename(&node.args[0]) {
                if name.ends_with(".db") {
                    if !is_scratch_receiver(&node.receiver) {
                        self.out.dbs.insert(name);
                    }
                } else if is_json_state(&name) && mentions_data_dir(&node.receiver) {
                    self.out.jsons.insert(name);
                }
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

// ─── Pragma discipline ───────────────────────────────────────────────

/// Does this block contain a call whose path ends in `tail`?
///
/// Tail matching on the last N segments, so `Connection::open` and
/// `rusqlite::Connection::open` resolve alike, and
/// `apply_standard_pragmas` matches with or without its
/// `crate::db_pragmas::` prefix.
///
/// The `Connection` segment is load-bearing: matching a bare `open`
/// would sweep in `File::open`, `OpenOptions::…::open` and every other
/// unrelated opener in three crates, and a gate that cries wolf is one
/// people learn to pass with an `#[allow]`.
fn calls_path_ending_in(block: &syn::Block, tail: &[&str]) -> bool {
    struct Finder<'a>(&'a [&'a str], bool);
    impl<'ast> Visit<'ast> for Finder<'_> {
        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = &*node.func {
                let segs: Vec<String> = p
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();
                if segs.len() >= self.0.len()
                    && segs[segs.len() - self.0.len()..]
                        .iter()
                        .zip(self.0)
                        .all(|(a, b)| a == b)
                {
                    self.1 = true;
                }
            }
            syn::visit::visit_expr_call(self, node);
        }
    }
    let mut v = Finder(tail, false);
    v.visit_block(block);
    v.1
}

/// Flags production functions that open a SQLite connection without
/// routing it through `db_pragmas::apply_standard_pragmas`.
///
/// Two open shapes are deliberately NOT flagged, because neither wants
/// the standard set:
///
/// - `open_in_memory` — WAL silently downgrades to `memory` journal
///   mode on a connection with no file behind it, so the pragmas are
///   meaningless rather than missing.
/// - `open_with_flags` — `db_housekeeping::checkpoint_one` uses it
///   precisely to avoid `SQLITE_OPEN_CREATE`, and sets its own short
///   busy timeout because backing off fast is the point there.
///
/// Both are matched on the method *name*, so this is a structural
/// exemption rather than an allowlist that can rot.
struct PragmaVisitor<'a> {
    file: String,
    out: &'a mut Vec<UnpragmadOpen>,
}

impl PragmaVisitor<'_> {
    fn check(&mut self, name: String, attrs: &[syn::Attribute], block: &syn::Block) -> bool {
        if is_cfg_test(attrs) {
            return false;
        }
        if calls_path_ending_in(block, &["Connection", "open"])
            && !calls_path_ending_in(block, &["apply_standard_pragmas"])
        {
            self.out.push(UnpragmadOpen {
                file: self.file.clone(),
                function: name,
            });
        }
        true
    }
}

impl<'ast> Visit<'ast> for PragmaVisitor<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if is_cfg_test(&node.attrs) {
            return;
        }
        syn::visit::visit_item_mod(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if !self.check(node.sig.ident.to_string(), &node.attrs, &node.block) {
            return;
        }
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if !self.check(node.sig.ident.to_string(), &node.attrs, &node.block) {
            return;
        }
        syn::visit::visit_impl_item_fn(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_src(src: &str) -> DataDirFiles {
        let f = syn::parse_file(src).expect("parse");
        let mut consts = BTreeMap::new();
        ConstCollector { out: &mut consts }.visit_file(&f);
        let mut v = JoinVisitor {
            consts: &consts,
            out: DataDirFiles::default(),
        };
        v.visit_file(&f);
        v.out
    }

    /// Bug 2: the filename is a const, not a literal.
    #[test]
    fn resolves_a_const_filename() {
        let out = scan_src(
            r#"
            pub const BOARDS_DB_FILENAME: &str = "boards.db";
            pub fn boards_db_path() -> PathBuf {
                claudepot_data_dir().join(BOARDS_DB_FILENAME)
            }
            "#,
        );
        assert!(out.dbs.contains("boards.db"));
    }

    /// Bug 3: the const lives in another module. One crate-wide map, so
    /// declaration and use need not share a file.
    #[test]
    fn resolves_a_const_declared_in_another_file() {
        let decl =
            syn::parse_file("pub const CACHE_FILENAME: &str = \"pricing-cache.json\";").unwrap();
        let usage =
            syn::parse_file("fn p() -> PathBuf { claudepot_data_dir().join(CACHE_FILENAME) }")
                .unwrap();
        let mut consts = BTreeMap::new();
        ConstCollector { out: &mut consts }.visit_file(&decl);
        let mut v = JoinVisitor {
            consts: &consts,
            out: DataDirFiles::default(),
        };
        v.visit_file(&usage);
        assert!(v.out.jsons.contains("pricing-cache.json"));
    }

    /// Bug 1: a `#[cfg(test)]` method must not truncate the production
    /// code that follows it in the same file.
    #[test]
    fn production_after_a_cfg_test_method_is_still_scanned() {
        let out = scan_src(
            r#"
            impl S {
                #[cfg(test)]
                fn helper() -> PathBuf { claudepot_data_dir().join("scratch.json") }
            }
            fn real() -> PathBuf { claudepot_data_dir().join("agents.json") }
            "#,
        );
        assert!(
            out.jsons.contains("agents.json"),
            "production code after a cfg(test) method must be seen"
        );
        assert!(
            !out.jsons.contains("scratch.json"),
            "the cfg(test) method must be skipped"
        );
    }

    #[test]
    fn test_modules_are_skipped_wherever_they_sit() {
        let out = scan_src(
            r#"
            #[cfg(test)]
            mod tests {
                fn t() -> PathBuf { dir.path().join("test.db") }
            }
            fn real() -> PathBuf { claudepot_data_dir().join("corpus.db") }
            "#,
        );
        assert!(out.dbs.contains("corpus.db"));
        assert!(!out.dbs.contains("test.db"));
    }

    /// Tempdir scratch files are excluded by receiver shape, not by
    /// matching the text `.path()`.
    #[test]
    fn scratch_rooted_databases_are_excluded() {
        let out = scan_src(r#"fn t() -> PathBuf { dir.path().join("a.db") }"#);
        assert!(out.dbs.is_empty());
    }

    /// `.json` is anchored; `.db` is not. CC's own files must not be
    /// swept into the data-dir list.
    #[test]
    fn json_requires_the_data_dir_anchor() {
        let out = scan_src(
            r#"
            fn cc() -> PathBuf { config_dir.join("settings.json") }
            fn bundle() -> PathBuf { staged.join("manifest.json") }
            fn ours() -> PathBuf { claudepot_data_dir().join("routes.json") }
            "#,
        );
        assert_eq!(
            out.jsons.into_iter().collect::<Vec<_>>(),
            vec!["routes.json".to_string()]
        );
    }

    /// Scope bug 5: `.jsonl` is a real data-dir shape, and it is not a
    /// suffix of `.json`, so it was never even a near miss.
    #[test]
    fn jsonl_counts_as_json_state() {
        let out = scan_src(
            r#"fn t() -> PathBuf { claudepot_data_dir().join("doctor-parse-failures.jsonl") }"#,
        );
        assert!(out.jsons.contains("doctor-parse-failures.jsonl"));
    }

    /// `std::env::temp_dir()` is a scratch location, not the data dir.
    /// The unanchored `.db` match reported `src-tauri`'s fallback
    /// database until the exclusion covered this shape too.
    #[test]
    fn system_temp_dir_databases_are_excluded() {
        let out = scan_src(
            r#"fn t() -> PathBuf { std::env::temp_dir().join("claudepot-memory_changes.db") }"#,
        );
        assert!(out.dbs.is_empty());
    }

    /// A name the scan cannot resolve is skipped, never guessed at.
    #[test]
    fn unresolvable_names_are_skipped() {
        let out = scan_src(r#"fn t(name: &str) -> PathBuf { claudepot_data_dir().join(name) }"#);
        assert!(out.jsons.is_empty() && out.dbs.is_empty());
    }

    // ─── Pragma discipline ───────────────────────────────────────────

    fn pragma_scan(src: &str) -> Vec<UnpragmadOpen> {
        let f = syn::parse_file(src).expect("parse");
        let mut out = Vec::new();
        PragmaVisitor {
            file: "x.rs".into(),
            out: &mut out,
        }
        .visit_file(&f);
        out
    }

    /// The `corpus.rs` shape: a real store that configures itself by
    /// hand. It looks deliberate, which is exactly why nobody noticed
    /// it had dropped two of the four pragmas.
    #[test]
    fn hand_rolled_pragmas_do_not_satisfy_the_rule() {
        let out = pragma_scan(
            r#"
            impl S {
                pub fn open(path: &Path) -> Result<Self> {
                    let conn = Connection::open(path)?;
                    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
                    Ok(Self { conn })
                }
            }
            "#,
        );
        assert_eq!(out.len(), 1, "hand-rolled batch must still be flagged");
        assert_eq!(out[0].function, "open");
    }

    #[test]
    fn calling_the_helper_satisfies_the_rule() {
        for call in [
            "apply_standard_pragmas(&db)?;",
            "crate::db_pragmas::apply_standard_pragmas(&db)?;",
        ] {
            let out = pragma_scan(&format!(
                "impl S {{ pub fn open(p: &Path) -> R {{ let db = Connection::open(p)?; {call} Ok(()) }} }}"
            ));
            assert!(out.is_empty(), "should accept `{call}`");
        }
    }

    /// `rusqlite::Connection::open` is the same call with a prefix.
    #[test]
    fn a_qualified_connection_path_is_still_an_open() {
        let out =
            pragma_scan("fn f(p: &Path) -> R { let c = rusqlite::Connection::open(p)?; Ok(()) }");
        assert_eq!(out.len(), 1);
    }

    /// The two exempt shapes, and the reason each is exempt: WAL is
    /// meaningless in memory, and `checkpoint_one` wants its own short
    /// busy timeout so it can back off fast.
    #[test]
    fn in_memory_and_flagged_opens_are_exempt() {
        let out = pragma_scan(
            r#"
            fn a() -> R { let c = Connection::open_in_memory()?; Ok(()) }
            fn b(p: &Path) -> R { let c = Connection::open_with_flags(p, flags)?; Ok(()) }
            "#,
        );
        assert!(out.is_empty(), "got {out:?}");
    }

    /// Unrelated openers must not be swept in — the check keys on
    /// `Connection`, not on the bare method name.
    #[test]
    fn other_open_calls_are_not_database_opens() {
        let out = pragma_scan(
            r#"
            fn a(p: &Path) -> R { let f = File::open(p)?; Ok(()) }
            fn b(p: &Path) -> R { let f = std::fs::File::open(p)?; Ok(()) }
            "#,
        );
        assert!(out.is_empty(), "got {out:?}");
    }

    /// Test fixtures open connections constantly and correctly do not
    /// pragma them; flagging those would train people to ignore this.
    #[test]
    fn test_code_is_not_held_to_the_rule() {
        let out = pragma_scan(
            r#"
            #[cfg(test)]
            mod tests {
                fn fixture(p: &Path) { let c = Connection::open(p).unwrap(); }
            }
            #[cfg(test)]
            fn helper(p: &Path) { let c = Connection::open(p).unwrap(); }
            "#,
        );
        assert!(out.is_empty(), "got {out:?}");
    }

    /// `shared_memory/migration_tests.rs` in miniature: a whole file of
    /// test code whose only `#[cfg(test)]` is on the parent's `mod`
    /// line. Both file layouts Rust permits must resolve.
    #[test]
    fn a_cfg_test_mod_declaration_marks_the_whole_file() {
        let decl =
            syn::parse_file("pub mod real;\n#[cfg(test)]\npub mod migration_tests;\n").unwrap();
        let dir = PathBuf::from("/repo/src/shared_memory");
        let out = test_only_files(&[dir.join("mod.rs")], &[decl]);

        assert!(out.contains(&dir.join("migration_tests.rs")));
        assert!(out.contains(&dir.join("migration_tests").join("mod.rs")));
        assert!(
            !out.contains(&dir.join("real.rs")),
            "a plain `mod` declaration must not be marked test-only"
        );
    }

    /// A non-`mod.rs` file declares its children in a same-named
    /// subdirectory, not beside itself.
    #[test]
    fn child_modules_of_a_leaf_file_resolve_into_its_subdirectory() {
        let decl = syn::parse_file("#[cfg(test)]\nmod swap_tests;\n").unwrap();
        let out = test_only_files(&[PathBuf::from("/repo/src/desktop_backend.rs")], &[decl]);
        assert!(out.contains(&PathBuf::from("/repo/src/desktop_backend/swap_tests.rs")));
    }

    /// A wrapped chain is a tree, so line breaks are irrelevant — the
    /// window heuristic the string scan needed is simply gone.
    #[test]
    fn wrapping_is_irrelevant_to_a_parser() {
        let out = scan_src(
            "fn t() -> PathBuf {\n    crate::paths::claudepot_data_dir()\n        .join(\"usage_alert_state.json\")\n}",
        );
        assert!(out.jsons.contains("usage_alert_state.json"));
    }
}

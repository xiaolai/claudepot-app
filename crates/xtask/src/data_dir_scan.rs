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
//! # What it deliberately does not do
//!
//! No const *expression* evaluation: `concat!`, `format!`, and a name
//! built from parts all resolve to `None` and are skipped rather than
//! guessed at. If that ever hides a real file, the blindness guards in
//! `verify_docs` fail loudly rather than passing green — a scan that
//! silently matches nothing is the failure mode this module exists to
//! make impossible.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;

/// Filenames the crate joins onto its data dir.
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
}

/// Parse every `.rs` file under `core_src` and report what it writes.
pub fn scan(core_src: &Path) -> Result<DataDirFiles> {
    let mut files = Vec::new();
    collect_files(core_src, &mut files)?;

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
    Ok(v.out)
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

/// Is the receiver a tempdir handle — `dir.path()`, `td.path()`?
///
/// Structural, where the old scan matched the literal text `.path()`.
/// Test scratch files (`test.db`, `a.db`) are all rooted this way, and
/// they are the one shape a loose `.db` match must exclude.
fn is_tempdir_receiver(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::MethodCall(m) => m.method == "path",
        syn::Expr::Reference(r) => is_tempdir_receiver(&r.expr),
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
                    if !is_tempdir_receiver(&node.receiver) {
                        self.out.dbs.insert(name);
                    }
                } else if name.ends_with(".json") && mentions_data_dir(&node.receiver) {
                    self.out.jsons.insert(name);
                }
            }
        }
        syn::visit::visit_expr_method_call(self, node);
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
    fn tempdir_rooted_databases_are_excluded() {
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

    /// A name the scan cannot resolve is skipped, never guessed at.
    #[test]
    fn unresolvable_names_are_skipped() {
        let out = scan_src(r#"fn t(name: &str) -> PathBuf { claudepot_data_dir().join(name) }"#);
        assert!(out.jsons.is_empty() && out.dbs.is_empty());
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

//! Rust-side string catalog for the GUI's native surfaces — app menu,
//! tray, quit dialog, and the four OS-banner modules (i18n plan §2.4).
//!
//! Hand-rolled on purpose: ~120 strings and zh-CN has trivial plural
//! rules, so a fluent/rust-i18n dependency buys nothing here. The
//! webview has its own i18next instance over `src/locales/`; the two
//! catalogs are separate because the surfaces never share strings.
//!
//! Contract:
//! - Catalogs are embedded (`include_str!`) flat `"dotted.key" → value`
//!   maps. English is canonical; a zh-CN miss falls back to en; an
//!   unknown key returns the key itself. Never panics.
//! - The active locale is a process global. It defaults to `En`, which
//!   is what keeps every existing English-string unit test in this
//!   crate byte-identical — **tests must never call [`set_locale`]**;
//!   per-locale assertions go through the `*_in` internals instead.
//! - Plurals use flat `key_one` / `key_other` entries. English selects
//!   `_one` at n == 1; zh-CN always selects `_other` (its catalog
//!   carries `_one` duplicates only so the completeness test can
//!   demand identical key sets).
//! - Log/tracing strings stay English and never route through here.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

const EN_JSON: &str = include_str!("../i18n/en.json");
const ZH_CN_JSON: &str = include_str!("../i18n/zh-CN.json");

static EN: LazyLock<HashMap<String, String>> = LazyLock::new(|| parse_catalog(EN_JSON, "en"));
static ZH_CN: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| parse_catalog(ZH_CN_JSON, "zh-CN"));

/// Active locale. `En` at rest so pre-boot code paths (and the test
/// suite, which shares this global across threads) render canonical
/// English until `setup()` applies the persisted preference.
static LOCALE: RwLock<Locale> = RwLock::new(Locale::En);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Locale {
    En,
    ZhCn,
}

impl Locale {
    fn tag(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::ZhCn => "zh-CN",
        }
    }
}

/// The catalogs are compile-time constants, so a parse failure is a
/// build defect the unit tests catch; degrading to an empty map (every
/// `tr` returns its key) is still preferable to panicking a release
/// binary that somehow shipped one.
fn parse_catalog(json: &str, tag: &str) -> HashMap<String, String> {
    match serde_json::from_str::<HashMap<String, String>>(json) {
        Ok(map) => map,
        Err(e) => {
            tracing::error!(locale = tag, error = %e, "i18n catalog parse failed; keys will render raw");
            HashMap::new()
        }
    }
}

/// Resolve a persisted preference (`None` = follow the OS) against an
/// OS-language hint. Mirrors `resolveLocale` in `src/lib/i18n.ts`:
/// any `zh*` tag maps to zh-CN, everything else to en.
fn resolve(pref: Option<&str>, os_language: Option<&str>) -> Locale {
    let tag = pref.or(os_language).unwrap_or("en");
    // `get(..2)` rather than a slice: a tag starting with a multi-byte
    // char must not panic on the byte boundary.
    match tag.get(..2) {
        Some(p) if p.eq_ignore_ascii_case("zh") => Locale::ZhCn,
        _ => Locale::En,
    }
}

/// Apply a locale preference to the process global. `None` follows the
/// OS language via `sys-locale` (CFLocale on macOS — env-var sources
/// like `LANG` are empty for Dock-launched GUI processes, which is why
/// `whoami`'s env-based language surface was not reused here).
///
/// Called from `setup()` before the app menu and tray are built, and
/// from `preferences_set_locale` after a successful persist.
pub fn set_locale(pref: Option<&str>) {
    let os = sys_locale::get_locale();
    let next = resolve(pref, os.as_deref());
    match LOCALE.write() {
        Ok(mut guard) => *guard = next,
        // A poisoned lock only means a panic elsewhere; the stored
        // value is a Copy enum, always valid — recover and write.
        Err(poisoned) => *poisoned.into_inner() = next,
    }
}

/// BCP-47 tag of the active locale (`"en"` | `"zh-CN"`).
pub fn current_locale() -> &'static str {
    active().tag()
}

fn active() -> Locale {
    match LOCALE.read() {
        Ok(guard) => *guard,
        Err(poisoned) => *poisoned.into_inner(),
    }
}

/// Look up `key` in `primary`, then in the en fallback when given one.
fn lookup_from<'a>(
    primary: &'a HashMap<String, String>,
    fallback: Option<&'a HashMap<String, String>>,
    key: &str,
) -> Option<&'a str> {
    primary
        .get(key)
        .or_else(|| fallback.and_then(|f| f.get(key)))
        .map(String::as_str)
}

fn lookup(locale: Locale, key: &str) -> Option<&'static str> {
    match locale {
        Locale::En => lookup_from(&EN, None, key),
        Locale::ZhCn => lookup_from(&ZH_CN, Some(&EN), key),
    }
}

/// Translate `key` in the active locale. Unknown key → the key itself.
pub fn tr(key: &str) -> String {
    tr_in(active(), key)
}

fn tr_in(locale: Locale, key: &str) -> String {
    lookup(locale, key)
        .map(str::to_string)
        .unwrap_or_else(|| key.to_string())
}

/// Translate with one `{name}` placeholder substituted.
pub fn tr1(key: &str, name: &str, value: &str) -> String {
    tr_args(key, &[(name, value)])
}

/// Translate with every listed `{name}` placeholder substituted.
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> String {
    interpolate(&tr(key), args)
}

/// Plural-aware translate: selects `key_one` / `key_other` per the
/// active locale's rules and substitutes `{n}`.
pub fn tr_n(key: &str, n: u64) -> String {
    tr_n_args(key, n, &[])
}

/// [`tr_n`] with extra placeholders beyond `{n}`.
pub fn tr_n_args(key: &str, n: u64, args: &[(&str, &str)]) -> String {
    let s = tr_n_in(active(), key, n);
    interpolate(&interpolate(&s, &[("n", &n.to_string())]), args)
}

fn tr_n_in(locale: Locale, key: &str, n: u64) -> String {
    let suffix = match locale {
        // zh-CN has no singular/plural split.
        Locale::ZhCn => "_other",
        Locale::En => {
            if n == 1 {
                "_one"
            } else {
                "_other"
            }
        }
    };
    let full = format!("{key}{suffix}");
    lookup(locale, &full).map(str::to_string).unwrap_or(full)
}

fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    // NOTE: no test here (or anywhere in this crate) may call
    // `set_locale` — the global is shared across the parallel test
    // runner, and every English-output assertion in tray.rs /
    // tray_menu.rs / the watcher modules depends on the `En` default.

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn catalogs_parse_nonempty() {
        assert!(!EN.is_empty(), "en catalog failed to parse");
        assert!(!ZH_CN.is_empty(), "zh-CN catalog failed to parse");
    }

    #[test]
    fn catalogs_have_identical_key_sets() {
        let en: BTreeSet<&String> = EN.keys().collect();
        let zh: BTreeSet<&String> = ZH_CN.keys().collect();
        let missing_in_zh: Vec<_> = en.difference(&zh).collect();
        let missing_in_en: Vec<_> = zh.difference(&en).collect();
        assert!(
            missing_in_zh.is_empty() && missing_in_en.is_empty(),
            "catalog drift — missing in zh-CN: {missing_in_zh:?}; missing in en: {missing_in_en:?}"
        );
    }

    #[test]
    fn catalogs_agree_on_placeholders() {
        // A translation that drops or renames a `{placeholder}` ships a
        // literal brace token to the user; catch it at test time.
        fn placeholders(s: &str) -> BTreeSet<String> {
            let mut out = BTreeSet::new();
            let mut rest = s;
            while let Some(start) = rest.find('{') {
                let Some(len) = rest[start + 1..].find('}') else {
                    break;
                };
                out.insert(rest[start + 1..start + 1 + len].to_string());
                rest = &rest[start + 1 + len..];
            }
            out
        }
        for (key, en_value) in EN.iter() {
            if let Some(zh_value) = ZH_CN.get(key) {
                assert_eq!(
                    placeholders(en_value),
                    placeholders(zh_value),
                    "placeholder drift on {key:?}"
                );
            }
        }
    }

    #[test]
    fn zh_missing_key_falls_back_to_en() {
        let en = map(&[("a.b", "hello"), ("a.c", "shared")]);
        let zh = map(&[("a.c", "共享")]);
        assert_eq!(lookup_from(&zh, Some(&en), "a.b"), Some("hello"));
        assert_eq!(lookup_from(&zh, Some(&en), "a.c"), Some("共享"));
        assert_eq!(lookup_from(&zh, Some(&en), "a.d"), None);
    }

    #[test]
    fn unknown_key_returns_key_itself() {
        assert_eq!(tr_in(Locale::En, "no.such.key"), "no.such.key");
        assert_eq!(tr_in(Locale::ZhCn, "no.such.key"), "no.such.key");
    }

    #[test]
    fn en_plural_selects_one_and_other() {
        assert_eq!(
            tr_n_in(Locale::En, "tray.alertSuffix", 1),
            "{n} alerting session"
        );
        assert_eq!(
            tr_n_in(Locale::En, "tray.alertSuffix", 2),
            "{n} alerting sessions"
        );
        // 0 is plural in English.
        assert_eq!(
            tr_n_in(Locale::En, "tray.alertSuffix", 0),
            "{n} alerting sessions"
        );
    }

    #[test]
    fn zh_plural_always_selects_other() {
        assert_eq!(
            tr_n_in(Locale::ZhCn, "tray.healthWarnCount", 1),
            "健康：{n} 项警告"
        );
        assert_eq!(
            tr_n_in(Locale::ZhCn, "tray.healthWarnCount", 5),
            "健康：{n} 项警告"
        );
    }

    #[test]
    fn plural_unknown_key_returns_suffixed_key() {
        assert_eq!(tr_n_in(Locale::En, "no.such", 1), "no.such_one");
        assert_eq!(tr_n_in(Locale::ZhCn, "no.such", 1), "no.such_other");
    }

    #[test]
    fn interpolate_substitutes_all_named_args() {
        assert_eq!(
            interpolate(
                "Moving session {from} → {to}",
                &[("from", "a"), ("to", "b")]
            ),
            "Moving session a → b"
        );
        // Unknown placeholder is left intact — visible, not a panic.
        assert_eq!(interpolate("{x} stays", &[("y", "1")]), "{x} stays");
    }

    #[test]
    fn resolve_pref_beats_os() {
        assert_eq!(resolve(Some("en"), Some("zh-CN")), Locale::En);
        assert_eq!(resolve(Some("zh-CN"), Some("en-US")), Locale::ZhCn);
    }

    #[test]
    fn resolve_follows_os_when_no_pref() {
        assert_eq!(resolve(None, Some("zh-Hans-CN")), Locale::ZhCn);
        assert_eq!(resolve(None, Some("ZH-TW")), Locale::ZhCn);
        assert_eq!(resolve(None, Some("en-GB")), Locale::En);
        assert_eq!(resolve(None, Some("fr-FR")), Locale::En);
        assert_eq!(resolve(None, None), Locale::En);
    }

    #[test]
    fn resolve_multibyte_tag_does_not_panic() {
        assert_eq!(resolve(None, Some("中文")), Locale::En);
    }

    #[test]
    fn default_locale_is_en_and_tr_matches_source_strings() {
        // Spot-check that the en catalog is byte-identical to the
        // strings the menus shipped before extraction.
        assert_eq!(current_locale(), "en");
        assert_eq!(tr("menu.about"), "About Claudepot");
        assert_eq!(tr("tray.noAccountsActive"), "No accounts active");
        assert_eq!(
            tr1("tray.usageNoData", "email", "a@x.com"),
            "a@x.com — (no data — click Refresh)"
        );
        assert_eq!(tr_n("quit.opsInProgress", 1), "1 operation in progress");
        assert_eq!(tr_n("quit.opsInProgress", 3), "3 operations in progress");
    }
}

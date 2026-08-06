import { afterEach, describe, expect, it } from "vitest";
import {
  applyLocalePreference,
  getActiveLocale,
  i18n,
  resolveLocale,
  SUPPORTED_LOCALES,
} from "./i18n";
import { LOCALE_KEY } from "./storageKeys";

afterEach(async () => {
  // Tests share the module-global instance — restore the en default so
  // ordering can't leak zh-CN into another file's assertions.
  await applyLocalePreference(null);
});

describe("resolveLocale", () => {
  it("honors an explicit supported preference", () => {
    expect(resolveLocale("zh-CN")).toBe("zh-CN");
    expect(resolveLocale("en")).toBe("en");
  });

  it("falls back to the navigator language for null/unknown", () => {
    // jsdom reports en-US.
    expect(resolveLocale(null)).toBe("en");
    expect(resolveLocale(undefined)).toBe("en");
    expect(resolveLocale("fr")).toBe("en");
  });
});

describe("applyLocalePreference", () => {
  it("switches the live instance and mirrors the preference", async () => {
    await applyLocalePreference("zh-CN");
    expect(getActiveLocale()).toBe("zh-CN");
    expect(localStorage.getItem(LOCALE_KEY)).toBe("zh-CN");

    await applyLocalePreference(null);
    expect(getActiveLocale()).toBe("en");
    expect(localStorage.getItem(LOCALE_KEY)).toBeNull();
  });
});

describe("shell catalog — the P0 proving slice", () => {
  it("renders English plurals", () => {
    const t = i18n.getFixedT("en", "shell");
    expect(t("statusbar.projects", { count: 1 })).toBe("1 project");
    expect(t("statusbar.projects", { count: 3 })).toBe("3 projects");
  });

  it("renders zh-CN counts and section labels", async () => {
    await applyLocalePreference("zh-CN");
    const t = i18n.getFixedT("zh-CN", "shell");
    expect(t("statusbar.projects", { count: 3 })).toBe("3 个项目");
    expect(t("sections.accounts")).toBe("账户");
    expect(t("sidebar.synced")).toBe("已同步");
  });

  it("falls back to English for a key missing from zh-CN", async () => {
    await applyLocalePreference("zh-CN");
    // fallbackLng: "en" — a missing zh key must render English, never
    // the raw key. (No key is intentionally missing today; simulate.)
    // The cast exists because typed t() rightly rejects a key that is
    // not in the catalog — which is exactly what this probe is.
    i18n.addResource("en", "shell", "___fallbackProbe", "probe");
    const tRaw = i18n.t as unknown as (k: string, o?: object) => string;
    expect(tRaw("___fallbackProbe", { ns: "shell" })).toBe("probe");
  });
});

// The supported-locale list is hand-written twice: `SUPPORTED_LOCALES`
// here and a `SUPPORTED` array in `preferences_set_locale`. A comment
// in each says "keep them in sync", which is the weakest possible
// mechanism — the first added locale drifts them, and the symptom is a
// picker offering a language the backend silently rejects. Two lists
// with no check between them is the pattern this repo's section
// registry exists to prevent; lock them instead.
describe("locale allowlist parity with Rust", () => {
  it("SUPPORTED_LOCALES matches preferences_set_locale's allowlist", async () => {
    const src = (
      await import("../../src-tauri/src/commands/preferences.rs?raw")
    ).default as string;
    const block = src.match(/const SUPPORTED:\s*\[&str;\s*\d+\]\s*=\s*\[([^\]]*)\]/);
    expect(block, "could not find the Rust allowlist — the check must not silently pass").toBeTruthy();
    const rust = [...block![1].matchAll(/"([^"]+)"/g)].map((m) => m[1]).sort();
    expect(rust).toEqual([...SUPPORTED_LOCALES].sort());
  });
});

// A type-to-confirm token is the one string in a destructive dialog the
// user must reproduce character for character. The general rule is that
// values a user types stay English (paths, commands, setting keys) —
// a confirmation token is the documented exception, because the friction
// only works if the word was read and understood. `ABANDON` was a
// hardcoded English literal inside an otherwise fully-translated dialog,
// so a zh reader was asked to type a word the dialog never showed them.
describe("type-to-confirm tokens are localized", () => {
  const GATES = [
    { ns: "settings", key: "retention.confirmDisable.phrase" },
    { ns: "projects", key: "repair.abandonPhrase" },
  ] as const;

  it("every gate token exists in both locales", () => {
    for (const { ns, key } of GATES) {
      for (const loc of ["en", "zh-CN"] as const) {
        const v = i18n.getFixedT(loc, ns)(key);
        expect(v, `${loc} ${ns}.${key}`).toBeTruthy();
        expect(v, `${loc} ${ns}.${key} is a raw key`).not.toContain(key);
      }
    }
  });

  it("the zh token differs from the en token where the word is prose", () => {
    // Not a blanket rule: a token may legitimately be identical across
    // locales. But the retention phrase is a sentence and the abandon
    // token is a verb the dialog itself translates, so both must differ
    // — if they match, the gate is showing English inside a zh dialog.
    for (const { ns, key } of GATES) {
      const en = i18n.getFixedT("en", ns)(key);
      const zh = i18n.getFixedT("zh-CN", ns)(key);
      expect(zh, `${ns}.${key} was left untranslated`).not.toBe(en);
    }
  });

  it("the instruction that frames the token is localized too", () => {
    // Typing the right word is useless if the sentence asking for it is
    // in another language.
    const zh = i18n.getFixedT("zh-CN", "components")("modals.typeToConfirm", {
      token: "X",
    });
    expect(zh).not.toContain("Type ");
    expect(zh).toContain("确认");
  });
});

// Claudepot has two distinct trashes, and English separates them only by
// capitalisation: lowercase "trash" is Claudepot's own store under
// `~/.claudepot/trash/`, restorable in Settings → Cleanup; capital "Trash"
// is the OS Trash reached via the `trash` crate (session/move_), restorable
// in Finder. Chinese has no capitalisation to lean on, so the two need two
// words — 回收站 for ours, 废纸篓 for the OS one.
//
// They had drifted: Settings called our artifact trash 废纸篓 while Config,
// Sessions and the error catalog called the same store 回收站. That broke a
// pointer — Config said "移入回收站…可在设置 → 清理中恢复" and the pane it
// named was titled 工件废纸篓 — and it collided with adopt's 废纸篓, which
// really is the OS Trash and has a different recovery procedure entirely.
//
// No structural gate can catch this: every key existed, placeholders and
// tags matched, and both words are real Chinese. Only the mapping was wrong.
describe("trash terminology maps to the right store", () => {
  // The only surfaces that mean the OS Trash. `projects.adopt.*` moves an
  // orphan slug dir via session::move_, which calls trash::delete;
  // `errors.session_move.trash_failed` is that call's failure.
  const OS_TRASH_KEYS = [/^adopt\./, /^session_move\.trash_failed$/];

  const flatten = (obj: unknown, prefix = ""): [string, string][] => {
    if (typeof obj === "string") return [[prefix, obj]];
    if (!obj || typeof obj !== "object") return [];
    return Object.entries(obj as Record<string, unknown>).flatMap(([k, v]) =>
      flatten(v, prefix ? `${prefix}.${k}` : k),
    );
  };

  const zhEntries = (): { ns: string; key: string; value: string }[] => {
    const namespaces = (i18n.options.ns ?? []) as string[];
    return namespaces.flatMap((ns) =>
      flatten(i18n.getResourceBundle("zh-CN", ns)).map(([key, value]) => ({
        ns,
        key,
        value,
      })),
    );
  };

  it("废纸篓 appears only where the OS Trash is actually meant", () => {
    const stray = zhEntries()
      .filter((e) => e.value.includes("废纸篓"))
      .filter((e) => !OS_TRASH_KEYS.some((re) => re.test(e.key)));
    expect(
      stray.map((e) => `${e.ns}:${e.key}`),
      "these say 废纸篓 (OS Trash) but back onto Claudepot's own store",
    ).toEqual([]);
  });

  it("every OS-trash surface says 废纸篓, not 回收站", () => {
    const wrong = zhEntries()
      .filter((e) => OS_TRASH_KEYS.some((re) => re.test(e.key)))
      .filter((e) => e.value.includes("回收站"));
    expect(
      wrong.map((e) => `${e.ns}:${e.key}`),
      "these reach the OS Trash but name Claudepot's store",
    ).toEqual([]);
  });

  it("the Config pointer names the pane Settings actually renders", () => {
    // The whole failure mode in one assertion: follow the cross-reference.
    const pointer = i18n.getFixedT("zh-CN", "config")("lifecycle.trashTitle");
    const destination = i18n.getFixedT("zh-CN", "settings")("trash.title");
    const term = "回收站";
    expect(pointer).toContain(term);
    expect(destination).toContain(term);
  });
});

// The zh catalogs use two quote marks with different jobs: 「」 names a UI
// affordance the reader can go and click (「刷新」, 「审阅」, 「设置 → 清理」),
// and “” quotes data — a value they typed, a name they chose, a mode that
// failed to parse. That split is worth keeping because it tells the reader
// which words are navigation and which are their own content.
//
// Thirteen strings had it backwards, wrapping an interpolated value in the
// brackets that mean "button" — so a user's artifact name rendered as though
// it were something to click. Same drift shape as the trash terms: one
// feature, two surfaces, two answers. Config said 已把 {{kind}}「{{name}}」移入回收站
// where Settings said 已把 {{kind}}“{{name}}”移入回收站, from one English source.
describe("zh quote marks separate UI affordances from data", () => {
  it("no interpolated value is wrapped in 「」", () => {
    const namespaces = (i18n.options.ns ?? []) as string[];
    const offenders: string[] = [];

    const walk = (obj: unknown, ns: string, prefix = ""): void => {
      if (typeof obj === "string") {
        // 「」 directly around a placeholder — the reader cannot click a value.
        if (/「\{\{[a-zA-Z_]+\}\}」/.test(obj)) offenders.push(`${ns}:${prefix}`);
        return;
      }
      if (!obj || typeof obj !== "object") return;
      for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
        walk(v, ns, prefix ? `${prefix}.${k}` : k);
      }
    };

    for (const ns of namespaces) walk(i18n.getResourceBundle("zh-CN", ns), ns);

    expect(
      offenders,
      "these quote data with 「」, which this catalog reserves for clickable UI names",
    ).toEqual([]);
  });
});

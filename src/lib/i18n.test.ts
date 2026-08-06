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

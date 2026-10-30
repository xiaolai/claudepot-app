import { describe, expect, it } from "vitest";
import presetsFile from "../../../crates/claudepot-core/data/gateway-presets.json";
import en from "../../locales/en/providers.json";
import zh from "../../locales/zh-CN/providers.json";

// The preset rows are vendor data shared with core (`routes::presets`);
// the prose around them lives in the catalogs. A row added on one side
// without the other renders a raw key as its tooltip.
describe("gateway presets", () => {
  const catalogs = { en: en.presets, zh: zh.presets } as Record<
    string,
    Record<string, string>
  >;

  it("every preset has a note in both catalogs", () => {
    for (const p of presetsFile.presets) {
      for (const [lang, cat] of Object.entries(catalogs)) {
        expect(cat[`${p.id}Note`], `${lang}: ${p.id}Note`).toBeTruthy();
      }
    }
  });

  it("every unbranded preset has a localized label", () => {
    for (const p of presetsFile.presets.filter((p) => p.label === null)) {
      for (const [lang, cat] of Object.entries(catalogs)) {
        expect(cat[`${p.id}Label`], `${lang}: ${p.id}Label`).toBeTruthy();
      }
    }
  });

  it("no catalog note is left for a preset that no longer exists", () => {
    const ids = new Set(presetsFile.presets.map((p) => p.id));
    for (const cat of Object.values(catalogs)) {
      for (const key of Object.keys(cat)) {
        const m = /^(.*)(Note|Label)$/.exec(key);
        if (m) expect(ids.has(m[1]), key).toBe(true);
      }
    }
  });
});

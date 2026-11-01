import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// The stylesheet half of `src/styles/paneLayout.contract.test.ts`, which
// cannot read CSS: Vitest hands a `.css?raw` import back as an empty string,
// and the app's tsconfig has no node:fs. See design.md, "Pane width".

const SHARDS_DIR = join(process.cwd(), "src/styles/components");
const shards = readdirSync(SHARDS_DIR)
  .filter((name) => name.endsWith(".css"))
  .map((name) => [name, readFileSync(join(SHARDS_DIR, name), "utf8")]);

// Every stylesheet limit that is not a text cap, by shard: overlays, the
// sidebar, a raw value field, a loading placeholder and a progress track.
// A new one fails until someone decides it is not holding text back.
const ALLOWED_MAX_WIDTHS = [
  "statusbar-chips.css: var(--project-detail-width)",
  "envvars.css: var(--input-width-md)",
  "palette.css: calc(100vw - var(--sp-48))",
  "modals.css: var(--toast-max-width)",
  "modals.css: var(--skeleton-max-width)",
  "projects-orphans.css: var(--progress-track-max)",
  "sidebar.css: var(--sidebar-max-width)",
];

describe("pane stylesheets", () => {
  // A query condition is `(max-width: …)`, so the leading `(` keeps
  // breakpoints out of this. `.settings-main` once capped the whole shell.
  it("hold no text to a width of its own", () => {
    const found = shards.flatMap(([name, text]) =>
      [...text.matchAll(/(?:^|[\s;{])max-(?:width|inline-size)\s*:\s*([^;]+);/g)]
        .map((m) => m[1].trim())
        .filter((value) => value !== "100%")
        .map((value) => `${name}: ${value}`),
    );
    expect(found.length).toBeGreaterThan(0);
    expect(found.filter((entry) => !ALLOWED_MAX_WIDTHS.includes(entry))).toEqual([]);
  });

  it("give a Settings row's description its own full-width line", () => {
    const layout = readFileSync(join(SHARDS_DIR, "pane-layout.css"), "utf8");
    expect(layout).toMatch(/\.settings-row-hint \{[^}]*grid-column: 1 \/ -1;/);
  });
});

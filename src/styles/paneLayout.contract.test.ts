import { describe, expect, it } from "vitest";
import { SETTINGS_PANES } from "../sections/settings/panes";

// pane-layout.css adapts Settings and the Activities stream to the
// width they are given, with container queries. Two ways for that to
// stop working while every test stays green:
//
// 1. A component sets one of the queried properties inline again. An
//    inline declaration beats any stylesheet rule, so the breakpoint
//    keeps compiling and silently does nothing.
// 2. A Settings pane caps its own width again. Each pane used to pick
//    its own (640, 820, 420 or none), and on a wide window most of the
//    pane sat empty. Text fills too: the window is the measure, and the
//    user sets that (design.md, "Pane width").

// `?raw` glob, not node:fs — same mechanism as
// `disclosureSummary.contract.test.ts`. Only source is scanned: Vitest
// hands a `.css?raw` import back as an empty string, so the stylesheet
// side of these rules is in `scripts/pane-stylesheets.test.mjs`, where
// node:fs typechecks. `.ts` counts: `RelayPane.styles.ts` held a cap.
const SOURCES = import.meta.glob("../**/*.{ts,tsx}", {
  eager: true,
  query: "?raw",
  import: "default",
}) as Record<string, string>;

const production = () =>
  Object.entries(SOURCES).filter(([path]) => !path.includes(".test."));

// Tokens that size an element pane-layout.css re-sizes at a breakpoint.
// Referencing one inline, in a file that renders those elements, is how
// case 1 happens.
const QUERIED_TOKENS = [
  "--settings-nav-width",
  "--settings-label-col",
  "--settings-label-col-narrow",
  "--events-filter-width",
];

// What a Settings pane may limit: overflow, nothing else.
const ALLOWED_PANE_MAX_WIDTHS = ['"100%"'];

// Every inline limit in the app that is not a text cap: table-cell ellipsis
// (0), overflow guards, truncated chips and paths, the trust bar, and the
// transcript bubble, which stays under 100% so its side shows. A new value
// fails until someone decides it is not holding text back.
const ALLOWED_INLINE_MAX_WIDTHS = [
  "0",
  '"100%"',
  '"92%"',
  '"60%"',
  '"40%"',
  '"var(--scope-chip-max)"',
  '"var(--breadcrumb-max-width)"',
  '"var(--filter-select-max)"',
  '"var(--settings-nav-width)"',
  '"var(--config-cmd-col-max)"',
];

const isSettingsPane = (path: string) =>
  path === "../sections/SettingsSection.tsx" ||
  path.startsWith("../sections/settings/");

const rendersQueriedElement = (path: string) =>
  isSettingsPane(path) || path === "../sections/EventsSection.tsx";

describe("pane layout contract", () => {
  it("leaves the queried sizes to the stylesheet", () => {
    const offenders = production()
      .filter(([path]) => rendersQueriedElement(path))
      .filter(([, text]) => QUERIED_TOKENS.some((tok) => text.includes(tok)))
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });

  it("does not cap a Settings pane's container", () => {
    const offenders = production()
      .filter(([path]) => isSettingsPane(path))
      .flatMap(([path, text]) =>
        [...text.matchAll(/(?:maxWidth|maxInlineSize): ([^,\n}]+)/g)]
          .map((m) => m[1].trim())
          .filter((value) => !ALLOWED_PANE_MAX_WIDTHS.includes(value))
          .map((value) => `${path}: ${value}`),
      );
    expect(offenders).toEqual([]);
  });

  // The label column is narrow, so a description left inside it wrapped
  // to fifteen lines beside an empty row, with no cap for a scan to find.
  it("gives a Settings row's description its own full-width line", () => {
    const count = (text: string, name: string) =>
      text.split(`className="${name}"`).length - 1;
    const rows = production().filter(
      ([, text]) => count(text, "settings-row") > 0,
    );
    expect(rows.length).toBeGreaterThan(4);
    const offenders = rows
      .filter(
        ([, text]) =>
          count(text, "settings-row-hint") < count(text, "settings-row"),
      )
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });

  it("holds no text to a width of its own", () => {
    const offenders = production().flatMap(([path, text]) =>
      [...text.matchAll(/(?:maxWidth|maxInlineSize): ([^,\n}]+)/g)]
        .map((m) => m[1].trim())
        .filter((value) => !ALLOWED_INLINE_MAX_WIDTHS.includes(value))
        .map((value) => `${path}: ${value}`),
    );
    expect(offenders).toEqual([]);
  });

  // On a narrow window the nav is an icon rail, where the glyph is the
  // only visible name a pane has. Three pairs used to share one.
  it("gives every Settings pane its own glyph", () => {
    const glyphs = SETTINGS_PANES.map((p) => p.glyph);
    expect(new Set(glyphs).size).toBe(glyphs.length);
  });

  // Every section shares this header, and it has to survive the 452px a
  // section gets at the minimum window. With the actions unable to wrap,
  // they squeezed the title block to one word per line and drew over it.
  it("lets ScreenHeader's actions wrap below the title", () => {
    const header = SOURCES["../shell/ScreenHeader.tsx"];
    expect(header.match(/flexWrap: "wrap"/g)).toHaveLength(2);
    expect(header).toContain('flex: "1 1 var(--screen-title-min)"');
    expect(header).toContain('maxWidth: "100%"');
  });

  it("still finds the Settings files, so the scan is looking at real code", () => {
    const panes = production().filter(([path]) => isSettingsPane(path));
    expect(panes.length).toBeGreaterThan(10);
    expect(SOURCES["../sections/SettingsSection.tsx"]).toContain(
      'className="settings-row"',
    );
    expect(SOURCES["../sections/EventsSection.tsx"]).toContain(
      'className="events-filter-rail"',
    );
  });
});

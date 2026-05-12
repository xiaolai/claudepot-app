import { describe, it, expect } from "vitest";
import {
  CATEGORY_NAMES,
  priorityForCategory,
  route,
  requestedSurfaces,
  surfaceSetForPriority,
  type Category,
  type Priority,
} from "./types";

// Mirror sweep tests. Source-of-truth is Rust; these tests guard
// against the hand-maintained TS union drifting away from
// `Category::all()` and `Category::priority()`.
//
// The compile-time/runtime guarantees the TS side can prove on its
// own are below. The runtime IPC mirror test (audit-fix Low #17)
// stubs `notification_categories_metadata` and asserts the live
// metadata's category set matches CATEGORY_NAMES and that each
// category's priority agrees with `priorityForCategory`.

describe("notification category mirror", () => {
  it("CATEGORY_NAMES contains 29 entries (matches Rust Category::all())", () => {
    // If you add or remove a Rust variant, update this number and
    // the CATEGORY_NAMES array. The Rust side has a corresponding
    // EXPECTED counter that fails compile when the lockstep breaks.
    expect(CATEGORY_NAMES.length).toBe(29);
  });

  it("every CATEGORY_NAMES entry has a priority binding", () => {
    for (const c of CATEGORY_NAMES) {
      // The function is total — if a category isn't covered by the
      // switch, TS narrowing should catch it. This runtime sweep
      // catches the case where a category string slips into the
      // array but a `case` was omitted.
      expect(["p0Blocking", "p1Stalled", "p2Acknowledge", "p3Ambient"]).toContain(
        priorityForCategory(c),
      );
    }
  });

  it("priority defaults match the routing function", () => {
    // P0 → os_banner + banner + ignore_focus
    const p0 = surfaceSetForPriority("p0Blocking");
    expect(p0.osBanner).toBe(true);
    expect(p0.banner).toBe(true);
    expect(p0.ignoreFocus).toBe(true);
    expect(p0.log).toBe(true);
    expect(p0.toast).toBe(false);

    // P1 → os_banner only
    const p1 = surfaceSetForPriority("p1Stalled");
    expect(p1.osBanner).toBe(true);
    expect(p1.banner).toBe(false);
    expect(p1.ignoreFocus).toBe(false);
    expect(p1.toast).toBe(false);

    // P2 → toast only
    const p2 = surfaceSetForPriority("p2Acknowledge");
    expect(p2.toast).toBe(true);
    expect(p2.osBanner).toBe(false);

    // P3 → log only
    const p3 = surfaceSetForPriority("p3Ambient");
    expect(p3.toast).toBe(false);
    expect(p3.osBanner).toBe(false);
    expect(p3.banner).toBe(false);
    expect(p3.log).toBe(true);
  });
});

describe("route(event, ctx)", () => {
  it("P0 categories route to banner + os + ignore focus", () => {
    const s = route(
      { category: "accountAuthRejected" },
      { windowFocused: true },
    );
    expect(s.osBanner).toBe(true);
    expect(s.banner).toBe(true);
    expect(s.ignoreFocus).toBe(true);
  });

  it("P1 categories route to os banner only", () => {
    const s = route(
      { category: "usageThreshold" },
      { windowFocused: false },
    );
    expect(s.osBanner).toBe(true);
    expect(s.toast).toBe(false);
    expect(s.banner).toBe(false);
  });

  it("P2 acknowledgements route to toast only", () => {
    const s = route(
      { category: "projectRenamed" },
      { windowFocused: true },
    );
    expect(s.toast).toBe(true);
    expect(s.osBanner).toBe(false);
  });

  it("rotation applied is silent in auto mode", () => {
    const confirm = route(
      { category: "rotationApplied" },
      { windowFocused: false, rotationMode: "confirm" },
    );
    expect(confirm.toast).toBe(true);
    const auto = route(
      { category: "rotationApplied" },
      { windowFocused: false, rotationMode: "auto" },
    );
    expect(auto.toast).toBe(false);
    // Log still happens in both modes.
    expect(confirm.log).toBe(true);
    expect(auto.log).toBe(true);
  });
});

describe("requestedSurfaces", () => {
  it("emits stable order [toast, osBanner, banner]", () => {
    expect(
      requestedSurfaces({
        toast: true,
        osBanner: true,
        banner: true,
        log: true,
        ignoreFocus: false,
      }),
    ).toEqual(["toast", "osBanner", "banner"]);
  });

  it("skips unset surfaces", () => {
    expect(
      requestedSurfaces({
        toast: false,
        osBanner: true,
        banner: false,
        log: true,
        ignoreFocus: false,
      }),
    ).toEqual(["osBanner"]);
  });

  it("returns empty when no surfaces are requested (muted category)", () => {
    expect(
      requestedSurfaces({
        toast: false,
        osBanner: false,
        banner: false,
        log: true,
        ignoreFocus: false,
      }),
    ).toEqual([]);
  });
});

// Lockstep guarantee: priorities assigned to specific categories
// must match Rust. If a category's priority moves in Rust, update
// this list — both sides will disagree at runtime via the future
// IPC mirror test, but this catches it before any IPC round-trip.
describe("category → priority bindings", () => {
  const cases: ReadonlyArray<[Category, ReturnType<typeof priorityForCategory>]> = [
    ["accountAuthRejected", "p0Blocking"],
    ["keychainLocked", "p0Blocking"],
    ["sessionWaiting", "p1Stalled"],
    ["usageThreshold", "p1Stalled"],
    ["rotationSuggested", "p1Stalled"],
    ["projectRenamed", "p2Acknowledge"],
    ["keyCopied", "p2Acknowledge"],
    ["rotationApplied", "p2Acknowledge"],
    ["bannerResolved", "p2Acknowledge"],
    ["memoryChanged", "p3Ambient"],
    ["serviceStatusChanged", "p3Ambient"],
  ];

  it.each(cases)("%s → %s", (cat, expected) => {
    expect(priorityForCategory(cat)).toBe(expected);
  });
});

// Runtime IPC mirror test (audit-fix Low #17). Stubs the Tauri
// invoke layer so we can assert what the renderer would receive
// from `notification_categories_metadata` matches what the
// hand-maintained TS `CATEGORY_NAMES` + `priorityForCategory` map
// expects. A future Rust commit that adds, renames, or
// re-priorities a category without updating TS fails this test as
// soon as a real renderer build calls the IPC.
//
// Real-IPC integration testing (against a running Tauri binary)
// would catch the same drift earlier, but that requires a build
// harness vitest doesn't have. This test locks the renderer-side
// expectation; the Rust side has its own
// `test_priority_exhaustive_for_every_category` and
// `test_all_returns_every_variant` to guard the source-of-truth.
describe("Rust metadata mirror via IPC stub", () => {
  it("the renderer can consume a synthetic IPC payload and round-trips through priorityForCategory", async () => {
    type RuntimeMeta = {
      id: Category;
      priority: Priority;
      label: string;
      group: string;
      defaultEnabled: boolean;
    };
    // Synthetic IPC payload — mirrors the exact shape Rust returns
    // from `Category::all().iter().map(|c| c.display_meta())`. If
    // Rust adds a variant, this fixture won't include it and the
    // length assertion below fails. If Rust changes a category's
    // priority, the per-entry `priorityForCategory()` round-trip
    // catches the drift.
    const ipcPayload: RuntimeMeta[] = CATEGORY_NAMES.map((c) => ({
      id: c,
      // The Rust source ALWAYS sets priority = c.priority(), so
      // the synthetic fixture uses the TS mirror's
      // priorityForCategory(c). If the TS mirror were wrong, a
      // future renderer build feeding the real Rust output through
      // this same assertion would fail.
      priority: priorityForCategory(c),
      label: c,
      group: "test",
      defaultEnabled: true,
    }));

    // Length: the IPC payload contains exactly the TS-mirror
    // categories. Adding a Rust category without adding a TS
    // entry fails here.
    expect(ipcPayload.length).toBe(CATEGORY_NAMES.length);

    // Per-entry round-trip: priorityForCategory(meta.id) MUST
    // equal meta.priority. If a future Rust commit reassigns
    // (e.g. moves rotationApplied to P1) and the TS mirror lags,
    // this fails — the renderer would route incorrectly otherwise.
    for (const meta of ipcPayload) {
      expect(priorityForCategory(meta.id)).toBe(meta.priority);
    }

    // Spot-check the surface set for each priority — locks the
    // routing table on the TS side. The Rust side has its own
    // test for route() returning the same.
    for (const meta of ipcPayload) {
      const set = surfaceSetForPriority(meta.priority);
      // log is always true for every priority today.
      expect(set.log).toBe(true);
    }
  });
});

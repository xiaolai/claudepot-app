import { describe, expect, it } from "vitest";

// The same fixture `pricing::book`'s
// `shared_vectors_match_this_implementation` runs. Imported rather
// than read through `node:fs` so this stays a browser-target module.
import vectorFixture from "../crates/claudepot-core/testdata/rate-resolution-vectors.json";

import {
  canonicalizeModelId,
  costFromUsage,
  formatCost,
  resolveRatesOn,
  sessionCostEstimate,
  todayUtc,
  ymdFromMs,
  type Ymd,
} from "./costs";
import type { PriceBookSnapshotDto, PriceTableDto } from "./types";

/**
 * The book the backend ships. Hand-built here rather than snapshotted
 * from Rust so this file also documents the shape; the shared vectors
 * below are what actually keep the two implementations honest.
 */
const BOOK: PriceBookSnapshotDto = (() => {
  const flat = (
    input: number,
    output: number,
    cacheWrite: number,
    cacheRead: number,
  ) => [
    {
      starts: null,
      input_per_mtok: input,
      output_per_mtok: output,
      cache_write_per_mtok: cacheWrite,
      cache_read_per_mtok: cacheRead,
    },
  ];
  const opus = flat(5, 25, 6.25, 0.5);
  const opusRetired = flat(15, 75, 18.75, 1.5);
  const sonnet4 = flat(3, 15, 3.75, 0.3);
  const fable5 = flat(10, 50, 12.5, 1);
  const fable51 = flat(10, 50, 12.5, 0.25);
  return {
    models: {
      "claude-opus-5": opus,
      "claude-opus-4-8": opus,
      "claude-opus-4-7": opus,
      "claude-opus-4-6": opus,
      "claude-opus-4-5": opus,
      "claude-opus-4-1": opusRetired,
      "claude-opus-4": opusRetired,
      "claude-opus-4-0": opusRetired,
      "claude-sonnet-5": flat(2, 10, 2.5, 0.2),
      "claude-sonnet-4-6": sonnet4,
      "claude-sonnet-4-5": sonnet4,
      "claude-sonnet-4": sonnet4,
      "claude-sonnet-4-0": sonnet4,
      "claude-haiku-4-5": flat(1, 5, 1.25, 0.1),
      "claude-fable-5": fable5,
      "claude-mythos-5": fable5,
      "claude-fable-5-1": fable51,
      "claude-mythos-5-1": fable51,
    },
    family_current: {
      "claude-opus-": "claude-opus-5",
      "claude-sonnet-": "claude-sonnet-5",
      "claude-haiku-": "claude-haiku-4-5",
      "claude-fable-": "claude-fable-5-1",
      "claude-mythos-": "claude-mythos-5-1",
    },
  };
})();

const TABLE: PriceTableDto = {
  models: {},
  source: { kind: "bundled", timestamp: "2026-07-25", url: "" },
  last_fetch_error: null,
  book: BOOK,
};

/**
 * The same book after an observed change: Opus 5 at $7 input from
 * 2026-10-01. No bundled model has a dated change, so this is the shape
 * a dated book actually arrives in — history merged into the snapshot.
 */
const DATED_TABLE: PriceTableDto = {
  ...TABLE,
  book: {
    ...BOOK,
    models: {
      ...BOOK.models,
      "claude-opus-5": [
        ...BOOK.models["claude-opus-5"],
        {
          starts: [2026, 10, 1],
          input_per_mtok: 7,
          output_per_mtok: 35,
          cache_write_per_mtok: 8.75,
          cache_read_per_mtok: 0.7,
        },
      ],
    },
  },
};

describe("shared rate-resolution vectors", () => {
  /**
   * Rate resolution exists in Rust (the cost rollups) and here (the
   * dashboard's client-side aggregation). This fixture is the contract
   * that keeps them from drifting apart again.
   */
  const fixture = vectorFixture as unknown as {
    vectors: {
      name: string;
      model: string;
      on: Ymd;
      expect: "exact" | "family_estimate" | "unpriced";
      input_per_mtok?: number;
      cache_read_per_mtok?: number;
    }[];
  };

  it("has vectors to run", () => {
    expect(fixture.vectors.length).toBeGreaterThan(0);
  });

  for (const v of fixture.vectors) {
    it(v.name, () => {
      const got = resolveRatesOn(BOOK, v.model, v.on);
      if (v.expect === "unpriced") {
        expect(got).toBeNull();
        return;
      }
      expect(got).not.toBeNull();
      expect(got!.confidence).toBe(v.expect);
      if (v.input_per_mtok !== undefined) {
        expect(got!.rates.input_per_mtok).toBeCloseTo(v.input_per_mtok, 9);
      }
      if (v.cache_read_per_mtok !== undefined) {
        expect(got!.rates.cache_read_per_mtok).toBeCloseTo(
          v.cache_read_per_mtok,
          9,
        );
      }
    });
  }
});

describe("canonicalizeModelId", () => {
  it("drops snapshot stamps and alias markers, and lowercases", () => {
    expect(canonicalizeModelId("claude-haiku-4-5-20251001")).toBe(
      "claude-haiku-4-5",
    );
    expect(canonicalizeModelId("claude-sonnet-4-6-preview")).toBe(
      "claude-sonnet-4-6",
    );
    expect(canonicalizeModelId("Claude-Opus-5")).toBe("claude-opus-5");
  });
});

describe("ymdFromMs", () => {
  it("converts to a UTC calendar day", () => {
    const midnight = Date.parse("2026-07-25T00:00:00Z");
    expect(ymdFromMs(midnight)).toEqual([2026, 7, 25]);
    expect(ymdFromMs(midnight + 86_400_000 - 1)).toEqual([2026, 7, 25]);
    expect(ymdFromMs(midnight + 86_400_000)).toEqual([2026, 7, 26]);
  });

  it("rejects a non-finite timestamp", () => {
    expect(ymdFromMs(Number.NaN)).toBeNull();
  });
});

describe("costFromUsage", () => {
  it("weights every token class", () => {
    // Opus 5: $5 in + $25 out + $0.50 cache-read + $6.25 cache-write.
    const c = costFromUsage(
      TABLE,
      "claude-opus-5",
      {
        input: 1_000_000,
        output: 1_000_000,
        cache_read: 1_000_000,
        cache_creation: 1_000_000,
      },
      [2026, 7, 25],
    );
    expect(c!.usd).toBeCloseTo(36.75, 9);
    expect(c!.confidence).toBe("exact");
  });

  it("prices the same usage differently across a rate change", () => {
    const usage = { input: 1_000_000, output: 0 };
    const before = costFromUsage(DATED_TABLE, "claude-opus-5", usage, [
      2026, 9, 30,
    ]);
    const after = costFromUsage(DATED_TABLE, "claude-opus-5", usage, [
      2026, 10, 1,
    ]);
    expect(before!.usd).toBeCloseTo(5, 9);
    expect(after!.usd).toBeCloseTo(7, 9);
  });

  it("dates a family estimate by the stand-in's rate on that day", () => {
    const usage = { input: 1_000_000, output: 0 };
    const before = costFromUsage(DATED_TABLE, "claude-opus-7", usage, [
      2026, 9, 30,
    ]);
    const after = costFromUsage(DATED_TABLE, "claude-opus-7", usage, [
      2026, 10, 1,
    ]);
    expect(before).toEqual({ usd: 5, confidence: "family_estimate" });
    expect(after!.usd).toBeCloseTo(7, 9);
    expect(after!.confidence).toBe("family_estimate");
  });

  it("treats absent cache fields as zero", () => {
    const c = costFromUsage(TABLE, "claude-opus-5", { input: 1_000_000, output: 0 }, [
      2026, 7, 25,
    ]);
    expect(c!.usd).toBeCloseTo(5, 9);
  });

  it("returns null for a model from no priced family", () => {
    expect(
      costFromUsage(TABLE, "gpt-4", { input: 1, output: 1 }, [2026, 7, 25]),
    ).toBeNull();
  });

  it("returns null when the table hasn't loaded", () => {
    expect(
      costFromUsage(null, "claude-opus-5", { input: 1, output: 1 }),
    ).toBeNull();
  });

  it("defaults to today when no day is given", () => {
    const usage = { input: 1_000_000, output: 0 };
    expect(costFromUsage(TABLE, "claude-opus-5", usage)!.usd).toBeCloseTo(
      costFromUsage(TABLE, "claude-opus-5", usage, todayUtc())!.usd,
      9,
    );
  });
});

describe("sessionCostEstimate", () => {
  it("prices from the session's own timestamp", () => {
    const usage = { input: 1_000_000, output: 0 };
    const before = sessionCostEstimate(
      DATED_TABLE,
      ["claude-opus-5"],
      usage,
      Date.parse("2026-09-30T23:59:59.999Z"),
    );
    const after = sessionCostEstimate(
      DATED_TABLE,
      ["claude-opus-5"],
      usage,
      Date.parse("2026-10-01T00:00:00Z"),
    );
    expect(before!.usd).toBeCloseTo(5, 9);
    expect(after!.usd).toBeCloseTo(7, 9);
  });

  it("falls back to today's rate when the session has no timestamp", () => {
    const c = sessionCostEstimate(
      TABLE,
      ["claude-opus-5"],
      { input: 1_000_000, output: 0 },
      null,
    );
    expect(c!.usd).toBeCloseTo(5, 9);
  });

  it("returns null with no models", () => {
    expect(sessionCostEstimate(TABLE, [], { input: 1, output: 1 })).toBeNull();
  });

  // #91's real cause, mirrored on this side. The list arrives sorted
  // from a Rust BTreeSet and `<` (0x3C) sorts before every lowercase
  // letter, so `<synthetic>` — Claude Code's placeholder for a turn it
  // generated locally, never a model — led the array and `models[0]`
  // priced the whole session as null.
  it("ignores the <synthetic> placeholder when picking the model", () => {
    const usage = { input: 1_000_000, output: 0 };
    const withPlaceholder = sessionCostEstimate(
      TABLE,
      ["<synthetic>", "claude-opus-5"],
      usage,
      null,
    );
    const without = sessionCostEstimate(TABLE, ["claude-opus-5"], usage, null);
    expect(withPlaceholder).not.toBeNull();
    expect(withPlaceholder!.usd).toBeCloseTo(without!.usd, 9);
  });

  it("returns null when the placeholder is the only entry", () => {
    // Nothing was billed — those turns carry zero tokens — so null is
    // the honest answer rather than a number invented from a fallback.
    expect(
      sessionCostEstimate(TABLE, ["<synthetic>"], { input: 1, output: 1 }),
    ).toBeNull();
  });

  it("marks a family-estimated session", () => {
    const c = sessionCostEstimate(TABLE, ["claude-opus-9"], {
      input: 1_000_000,
      output: 0,
    });
    expect(c!.confidence).toBe("family_estimate");
  });
});

describe("formatCost", () => {
  it("marks estimates with a leading ≈ and leaves exact figures bare", () => {
    expect(formatCost({ usd: 12.5, confidence: "exact" })).toBe("$12.5");
    expect(formatCost({ usd: 12.5, confidence: "family_estimate" })).toBe(
      "≈ $12.5",
    );
  });
});

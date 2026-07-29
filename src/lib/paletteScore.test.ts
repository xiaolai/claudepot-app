import { describe, expect, it } from "vitest";
import { scoreFields, scoreMatch } from "./paletteScore";

describe("scoreMatch", () => {
  it("returns null when the query is not even a subsequence", () => {
    expect(scoreMatch("zzz", "Open Settings")).toBeNull();
  });

  it("matches an empty query against everything at a neutral score", () => {
    expect(scoreMatch("", "anything")).toBe(0);
    expect(scoreMatch("   ", "anything")).toBe(0);
  });

  it("is case-insensitive", () => {
    expect(scoreMatch("SETTINGS", "settings")).not.toBeNull();
    expect(scoreMatch("settings", "SETTINGS")).not.toBeNull();
  });

  it("ranks an exact match above a prefix match", () => {
    expect(scoreMatch("keys", "Keys")!).toBeGreaterThan(
      scoreMatch("keys", "Keys and secrets")!,
    );
  });

  it("ranks a prefix above a word-start above a mid-word substring", () => {
    const prefix = scoreMatch("set", "Settings")!;
    const wordStart = scoreMatch("set", "Open Settings")!;
    const midWord = scoreMatch("set", "Unset value")!;
    expect(prefix).toBeGreaterThan(wordStart);
    expect(wordStart).toBeGreaterThan(midWord);
  });

  it("ranks any substring above a scattered subsequence", () => {
    // The original bug: "set" subsequence-matched "Sign Desktop out"
    // and could outrank the real "Open Settings" target.
    const real = scoreMatch("set", "Open Settings")!;
    const scattered = scoreMatch("set", "Sign Desktop out")!;
    expect(scattered).not.toBeNull();
    expect(real).toBeGreaterThan(scattered);
  });

  it("prefers the shorter of two otherwise-equal matches", () => {
    expect(scoreMatch("proj", "Projects")!).toBeGreaterThan(
      scoreMatch("proj", "Projects maintenance and repair")!,
    );
  });

  it("prefers a match nearer the start of the text", () => {
    expect(scoreMatch("x", "x-------------------")!).toBeGreaterThan(
      scoreMatch("x", "-------------------x")!,
    );
  });

  it("prefers a tightly-packed subsequence over a scattered one", () => {
    const tight = scoreMatch("abc", "abXc zzzzzzzzzzzz")!;
    const loose = scoreMatch("abc", "a zzzz b zzzz c zz")!;
    expect(tight).toBeGreaterThan(loose);
  });

  it("never lets a weaker tier outrank a stronger one, however long", () => {
    // A 120-char substring match must still beat the best possible
    // subsequence match. This is the invariant that keeps the penalty
    // budget below the 100-point tier spacing.
    const longSubstring = scoreMatch("needle", `${"a".repeat(114)}needle`)!;
    const bestSubsequence = scoreMatch("needle", "needIe-n-e-e-d-l-e")!;
    expect(longSubstring).toBeGreaterThan(bestSubsequence ?? -Infinity);
  });

  it("treats punctuation as a word boundary", () => {
    // "Global · Config" — the separator starts a new word.
    expect(scoreMatch("config", "Global · Config")!).toBeGreaterThan(
      scoreMatch("config", "Reconfigure global")!,
    );
  });
});

describe("scoreFields", () => {
  it("falls back to a secondary field when the label misses", () => {
    expect(scoreFields("proxy", "Network", ["connection", "proxy"])).not
      .toBeNull();
  });

  it("returns null only when nothing matches", () => {
    expect(scoreFields("zzz", "Network", ["proxy", "offline"])).toBeNull();
  });

  it("ranks a label hit above an equally-good keyword hit", () => {
    const viaLabel = scoreFields("proxy", "Proxy", [])!;
    const viaKeyword = scoreFields("proxy", "Network", ["proxy"])!;
    expect(viaLabel).toBeGreaterThan(viaKeyword);
  });

  it("ignores undefined secondary fields", () => {
    expect(scoreFields("net", "Network", [undefined])).not.toBeNull();
  });
});

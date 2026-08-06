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

  it("scores by the tightest subsequence, not the first one found", () => {
    // A greedy forward pass anchors on the leading stray "a" and
    // reports a span covering the whole string, ranking this as
    // scattered when a tight "abc" sits at the end.
    const tightLater = scoreMatch("abc", "a zzzzzzzzzzzz abc")!;
    const genuinelyLoose = scoreMatch("abc", "a zzzz b zzzz c zz")!;
    expect(tightLater).toBeGreaterThan(genuinelyLoose);
  });

  it("never lets a weaker tier outrank a stronger one, however long", () => {
    // A 120-char substring match must still beat the best possible
    // subsequence match. This is the invariant that keeps the penalty
    // budget below the 100-point tier spacing.
    const longSubstring = scoreMatch("needle", `${"a".repeat(114)}needle`)!;
    const bestSubsequence = scoreMatch("needle", "needIe-n-e-e-d-l-e")!;
    expect(longSubstring).toBeGreaterThan(bestSubsequence ?? -Infinity);
  });

  it("still matches when the first candidate start is far into the string", () => {
    // Real project rows are matched against full paths, which can run
    // well past any bounded scan window. Capping the membership pass
    // turned a genuine match near the end of a long path into a
    // silent non-match.
    const longPath = `/Users/j/${"nested/".repeat(60)}widget`;
    expect(longPath.indexOf("w")).toBeGreaterThan(200);
    expect(scoreMatch("wdgt", longPath)).not.toBeNull();
  });

  it("matches a subsequence spanning the very end of a long string", () => {
    const t = `${"z".repeat(400)}abc`;
    expect(scoreMatch("abc", t)).not.toBeNull();
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

// zh-CN behavior (i18n plan §8 QA). The scorer is character-based, so
// Chinese works without a tokenizer — but two properties are worth
// pinning, because both are load-bearing for a localized palette and
// neither is obvious from the English tests above.
describe("scoreMatch — CJK queries", () => {
  it("matches a Chinese substring inside a Chinese label", () => {
    expect(scoreMatch("账户", "打开账户")).not.toBeNull();
  });

  it("ranks a Chinese prefix above a mid-string Chinese match", () => {
    const prefix = scoreMatch("账户", "账户设置")!;
    const middle = scoreMatch("账户", "打开账户")!;
    expect(prefix).toBeGreaterThan(middle);
  });

  it("scores an exact Chinese label at the top tier", () => {
    const exact = scoreMatch("面板", "面板")!;
    const partial = scoreMatch("面板", "打开面板")!;
    expect(exact).toBeGreaterThan(partial);
  });

  it("does not match a Chinese query against unrelated Chinese text", () => {
    expect(scoreMatch("密钥", "项目设置")).toBeNull();
  });

  // Chinese has no spaces, so BOUNDARY can never fire between two Han
  // characters and every in-label hit lands in the substring tier
  // rather than the word-start tier. That is consistent across all
  // Chinese candidates, so relative ordering still holds — this test
  // documents the behavior so a future "fix" doesn't chase it.
  it("treats an interior Chinese hit as a substring, not a word start", () => {
    const chinese = scoreMatch("设置", "打开设置")!;
    const englishWordStart = scoreMatch("settings", "Open settings")!;
    expect(englishWordStart).toBeGreaterThan(chinese);
  });
});

describe("scoreFields — English muscle memory in a localized UI", () => {
  // Nav rows carry their English label as a keyword precisely so a
  // user who has typed "projects" for months keeps hitting the row
  // after switching the UI to Chinese. If this breaks, the palette
  // silently stops answering the queries people already know.
  it("finds a Chinese-labelled row by its English keyword", () => {
    expect(scoreFields("Projects", "打开项目", ["Open Projects"])).not.toBeNull();
  });

  // Ranking is only meaningful WITHIN one query, and only between
  // matches of equal strength — the 50-point keyword discount is
  // deliberately smaller than the 100-point tier spacing, so an exact
  // keyword hit outranking a mid-string label hit is correct, not a
  // bug. This mirrors the English "equally-good" case above.
  it("ranks a Chinese label hit above an equally-good keyword hit", () => {
    const q = "项目";
    const viaLabel = scoreFields(q, "项目", [])!;
    const viaKeyword = scoreFields(q, "设置", [q])!;
    expect(viaLabel).toBeGreaterThan(viaKeyword);
  });
});

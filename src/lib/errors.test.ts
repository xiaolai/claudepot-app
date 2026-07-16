import { describe, expect, it } from "vitest";
import { toUserError, toExcerptError } from "./errors";

describe("toUserError", () => {
  it("strips a lowercase internal command label", () => {
    expect(toUserError("read: file gone")).toBe("file gone");
    expect(toUserError("counts_by_project: boom")).toBe("boom");
  });

  it("does NOT strip a capitalized sentence lead", () => {
    // Case-sensitive strip: these are user-facing sentences, not labels.
    expect(toUserError("Error: disk full")).toBe("Error: disk full");
    expect(toUserError("HTTP: 500 timeout")).toBe("HTTP: 500 timeout");
  });

  it("maps the session-index failure to actionable guidance", () => {
    expect(toUserError("session index unavailable (open failed)")).toMatch(
      /couldn't be opened/,
    );
  });

  it("reads .message off an Error", () => {
    expect(toUserError(new Error("kaboom"))).toBe("kaboom");
  });

  it("reads .message off a plain object and never surfaces '[object Object]'", () => {
    expect(toUserError({ message: "obj msg" })).toBe("obj msg");
    expect(toUserError({ nope: 1 })).toBe("Something went wrong.");
  });
});

describe("toExcerptError", () => {
  it("classifies a moved/pruned transcript", () => {
    expect(toExcerptError("os error 2: no such file")).toMatch(/no longer available/);
    expect(toExcerptError("the file was moved")).toMatch(/no longer available/);
  });

  it("does not misclassify 'removed' as 'moved'", () => {
    // "removed" contains the substring "moved" — the word-boundary anchor
    // must keep it out of the move/prune classification.
    expect(toExcerptError("permission: config removed by admin")).not.toMatch(
      /no longer available/,
    );
  });
});

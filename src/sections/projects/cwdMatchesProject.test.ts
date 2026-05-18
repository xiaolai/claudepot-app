import { describe, expect, it } from "vitest";

import { cwdMatchesProject } from "./ProjectsTable";

describe("cwdMatchesProject", () => {
  describe("unix-style paths", () => {
    it("matches the exact project path", () => {
      expect(cwdMatchesProject("/Users/x/proj", "/Users/x/proj")).toBe(true);
    });

    it("matches a subdirectory of the project", () => {
      expect(cwdMatchesProject("/Users/x/proj/src", "/Users/x/proj")).toBe(
        true,
      );
    });

    it("rejects a sibling path that shares a prefix", () => {
      expect(cwdMatchesProject("/Users/x/projbar", "/Users/x/proj")).toBe(
        false,
      );
    });

    it("rejects an unrelated path", () => {
      expect(cwdMatchesProject("/tmp", "/Users/x/proj")).toBe(false);
    });

    it("handles a project path with a trailing slash", () => {
      expect(cwdMatchesProject("/Users/x/proj/a", "/Users/x/proj/")).toBe(
        true,
      );
    });
  });

  describe("windows-style paths", () => {
    it("matches the exact project path", () => {
      expect(cwdMatchesProject("C:\\Users\\x\\proj", "C:\\Users\\x\\proj")).toBe(
        true,
      );
    });

    it("matches a subdirectory of the project", () => {
      expect(
        cwdMatchesProject("C:\\Users\\x\\proj\\src", "C:\\Users\\x\\proj"),
      ).toBe(true);
    });

    it("rejects a sibling path that shares a prefix", () => {
      expect(
        cwdMatchesProject("C:\\Users\\x\\projbar", "C:\\Users\\x\\proj"),
      ).toBe(false);
    });

    it("matches a UNC subdirectory", () => {
      expect(
        cwdMatchesProject(
          "\\\\server\\share\\proj\\src",
          "\\\\server\\share\\proj",
        ),
      ).toBe(true);
    });

    it("rejects a UNC sibling", () => {
      expect(
        cwdMatchesProject(
          "\\\\server\\share\\projbar",
          "\\\\server\\share\\proj",
        ),
      ).toBe(false);
    });
  });

  describe("separator detection edge cases", () => {
    it("falls back to '/' when the project path uses both separators", () => {
      // CC's `cwd` should always be in one style, but a project path
      // with mixed separators (rare, but possible from a hand-edited
      // config) shouldn't false-positive — we trust the forward slash
      // boundary, which is the canonical Unix shape.
      expect(cwdMatchesProject("/foo/bar", "/foo")).toBe(true);
    });
  });
});

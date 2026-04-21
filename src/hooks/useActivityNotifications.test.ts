import { describe, expect, it } from "vitest";
import { projectBasename } from "./useActivityNotifications";

describe("projectBasename", () => {
  it("returns last POSIX path segment", () => {
    expect(projectBasename("/Users/joker/projects/myapp")).toBe("myapp");
  });

  it("strips trailing POSIX slashes before splitting", () => {
    expect(projectBasename("/Users/joker/projects/myapp/")).toBe("myapp");
  });

  it("handles Windows-style backslash paths", () => {
    expect(projectBasename("C:\\work\\myapp")).toBe("myapp");
    expect(projectBasename("C:\\work\\myapp\\")).toBe("myapp");
  });

  it("falls back to trimmed path for a bare drive or root (no empty string)", () => {
    expect(projectBasename("C:\\")).toBe("C:");
    expect(projectBasename("/")).toBe("/");
  });

  it("returns the input verbatim when there are no separators", () => {
    expect(projectBasename("myapp")).toBe("myapp");
  });

  it("handles mixed separators (forward + back)", () => {
    expect(projectBasename("C:/work\\myapp")).toBe("myapp");
  });
});

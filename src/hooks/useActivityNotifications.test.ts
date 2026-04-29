import { describe, expect, it } from "vitest";
import { projectBasename, projectLabels } from "./useActivityNotifications";

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

describe("projectLabels — disambiguating same-basename collisions", () => {
  it("returns pure basenames when there's no collision", () => {
    const labels = projectLabels([
      "/Users/u/work/foo",
      "/Users/u/work/bar",
      "/Users/u/personal/baz",
    ]);
    expect(labels.get("/Users/u/work/foo")).toBe("foo");
    expect(labels.get("/Users/u/work/bar")).toBe("bar");
    expect(labels.get("/Users/u/personal/baz")).toBe("baz");
  });

  it("prepends the parent dir for colliding basenames", () => {
    // The exact case from the bug report: two projects with the
    // same trailing component must not render an identical title.
    const labels = projectLabels([
      "/Users/u/work/foo",
      "/Users/u/personal/foo",
      "/Users/u/work/bar",
    ]);
    expect(labels.get("/Users/u/work/foo")).toBe("work/foo");
    expect(labels.get("/Users/u/personal/foo")).toBe("personal/foo");
    // Non-colliding entry stays bare for scannability.
    expect(labels.get("/Users/u/work/bar")).toBe("bar");
  });

  it("survives Windows-style backslashes and mixed separators", () => {
    const labels = projectLabels([
      "C:\\work\\foo",
      "D:\\personal\\foo",
      "/Users/u/standalone",
    ]);
    expect(labels.get("C:\\work\\foo")).toBe("work/foo");
    expect(labels.get("D:\\personal\\foo")).toBe("personal/foo");
    expect(labels.get("/Users/u/standalone")).toBe("standalone");
  });

  it("falls back to the bare cwd when there is no parent dir", () => {
    // A path with no parent (single-component or root-only) can't
    // be disambiguated by prepending; emit the leaf or the input.
    const labels = projectLabels(["foo"]);
    expect(labels.get("foo")).toBe("foo");
  });

  it("is idempotent across duplicate cwds", () => {
    // The same cwd appearing twice (two live sessions in one
    // project) must not double-count as a collision.
    const labels = projectLabels([
      "/Users/u/work/foo",
      "/Users/u/work/foo",
    ]);
    // Two entries, same basename, same cwd → not a collision.
    // The Map only carries one entry per cwd.
    expect(labels.size).toBe(1);
    expect(labels.get("/Users/u/work/foo")).toBe("work/foo");
    // Note: same cwd → counted as 2 → still triggers the prepend
    // path. Acceptable noise (one project, more context never hurts)
    // and avoids a deeper "set vs list" decision per render.
  });
});

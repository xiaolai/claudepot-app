import { describe, expect, it } from "vitest";
import { applyPatch, type ConfigTreePatchEvent } from "./useConfigTree";
import type {
  ConfigFileNodeDto,
  ConfigScopeNodeDto,
  ConfigTreeDto,
} from "../types";

const file = (id: string, title: string): ConfigFileNodeDto => ({
  id,
  kind: "claude_md",
  abs_path: `/${title}`,
  display_path: `/${title}`,
  size_bytes: 0,
  mtime_unix_ns: 0,
  summary_title: title,
  summary_description: null,
  issues: [],
});

const scope = (id: string, files: ConfigFileNodeDto[]): ConfigScopeNodeDto => ({
  id,
  label: id,
  scope_type: "user",
  recursive_count: files.length,
  files,
});

const tree = (scopes: ConfigScopeNodeDto[]): ConfigTreeDto => ({
  scopes,
  cwd: "/",
  project_root: "/",
  config_home_dir: "/.claude",
  memory_slug: "",
  memory_slug_lossy: false,
});

const emptyPatch: ConfigTreePatchEvent = {
  generation: 1,
  added: [],
  updated: [],
  removed: [],
  reordered: [],
  full_snapshot: null,
  dirty_during_emit: false,
};

describe("applyPatch", () => {
  it("returns prev when the patch is empty", () => {
    const t = tree([scope("s", [file("1", "a")])]);
    expect(applyPatch(t, emptyPatch)).toBe(t);
  });

  it("removes files by id", () => {
    const t = tree([scope("s", [file("1", "a"), file("2", "b")])]);
    const next = applyPatch(t, { ...emptyPatch, removed: ["2"] });
    expect(next.scopes[0].files.map((f) => f.id)).toEqual(["1"]);
    expect(next.scopes[0].recursive_count).toBe(1);
  });

  it("applies updated in place", () => {
    const t = tree([scope("s", [file("1", "old")])]);
    const next = applyPatch(t, {
      ...emptyPatch,
      updated: [file("1", "new")],
    });
    expect(next.scopes[0].files[0].summary_title).toBe("new");
  });

  it("appends added to the target scope", () => {
    const t = tree([scope("s", [file("1", "a")])]);
    const next = applyPatch(t, {
      ...emptyPatch,
      added: [{ parent_scope_id: "s", file: file("2", "b") }],
    });
    expect(next.scopes[0].files.map((f) => f.id)).toEqual(["1", "2"]);
  });

  it("applies reordered last so additions land first", () => {
    const t = tree([scope("s", [file("1", "a")])]);
    const next = applyPatch(t, {
      ...emptyPatch,
      added: [{ parent_scope_id: "s", file: file("2", "b") }],
      reordered: [{ parent_scope_id: "s", child_ids: ["2", "1"] }],
    });
    expect(next.scopes[0].files.map((f) => f.id)).toEqual(["2", "1"]);
  });

  it("tolerates reordered child_ids that reference removed files", () => {
    const t = tree([scope("s", [file("1", "a"), file("2", "b")])]);
    const next = applyPatch(t, {
      ...emptyPatch,
      removed: ["2"],
      reordered: [{ parent_scope_id: "s", child_ids: ["2", "1"] }],
    });
    // "2" is filtered out; "1" survives.
    expect(next.scopes[0].files.map((f) => f.id)).toEqual(["1"]);
  });
});

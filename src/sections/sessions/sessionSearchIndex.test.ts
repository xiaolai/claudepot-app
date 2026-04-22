import { describe, expect, it } from "vitest";

import type { SessionRow } from "../../types";
import {
  buildSessionSearchHaystack,
  matchesQuery,
} from "./sessionSearchIndex";

function mk(id: string, mods: Partial<SessionRow> = {}): SessionRow {
  return {
    session_id: id,
    slug: `-${id}`,
    file_path: `/tmp/${id}.jsonl`,
    file_size_bytes: 0,
    last_modified_ms: 0,
    project_path: `/repo/${id}`,
    project_from_transcript: true,
    first_ts: null,
    last_ts: null,
    event_count: 0,
    message_count: 0,
    user_message_count: 0,
    assistant_message_count: 0,
    first_user_prompt: `prompt for ${id}`,
    models: [],
    tokens: { input: 0, output: 0, cache_creation: 0, cache_read: 0, total: 0 },
    git_branch: null,
    cc_version: null,
    display_slug: null,
    has_error: false,
    is_sidechain: false,
    ...mods,
  };
}

describe("buildSessionSearchHaystack", () => {
  it("returns undefined for unknown file_path", () => {
    const hay = buildSessionSearchHaystack([mk("a")]);
    expect(hay.get("/tmp/missing.jsonl")).toBeUndefined();
  });

  it("indexes by file_path", () => {
    const hay = buildSessionSearchHaystack([mk("a")]);
    expect(hay.get("/tmp/a.jsonl")).toBeDefined();
  });

  it("lowercases every field", () => {
    const hay = buildSessionSearchHaystack([
      mk("alpha", {
        project_path: "/REPO/CamelCase",
        first_user_prompt: "MixedCase Prompt",
        git_branch: "FeatureBranch",
        models: ["Claude-Sonnet-4-6"],
      }),
    ]);
    const s = hay.get("/tmp/alpha.jsonl") ?? "";
    expect(s).toContain("/repo/camelcase");
    expect(s).toContain("mixedcase prompt");
    expect(s).toContain("featurebranch");
    expect(s).toContain("claude-sonnet-4-6");
    // No uppercase letters survived.
    expect(s).toBe(s.toLowerCase());
  });

  it("absorbs null first_user_prompt and git_branch without throwing", () => {
    const hay = buildSessionSearchHaystack([
      mk("a", { first_user_prompt: null, git_branch: null }),
    ]);
    expect(hay.get("/tmp/a.jsonl")).toBeDefined();
  });

  it("absorbs an empty models array", () => {
    const hay = buildSessionSearchHaystack([mk("a", { models: [] })]);
    expect(hay.get("/tmp/a.jsonl")).toBeDefined();
  });

  it("absorbs a models field that arrived as null across the boundary", () => {
    // The TypeScript type promises string[] but the Tauri wire could
    // ship null on an older binary. The build path must not throw.
    const row = mk("a") as SessionRow & { models: string[] | null };
    row.models = null as unknown as string[];
    expect(() =>
      buildSessionSearchHaystack([row as SessionRow]),
    ).not.toThrow();
  });

  it("absorbs a null project_path", () => {
    const row = mk("a") as SessionRow & { project_path: string | null };
    row.project_path = null as unknown as string;
    expect(() =>
      buildSessionSearchHaystack([row as SessionRow]),
    ).not.toThrow();
    const hay = buildSessionSearchHaystack([row as SessionRow]);
    // The literal "null" should NOT appear — null is coerced to ""
    // by safeLower so it doesn't pollute matches.
    const s = hay.get("/tmp/a.jsonl") ?? "";
    expect(s).not.toContain("null");
  });

  it("returns an empty haystack for an empty sessions array", () => {
    const hay = buildSessionSearchHaystack([]);
    expect(hay.get("/tmp/anything.jsonl")).toBeUndefined();
  });

  it("uses a separator so a query straddling two fields does not match", () => {
    // session_id ends in "foo", project_path starts with "bar". A
    // naive concatenation (`parts.join("")`) would produce "foobar"
    // and match the query "foob". The separator prevents that.
    const hay = buildSessionSearchHaystack([
      mk("seek-foo", {
        project_path: "bar-haystack",
        first_user_prompt: null,
        git_branch: null,
        models: [],
      }),
    ]);
    const s = hay.get("/tmp/seek-foo.jsonl") ?? "";
    expect(s).toContain("seek-foo");
    expect(s).toContain("bar-haystack");
    // The seam-spanning substring must not exist.
    expect(s).not.toContain("foobar");
  });
});

describe("matchesQuery", () => {
  const sessions = [
    mk("alpha-001", {
      project_path: "/repo/auth",
      first_user_prompt: "discuss login flow",
      git_branch: "feature/auth",
      models: ["claude-sonnet-4-6"],
    }),
    mk("beta-002", {
      project_path: "/repo/payments",
      first_user_prompt: "refund the charge",
      git_branch: "main",
      models: ["claude-haiku-4-5"],
    }),
  ];
  const hay = buildSessionSearchHaystack(sessions);

  it("returns true for the empty query", () => {
    expect(matchesQuery(sessions[0], hay, "")).toBe(true);
    expect(matchesQuery(sessions[1], hay, "")).toBe(true);
  });

  it("matches a session_id prefix", () => {
    expect(matchesQuery(sessions[0], hay, "alpha")).toBe(true);
    expect(matchesQuery(sessions[1], hay, "alpha")).toBe(false);
  });

  it("matches a substring inside the project path", () => {
    expect(matchesQuery(sessions[0], hay, "auth")).toBe(true);
    expect(matchesQuery(sessions[1], hay, "auth")).toBe(false);
  });

  it("matches a substring inside the first prompt", () => {
    expect(matchesQuery(sessions[1], hay, "refund")).toBe(true);
    expect(matchesQuery(sessions[0], hay, "refund")).toBe(false);
  });

  it("matches a substring inside the git branch", () => {
    expect(matchesQuery(sessions[0], hay, "feature/")).toBe(true);
  });

  it("matches a substring inside a model id", () => {
    expect(matchesQuery(sessions[0], hay, "sonnet")).toBe(true);
    expect(matchesQuery(sessions[1], hay, "haiku")).toBe(true);
  });

  it("returns false when the file_path is not in the haystack", () => {
    const orphan = mk("orphan", { file_path: "/tmp/orphan.jsonl" });
    // Not registered in the haystack — only the session_id prefix
    // path can still match.
    expect(matchesQuery(orphan, hay, "orph")).toBe(true);
    expect(matchesQuery(orphan, hay, "anything-else")).toBe(false);
  });

  it("absorbs a null session_id without throwing", () => {
    const row = mk("a") as SessionRow & { session_id: string | null };
    row.session_id = null as unknown as string;
    const safeHay = buildSessionSearchHaystack([row as SessionRow]);
    expect(() => matchesQuery(row as SessionRow, safeHay, "x")).not.toThrow();
  });
});

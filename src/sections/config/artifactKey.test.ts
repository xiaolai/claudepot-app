import { describe, expect, it } from "vitest";
import type { ConfigFileNodeDto } from "../../types";
import { artifactKeyForFile, pluginIdFromPath } from "./artifactKey";

function file(partial: Partial<ConfigFileNodeDto>): ConfigFileNodeDto {
  return {
    id: "x",
    kind: "skill",
    abs_path: "",
    display_path: "",
    size_bytes: 0,
    mtime_unix_ns: 0,
    summary_title: null,
    summary_description: null,
    issues: [],
    included_by: null,
    include_depth: 0,
    ...partial,
  };
}

describe("pluginIdFromPath", () => {
  it("extracts plugin id from cache path", () => {
    expect(
      pluginIdFromPath(
        "/Users/me/.claude/plugins/cache/xiaolai/codex-toolkit/0.8.2/skills/audit/SKILL.md",
      ),
    ).toBe("codex-toolkit");
  });

  it("returns null for paths outside the cache", () => {
    expect(pluginIdFromPath("/Users/me/.claude/skills/foo/SKILL.md")).toBeNull();
  });

  it("returns null when path lacks the plugin segment", () => {
    expect(pluginIdFromPath("/Users/me/.claude/plugins/cache/")).toBeNull();
  });
});

describe("artifactKeyForFile — user artifacts", () => {
  it("user skill (dir form) becomes userSettings:<name>", () => {
    const r = artifactKeyForFile(
      file({
        kind: "skill",
        abs_path: "/Users/me/.claude/skills/hands-off/SKILL.md",
      }),
    );
    expect(r).toEqual({
      kind: "skill",
      artifactKey: "userSettings:hands-off",
      pluginId: null,
    });
  });

  it("user agent becomes the bare stem", () => {
    const r = artifactKeyForFile(
      file({
        kind: "agent",
        abs_path: "/Users/me/.claude/agents/Explore.md",
      }),
    );
    expect(r).toEqual({
      kind: "agent",
      artifactKey: "Explore",
      pluginId: null,
    });
  });

  it("user command becomes /<stem>", () => {
    const r = artifactKeyForFile(
      file({
        kind: "command",
        abs_path: "/Users/me/.claude/commands/audit.md",
      }),
    );
    expect(r).toEqual({
      kind: "command",
      artifactKey: "/audit",
      pluginId: null,
    });
  });
});

describe("artifactKeyForFile — plugin artifacts", () => {
  it("plugin skill becomes plugin:<id>:<name>", () => {
    const r = artifactKeyForFile(
      file({
        kind: "skill",
        abs_path:
          "/Users/me/.claude/plugins/cache/xiaolai/codex-toolkit/0.8.2/skills/audit-fix/SKILL.md",
      }),
    );
    expect(r).toEqual({
      kind: "skill",
      artifactKey: "plugin:codex-toolkit:audit-fix",
      pluginId: "codex-toolkit",
    });
  });

  it("plugin agent becomes <id>:<name>", () => {
    const r = artifactKeyForFile(
      file({
        kind: "agent",
        abs_path:
          "/Users/me/.claude/plugins/cache/xiaolai/loc-guardian/0.1.0/agents/counter.md",
      }),
    );
    expect(r).toEqual({
      kind: "agent",
      artifactKey: "loc-guardian:counter",
      pluginId: "loc-guardian",
    });
  });

  it("plugin command becomes /<id>:<name>", () => {
    const r = artifactKeyForFile(
      file({
        kind: "command",
        abs_path:
          "/Users/me/.claude/plugins/cache/xiaolai/loc-guardian/0.1.0/commands/scan.md",
      }),
    );
    expect(r).toEqual({
      kind: "command",
      artifactKey: "/loc-guardian:scan",
      pluginId: "loc-guardian",
    });
  });
});

describe("artifactKeyForFile — project scope", () => {
  it("project-scope skill becomes projectSettings:<name> when projectRoot is provided", () => {
    const r = artifactKeyForFile(
      file({
        kind: "skill",
        abs_path: "/repo/.claude/skills/lint/SKILL.md",
      }),
      "/repo",
    );
    expect(r).toEqual({
      kind: "skill",
      artifactKey: "projectSettings:lint",
      pluginId: null,
    });
  });

  it("falls back to userSettings: when projectRoot is null", () => {
    const r = artifactKeyForFile(
      file({
        kind: "skill",
        abs_path: "/repo/.claude/skills/lint/SKILL.md",
      }),
      null,
    );
    // Without project context we can't disambiguate — userSettings is
    // the legacy default.
    expect(r?.artifactKey).toBe("userSettings:lint");
  });

  it("plugin skill ignores projectRoot", () => {
    const r = artifactKeyForFile(
      file({
        kind: "skill",
        abs_path:
          "/repo/.claude/plugins/cache/owner/my-plugin/0.1.0/skills/x/SKILL.md",
      }),
      "/repo",
    );
    expect(r?.artifactKey).toBe("plugin:my-plugin:x");
  });

  it("user skill outside the project's .claude is NOT marked project scope", () => {
    // Regression: a user skill at ~/.claude/skills/X must NOT be
    // labeled projectSettings just because the project happens to
    // sit under the same parent. The check requires the file to
    // be under `<projectRoot>/.claude/`, not merely under projectRoot.
    const r = artifactKeyForFile(
      file({
        kind: "skill",
        abs_path: "/Users/joker/.claude/skills/hands-off/SKILL.md",
      }),
      "/Users/joker/code/myrepo", // a real project, NOT home
    );
    expect(r?.artifactKey).toBe("userSettings:hands-off");
  });

  it("contract: when caller passes home as projectRoot, ~/.claude/skills/* WILL be labeled project scope", () => {
    // Documents the boundary of the helper's responsibility:
    // artifactKeyForFile trusts its caller. If callers pass the user
    // home as projectRoot, we treat `~/.claude/skills/*` as project
    // scope by design — the caller (ConfigSection) is the layer that
    // knows whether it's in global-only mode and must pass null in
    // that case (it does — see useArtifactUsage's `globalOnly` arg).
    const r = artifactKeyForFile(
      file({
        kind: "skill",
        abs_path: "/Users/joker/.claude/skills/hands-off/SKILL.md",
      }),
      "/Users/joker",
    );
    expect(r?.artifactKey).toBe("projectSettings:hands-off");
  });
});

describe("artifactKeyForFile — non-trackable kinds", () => {
  it("returns null for hooks", () => {
    expect(
      artifactKeyForFile(
        file({ kind: "hook", abs_path: "/Users/me/.claude/settings.json" }),
      ),
    ).toBeNull();
  });

  it("returns null for plugins", () => {
    expect(
      artifactKeyForFile(
        file({
          kind: "plugin",
          abs_path:
            "/Users/me/.claude/plugins/cache/xiaolai/codex-toolkit/0.8.2/.claude-plugin/plugin.json",
        }),
      ),
    ).toBeNull();
  });

  it("returns null for claude_md", () => {
    expect(
      artifactKeyForFile(
        file({ kind: "claude_md", abs_path: "/repo/CLAUDE.md" }),
      ),
    ).toBeNull();
  });
});

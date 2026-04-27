// Map a Config FileNode to the canonical (kind, artifact_key) pair
// that `artifact_usage` records in `sessions.db`.
//
// Verified against the on-disk layout of ~/.claude/{skills,agents,
// commands} and ~/.claude/plugins/cache/<owner>/<plugin>/<hash>/...
// Cross-checked against actual `invoked_skills.skills[].path` values
// pulled from real session JSONL.
//
// Hooks and plugins are intentionally NOT trackable here:
//   - Hooks are 1:N with files (one settings.json declares many) —
//     surfaced separately in HooksRenderer.
//   - Plugins are aggregates — the right query is per-plugin rollup,
//     not per-file lookup.

import type { ArtifactUsageKind, ConfigFileNodeDto } from "../../types";

export interface ArtifactKeyResolution {
  kind: ArtifactUsageKind;
  artifactKey: string;
  /** Plugin id when derivable from the file path. Display-only. */
  pluginId: string | null;
}

const TRACKABLE_KINDS = new Set<string>(["skill", "agent", "command"]);

/**
 * Derive the canonical artifact key the JSONL extractor would have
 * written for a given Config FileNode. Returns `null` when the file
 * isn't one of the four trackable kinds, or when the path doesn't
 * fit a recognized layout (the lookup will simply miss in that case).
 *
 * Pass `projectRoot` (e.g. `tree.cwd`) so a project-scope skill is
 * keyed `projectSettings:<name>` rather than `userSettings:<name>`.
 * Without it, project skills get attributed to the user-scope namespace
 * and never match any recorded usage event.
 */
export function artifactKeyForFile(
  file: ConfigFileNodeDto,
  projectRoot?: string | null,
): ArtifactKeyResolution | null {
  if (!TRACKABLE_KINDS.has(file.kind)) return null;

  const path = file.abs_path;
  const pluginId = pluginIdFromPath(path);
  // Project skills live at `<projectRoot>/.claude/skills/<name>/SKILL.md`
  // (or the equivalent for agents/commands). Match the `<projectRoot>/.claude/`
  // segment specifically — `path.startsWith(projectRoot)` alone is too loose
  // (in global-only mode the backend reports project_root as the user home,
  // and `~/.claude/skills/*` would otherwise be mis-keyed as project scope).
  const isProjectScope =
    !pluginId &&
    projectRoot != null &&
    projectRoot.length > 0 &&
    path.startsWith(`${projectRoot}/.claude/`);

  switch (file.kind) {
    case "skill": {
      // Skill name is the parent directory of SKILL.md, OR the file
      // stem when CC uses the bare-file form.
      const name = skillNameFromPath(path);
      if (!name) return null;
      let artifactKey: string;
      if (pluginId) {
        artifactKey = `plugin:${pluginId}:${name}`;
      } else if (isProjectScope) {
        artifactKey = `projectSettings:${name}`;
      } else {
        artifactKey = `userSettings:${name}`;
      }
      return { kind: "skill", artifactKey, pluginId };
    }
    case "agent": {
      const name = stemFromMd(path);
      if (!name) return null;
      const artifactKey = pluginId ? `${pluginId}:${name}` : name;
      return { kind: "agent", artifactKey, pluginId };
    }
    case "command": {
      const name = stemFromMd(path);
      if (!name) return null;
      const artifactKey = pluginId ? `/${pluginId}:${name}` : `/${name}`;
      return { kind: "command", artifactKey, pluginId };
    }
    default:
      return null;
  }
}

/** Pull the plugin id from `~/.claude/plugins/cache/<owner>/<plugin>/...`. */
export function pluginIdFromPath(path: string): string | null {
  const i = path.indexOf("plugins/cache/");
  if (i < 0) return null;
  const after = path.slice(i + "plugins/cache/".length);
  const parts = after.split("/");
  if (parts.length < 2) return null;
  // owner = parts[0], plugin = parts[1]
  const plugin = parts[1];
  return plugin && plugin.length > 0 ? plugin : null;
}

function stemFromMd(path: string): string | null {
  const last = path.split("/").pop();
  if (!last) return null;
  if (!last.endsWith(".md")) return null;
  return last.slice(0, -3);
}

function skillNameFromPath(path: string): string | null {
  // Two layouts CC supports:
  //   <root>/skills/<name>/SKILL.md   (canonical, dir-form)
  //   <root>/skills/<name>.md         (rarely seen, file-form)
  const parts = path.split("/");
  const last = parts[parts.length - 1];
  const parent = parts[parts.length - 2];
  if (last === "SKILL.md" && parent) return parent;
  if (last && last.endsWith(".md")) return last.slice(0, -3);
  return null;
}

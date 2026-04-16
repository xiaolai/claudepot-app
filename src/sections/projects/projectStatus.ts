import type { ProjectInfo } from "../../types";

/**
 * Four-state classification for a project row. Drives both the
 * sidebar badge and the sidebar filter chip.
 *
 *   alive       — source path exists and is reachable; nothing to do.
 *   orphan      — source confirmed gone on a reachable filesystem;
 *                 safe to clean.
 *   unreachable — source can't be stat'd (unmounted volume,
 *                 permission denied). NOT cleanable; user must
 *                 re-mount or fix perms before Claudepot can judge.
 *   empty       — CC project dir itself is empty (no sessions, no
 *                 memory, <4 KiB on disk). Reclaimable regardless
 *                 of source status; typically a stub left behind
 *                 when CC was aborted before writing anything.
 *
 * The priority here matters: `empty` wins over `unreachable` because
 * an empty dir has nothing to lose, and `unreachable` wins over
 * `orphan` because unreachable projects are explicitly NOT flagged
 * by the backend as `is_orphan`.
 */
export type ProjectStatus = "alive" | "orphan" | "unreachable" | "empty";

export function classifyProject(p: ProjectInfo): ProjectStatus {
  if (p.is_empty) return "empty";
  if (!p.is_reachable) return "unreachable";
  if (p.is_orphan) return "orphan";
  return "alive";
}

// Memory-health audit — frontend binding for the
// `memory_health_get` Tauri command. Static-analysis metrics for
// the global CLAUDE.md and MEMORY.md files; pure read.

import { invoke } from "@tauri-apps/api/core";
import type { MemoryHealthReport } from "../types";

export const memoryHealthApi = {
  /**
   * Audit `~/.claude/CLAUDE.md` and `~/.claude/memory/MEMORY.md` and
   * return per-file metrics. Missing files are reported inline
   * (`missing: true`) — only non-NotFound I/O errors throw.
   */
  memoryHealthGet: () => invoke<MemoryHealthReport>("memory_health_get"),
};

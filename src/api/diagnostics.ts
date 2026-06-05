import { invoke } from "@tauri-apps/api/core";

/**
 * Diagnostic-surface API. Currently a single entry — opens the
 * rolling log directory in the OS file manager — but kept as its
 * own shard so future diagnostic commands (panic-history readback,
 * trace level toggle, log tail) land in an obvious place rather
 * than bloating account.ts / settings.ts.
 */
export const diagnosticsApi = {
  revealLogsDir: () => invoke<void>("logs_dir_reveal"),
};

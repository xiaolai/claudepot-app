// Boards API — the top-level Boards section.
//
// Change detection is a `data_version` poll, not a Tauri event. The
// store is written by *other processes* (the CLI, a scheduled agent, a
// script), so there is no in-process emitter to subscribe to. See
// `claudepot_core::board::monitor` for why a file watcher is the wrong
// tool here.

import { invoke } from "@tauri-apps/api/core";
import type { BoardDetail, BoardSummary } from "../types";

// Methods are prefixed because `api` is one flat merged surface — a
// bare `list` would collide with every other domain's.
export const boardApi = {
  boardList: (): Promise<BoardSummary[]> => invoke("board_list"),

  boardDetail: (boardId: string): Promise<BoardDetail> =>
    invoke("board_detail", { boardId }),

  /**
   * `PRAGMA data_version`. Changes when another connection commits.
   *
   * Carries no diff — when it moves, re-fetch a snapshot. Offering a
   * delta would imply a precision the mechanism does not have.
   */
  boardDataVersion: (): Promise<number> => invoke("board_data_version"),

  /** Explicit deletion. Nothing prunes boards automatically. */
  boardRemove: (boardId: string): Promise<void> =>
    invoke("board_delete", { boardId }),

  boardExport: (boardId: string, path: string): Promise<string> =>
    invoke("board_export", { boardId, path }),
};

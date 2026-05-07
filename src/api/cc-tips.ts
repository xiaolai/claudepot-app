// Frontend bindings for the CC tips ledger.
//
// Three commands match `src-tauri/src/commands/cc_tips.rs`:
//   - cc_tips_list — render the current tips view
//   - cc_tips_refresh — force re-extraction from the CC binary
//   - cc_tips_record_view — append a snapshot if the last is >1h old

import { invoke } from "@tauri-apps/api/core";
import type { TipsRefreshResult, TipsRender } from "../types/cc-tips";

export const ccTipsApi = {
  ccTipsList: () => invoke<TipsRender>("cc_tips_list"),
  ccTipsRefresh: () => invoke<TipsRefreshResult>("cc_tips_refresh"),
  ccTipsRecordView: () => invoke<boolean>("cc_tips_record_view"),
};

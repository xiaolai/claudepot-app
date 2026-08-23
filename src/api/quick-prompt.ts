import { invoke } from "@tauri-apps/api/core";

import type { QuickPrompt } from "../types";

/**
 * The chips above the remote panel's composer.
 *
 * `save` replaces the whole list — order is part of the data, and every
 * edit the pane offers is "here is the new list". An empty array is a
 * legitimate save: it is how the last chip is deleted.
 */
export const quickPromptApi = {
  list: () => invoke<QuickPrompt[]>("quick_prompt_list"),
  save: (prompts: QuickPrompt[]) =>
    invoke<QuickPrompt[]>("quick_prompt_save", { prompts }),
  defaults: () => invoke<QuickPrompt[]>("quick_prompt_defaults"),
};

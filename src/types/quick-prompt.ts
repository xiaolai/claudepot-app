/** One quick prompt: a short name you tap, a longer text that is sent. */
export interface QuickPrompt {
  /** Stable across renames, so an edit is not a delete plus an insert. */
  id: string;
  name: string;
  text: string;
}

/** Mirrors `claudepot_core::quick_prompt`'s caps. */
export const QUICK_PROMPT_MAX_NAME = 32;
export const QUICK_PROMPT_MAX_TEXT = 4000;
export const QUICK_PROMPT_MAX_COUNT = 24;

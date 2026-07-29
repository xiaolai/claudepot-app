import { NF, type NfIcon } from "../../icons";

/**
 * Global-section tab metadata. Same rationale as
 * `sections/settings/panes.ts`: the ⌘K palette needs these labels to
 * build deep links, and importing `GlobalSection` for them would pull
 * its lazy chunk into the main bundle.
 *
 * The order here is the rendered tab-bar order — `GlobalSection` maps
 * this array to build its tablist, so the two cannot disagree. They
 * did on the first cut: this table listed Updates before Tips while
 * the hand-written buttons rendered Tips before Updates.
 */
export interface GlobalTabDef {
  id: string;
  label: string;
  glyph: NfIcon;
  keywords?: readonly string[];
}

export const GLOBAL_TABS = [
  { id: "config", label: "Config", glyph: NF.fileCode,
    keywords: ["settings.json", "env variables", "plugins", "policy"] },
  { id: "memory", label: "Memory", glyph: NF.book,
    keywords: ["CLAUDE.md", "memory files"] },
  { id: "tips", label: "Tips", glyph: NF.info, keywords: ["hints"] },
  { id: "updates", label: "Updates", glyph: NF.download,
    keywords: ["upgrade", "channel", "version"] },
] as const satisfies readonly GlobalTabDef[];

export type GlobalTabId = (typeof GLOBAL_TABS)[number]["id"];

export function isGlobalTabId(v: string): v is GlobalTabId {
  return GLOBAL_TABS.some((t) => t.id === v);
}

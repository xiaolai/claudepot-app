import { NF, type NfIcon } from "../../icons";

/**
 * Global-section tab metadata. Same rationale as
 * `sections/settings/panes.ts`: the ⌘K palette needs these labels to
 * build deep links, and importing `GlobalSection` for them would pull
 * its lazy chunk into the main bundle.
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
  { id: "updates", label: "Updates", glyph: NF.download,
    keywords: ["upgrade", "channel", "version"] },
  { id: "tips", label: "Tips", glyph: NF.info, keywords: ["hints"] },
] as const satisfies readonly GlobalTabDef[];

export type GlobalTabId = (typeof GLOBAL_TABS)[number]["id"];

export function isGlobalTabId(v: string): v is GlobalTabId {
  return GLOBAL_TABS.some((t) => t.id === v);
}

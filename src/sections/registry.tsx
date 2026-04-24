import { NF } from "../icons";
import type { NfIcon } from "../icons";

/**
 * Primary-nav section metadata. The shell's Sidebar renders these in
 * order; ⌘1..⌘N maps to the first N entries. Keep `id` stable — it's
 * the localStorage key for the active section and for per-section
 * sub-routes.
 *
 * The `glyph` field is an NF codepoint (see `src/icons.ts`). Rendered
 * by the new paper-mono primitives via `<Glyph g={section.glyph} />`;
 * no SVG icons allowed.
 */
export interface SectionDef {
  id: string;
  label: string;
  glyph: NfIcon;
}

export const sections: readonly SectionDef[] = [
  { id: "accounts", label: "Accounts", glyph: NF.users },
  { id: "projects", label: "Projects", glyph: NF.folder },
  { id: "activities", label: "Activities", glyph: NF.chatAlt },
  { id: "global",   label: "Global",   glyph: NF.globe },
  { id: "keys",     label: "Keys",     glyph: NF.key     },
  { id: "settings", label: "Settings", glyph: NF.sliders },
];

export const sectionIds = sections.map((s) => s.id);

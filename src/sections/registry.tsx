import type { ReactNode } from "react";
import { Icon } from "../components/Icon";

/**
 * Section metadata. The shell renders the matching body itself — the
 * registry only carries the id, label, and rail icon so sections can
 * expose whatever props shape they need without a common interface.
 *
 * Sections are ordered top-down; ⌘1..⌘N maps to the first N entries.
 * Keep `id` stable — it's the localStorage key for the active section
 * and for per-section sub-routes.
 */
export interface SectionDef {
  id: string;
  label: string;
  icon: ReactNode;
}

export const sections: readonly SectionDef[] = [
  { id: "accounts", label: "Accounts", icon: <Icon name="user" size={18} /> },
  { id: "projects", label: "Projects", icon: <Icon name="folder" size={18} /> },
  { id: "settings", label: "Settings", icon: <Icon name="settings" size={18} /> },
];

export const sectionIds = sections.map((s) => s.id);

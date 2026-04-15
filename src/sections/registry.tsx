import type { ReactNode } from "react";
import { Folder, User } from "@phosphor-icons/react";

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
  { id: "accounts", label: "Accounts", icon: <User /> },
  { id: "projects", label: "Projects", icon: <Folder /> },
];

export const sectionIds = sections.map((s) => s.id);

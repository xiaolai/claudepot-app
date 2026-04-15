import type { ComponentType, ReactNode } from "react";
import { User } from "@phosphor-icons/react";
import { AccountsSection } from "./AccountsSection";

/**
 * Every top-level section the rail can navigate to.
 *
 * Sections are ordered top-down; ⌘1..⌘N maps to the first N entries.
 * New sections append at the bottom. Keep `id` stable — it's used as
 * the localStorage key for the active-section pointer.
 */
export interface SectionDef {
  id: string;
  label: string;
  icon: ReactNode;
  Component: ComponentType;
}

export const sections: readonly SectionDef[] = [
  {
    id: "accounts",
    label: "Accounts",
    icon: <User />,
    Component: AccountsSection,
  },
];

export const sectionIds = sections.map((s) => s.id);

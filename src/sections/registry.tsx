import { lazy, type ComponentType, type ReactElement } from "react";
import { NF } from "../icons";
import type { NfIcon } from "../icons";
import {
  SECTION_ACTIVE_KEY,
  SECTION_START_KEY,
} from "../lib/storageKeys";
import { requestIdle } from "../lib/idle";
// AccountsSection is the only eager section — it's the default
// landing tab and ships in the main chunk so the shell lands with
// real content in one round-trip. Every other section code-splits
// via the `loader` fields below. This static import is free: App.tsx
// (the only other registry importer besides AppSidebar) already
// bundles AccountsSection into the main chunk.
import { AccountsSection } from "./AccountsSection";

/**
 * Primary-nav section registry — the single source of truth for the
 * shell's sections. The Sidebar renders entries in order; ⌘1..⌘N maps
 * to the first N entries; App.tsx derives chunk preloading and the
 * section render switch from this list. Keep `id` stable — it's the
 * localStorage key for the active section and for per-section
 * sub-routes (see `src/lib/storageKeys.ts`).
 *
 * The `glyph` field is a semantic icon name from the `NF` map in
 * `src/icons.ts`, which resolves to a Lucide SVG component rendered
 * via `<Glyph g={section.glyph} />` (see `.claude/rules/design.md` —
 * Lucide SVG only; the `NF.*` call shape is kept from the older Nerd
 * Font pipeline).
 */

/** Shell-owned state and callbacks a section may need. App.tsx builds
 *  one of these per render; each entry's `render` picks the subset
 *  its component accepts. */
export interface SectionHostProps {
  subRoute: string | null;
  onSubRouteChange: (next: string | null) => void;
  /** Shell-level section navigation (useSection's setSection). */
  onNavigate: (id: string, subRoute?: string | null) => void;
  /** Cross-section deep-link state — consumed by ProjectsSection. */
  pendingProjectPath: string | null;
  pendingSessionPath: string | null;
  onPendingConsumed: () => void;
}

export interface SectionDef {
  id: string;
  label: string;
  glyph: NfIcon;
  /**
   * Shared chunk factory for lazy sections — handed to `React.lazy`
   * below AND fired early by the preload helpers. Sharing one
   * factory ensures the bundler caches the module once: lazy's first
   * invocation and the preload call resolve to the same promise.
   * Absent only for the eager AccountsSection.
   */
  loader?: () => Promise<{ default: ComponentType<never> }>;
  /** Render the section body. The ErrorBoundary wrapper (keyed on
   *  `id`, labeled with `label`) is applied by App.tsx. */
  render: (props: SectionHostProps) => ReactElement;
}

// Named import promises so React.lazy and the preload helpers share
// one factory per chunk (see `SectionDef.loader`).
const importProjects = () =>
  import("./ProjectsSection").then((m) => ({ default: m.ProjectsSection }));
const importSettings = () =>
  import("./SettingsSection").then((m) => ({ default: m.SettingsSection }));
const importEvents = () =>
  import("./EventsSection").then((m) => ({ default: m.EventsSection }));
const importKeys = () =>
  import("./KeysSection").then((m) => ({ default: m.KeysSection }));
const importConfig = () =>
  import("./ConfigSection").then((m) => ({ default: m.ConfigSection }));
const importGlobal = () =>
  import("./GlobalSection").then((m) => ({ default: m.GlobalSection }));
const importThirdParty = () =>
  import("./ThirdPartySection").then((m) => ({ default: m.ThirdPartySection }));
const importAgents = () =>
  import("./AgentsSection").then((m) => ({ default: m.AgentsSection }));
const importSharedMemory = () =>
  import("./SharedMemorySection").then((m) => ({
    default: m.SharedMemorySection,
  }));
// ConfigSection isn't rendered at the top level anymore — it lives
// inside GlobalSection and the Projects shell's Config tab. The
// import* chunk keys off GlobalSection's own import, and
// ProjectsSection statically imports ConfigSection, so we don't need
// to warm it separately. Keep the factory reference so tree-shaking
// can't drop the export accidentally.
void importConfig;

const ProjectsSection = lazy(importProjects);
const SettingsSection = lazy(importSettings);
const EventsSection = lazy(importEvents);
const KeysSection = lazy(importKeys);
const GlobalSection = lazy(importGlobal);
const ThirdPartySection = lazy(importThirdParty);
const AgentsSection = lazy(importAgents);
const SharedMemorySection = lazy(importSharedMemory);

export const sections: readonly SectionDef[] = [
  {
    id: "accounts",
    label: "Accounts",
    glyph: NF.users,
    render: (p) => <AccountsSection onNavigate={p.onNavigate} />,
  },
  {
    id: "events",
    label: "Activities",
    glyph: NF.dashboard,
    loader: importEvents,
    render: () => <EventsSection />,
  },
  {
    id: "projects",
    label: "Projects",
    glyph: NF.folder,
    loader: importProjects,
    render: (p) => (
      <ProjectsSection
        subRoute={p.subRoute}
        onSubRouteChange={p.onSubRouteChange}
        pendingProjectPath={p.pendingProjectPath}
        pendingSessionPath={p.pendingSessionPath}
        onPendingConsumed={p.onPendingConsumed}
      />
    ),
  },
  {
    id: "shared-memory",
    label: "Memory",
    glyph: NF.book,
    loader: importSharedMemory,
    render: () => <SharedMemorySection />,
  },
  {
    id: "keys",
    label: "Keys",
    glyph: NF.key,
    loader: importKeys,
    render: () => <KeysSection />,
  },
  // id kept as "third-party" for localStorage compatibility
  {
    id: "third-party",
    label: "Providers",
    glyph: NF.cpu,
    loader: importThirdParty,
    render: () => <ThirdPartySection />,
  },
  // id kept as "automations" for localStorage compatibility
  {
    id: "automations",
    label: "Agents",
    glyph: NF.clock,
    loader: importAgents,
    render: () => <AgentsSection />,
  },
  {
    id: "global",
    label: "Global",
    glyph: NF.globe,
    loader: importGlobal,
    render: (p) => (
      <GlobalSection
        subRoute={p.subRoute}
        onSubRouteChange={p.onSubRouteChange}
      />
    ),
  },
  {
    id: "settings",
    label: "Settings",
    glyph: NF.sliders,
    loader: importSettings,
    render: () => <SettingsSection />,
  },
];

export const sectionIds = sections.map((s) => s.id);

/** Kick off the saved section's chunk in parallel with first paint.
 *  Reads the same keys useSection resolves on its post-paint tick;
 *  the eager Accounts entry has no loader, so it's a no-op there. */
export function preloadSavedSection(): void {
  try {
    const id =
      localStorage.getItem(SECTION_START_KEY) ||
      localStorage.getItem(SECTION_ACTIVE_KEY);
    void sections.find((s) => s.id === id)?.loader?.();
  } catch {
    // localStorage unavailable — nothing to preload.
  }
}

/**
 * Warm every remaining section chunk during browser idle time so that
 * subsequent in-app navigations never hit a Suspense fallback flash.
 *
 * Each `import()` is fired sequentially on its own idle slice so the
 * fetches don't stampede the initial paint or the Tauri bridge — the
 * first paint owns the foreground, then we trickle chunks into the
 * module cache while the user reads the Accounts list.
 *
 * `startTransition` in `useSection` already eliminates the blank
 * flash by keeping the previous section visible while the new chunk
 * resolves; this preload makes that wait imperceptible by ensuring
 * the chunk is already in the module cache when the click lands.
 */
export function preloadAllSections(): void {
  const factories = sections
    .filter((s) => s.loader !== undefined)
    .map((s) => s.loader!);

  const schedule = (i: number) => {
    if (i >= factories.length) return;
    requestIdle(() => {
      const next = factories[i];
      if (!next) {
        schedule(i + 1);
        return;
      }
      // Swallow errors — preload is best-effort. If the chunk is
      // really unreachable, the eventual click will surface the same
      // error through the normal Suspense path with the fallback the
      // user expects.
      next()
        .catch(() => undefined)
        .finally(() => schedule(i + 1));
    });
  };
  schedule(0);
}

import { useEffect } from "react";
import { sectionIds } from "../sections/registry";

/**
 * Window-event navigation bridges, extracted from AppShell:
 *
 * `claudepot:navigate-section` — cross-section navigation requests
 * via DOM CustomEvent. Lets a child section (e.g. EventsSection card
 * click) switch to another section AND seed the target session
 * without prop-drilling `setSection` or coupling component trees.
 * Payload shape: `{ id: string, sessionPath?: string,
 * projectPath?: string }`. When `sessionPath` is set, also seeds the
 * pending session path so the destination section opens the right
 * transcript on mount. Per-line scroll-to-byte-offset is Phase 6 —
 * landing on the right session is the MVP.
 *
 * `cp-goto-session` — bridge from the command palette's
 * cross-session search: when the user selects a session hit, stash
 * the target path and jump to Projects. `ProjectsSection` consumes
 * the pending path on mount, resolves the owning project from the
 * file's slug, and opens the transcript in its master-detail pane.
 * A previous implementation raced via `setTimeout(0)` and could
 * drop the selection on slow mounts.
 */
export function useNavigationBridges(args: {
  setSection: (id: string) => void;
  setPendingSessionPath: (path: string | null) => void;
  setPendingProjectPath: (path: string | null) => void;
}): void {
  const { setSection, setPendingSessionPath, setPendingProjectPath } = args;

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (
        e as CustomEvent<{
          id?: string;
          sessionPath?: string;
          projectPath?: string;
        }>
      ).detail;
      const id = detail?.id;
      if (id && sectionIds.includes(id)) {
        if (detail.sessionPath) {
          setPendingSessionPath(detail.sessionPath);
        }
        if (detail.projectPath) {
          setPendingProjectPath(detail.projectPath);
        }
        setSection(id);
      }
    };
    window.addEventListener("claudepot:navigate-section", handler);
    return () =>
      window.removeEventListener("claudepot:navigate-section", handler);
  }, [setSection, setPendingSessionPath, setPendingProjectPath]);

  useEffect(() => {
    function onGoto(ev: Event) {
      const detail = (ev as CustomEvent<{ filePath: string }>).detail;
      if (!detail?.filePath) return;
      // After the events-into-projects collapse, transcripts open
      // inside ProjectsSection's master-detail pane. ProjectsSection
      // derives the matching project from the file's slug when no
      // explicit projectPath is supplied — see its pending-consumer
      // effect.
      setPendingSessionPath(detail.filePath);
      setSection("projects");
    }
    window.addEventListener("cp-goto-session", onGoto);
    return () => window.removeEventListener("cp-goto-session", onGoto);
  }, [setSection, setPendingSessionPath]);
}

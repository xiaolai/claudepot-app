import { useEffect } from "react";
import { installTrafficLightSync } from "../lib/trafficLights";

/**
 * Sync the OS-placed traffic-light row to a CSS custom property so
 * the WindowChrome breadcrumb / ⌘K pill can pin themselves onto
 * the lights' actual centerline (which AppKit puts a few px below
 * the chrome's geometric center). See
 * `~/.claude/skills/tauri/SKILL.md` and `src/lib/trafficLights.ts`.
 */
export function useTrafficLightSync(): void {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void installTrafficLightSync().then((un) => {
      if (cancelled) {
        un();
        return;
      }
      unlisten = un;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
}

import { useEffect } from "react";
import { readDevMode, writeDevMode } from "./useDevMode";

/**
 * Shell-level keyboard shortcuts, extracted from AppShell:
 *
 *   - ⌘,  — open Settings
 *   - ⌘K  — open the command palette
 *   - ⌘/  — open the shortcuts reference
 *   - ⌃⌥⌘L — toggle developer mode (hidden deep-system toggle)
 *   - ⌘⇧L — focus the first SidebarLiveStrip row
 *
 * All skip editable focus so typing inside an input doesn't hijack
 * them (per `.claude/rules/design.md` → Shortcuts). ⌘1..⌘9 section
 * switching lives in `useSection`, not here.
 */
export function useShellShortcuts(args: {
  setSection: (id: string) => void;
  openPalette: () => void;
  openShortcuts: () => void;
  pushToast: (kind: "info" | "error", text: string) => void;
}): void {
  const { setSection, openPalette, openShortcuts, pushToast } = args;

  // ⌘, / ⌘K / ⌘/.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.altKey) return;
      const el = document.activeElement as HTMLElement | null;
      const tag = el?.tagName?.toLowerCase();
      const editable =
        tag === "input" || tag === "textarea" || el?.isContentEditable;

      if (e.key === "," && !e.shiftKey) {
        e.preventDefault();
        setSection("settings");
        return;
      }
      if ((e.key === "k" || e.key === "K") && !e.shiftKey) {
        if (editable) return;
        e.preventDefault();
        openPalette();
        return;
      }
      if (e.key === "/" && !e.shiftKey) {
        if (editable) return;
        e.preventDefault();
        openShortcuts();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setSection, openPalette, openShortcuts]);

  // ⌃⌥⌘L — toggle developer mode globally. The combo requires all
  // four modifiers (Cmd + Alt + Ctrl + L), which is effectively
  // unreachable by accident; matches macOS's own deep-system-toggle
  // convention (⌃⌥⌘8 inverts screen colors, etc.). The settings
  // surface no longer renders a visible toggle, so this is the only
  // entry point. A toast confirms the new state since the toggle
  // is otherwise invisible.
  useEffect(() => {
    const onDevKey = (e: KeyboardEvent) => {
      if (!e.metaKey || !e.ctrlKey || !e.altKey) return;
      if (e.key !== "l" && e.key !== "L") return;
      e.preventDefault();
      const next = !readDevMode();
      writeDevMode(next);
      pushToast("info", next ? "Developer mode on" : "Developer mode off");
    };
    window.addEventListener("keydown", onDevKey);
    return () => window.removeEventListener("keydown", onDevKey);
  }, [pushToast]);

  // ⌘⇧L — focus the first SidebarLiveStrip row. Light-weight
  // fallback until the Activity section lands (M4) and claims this
  // shortcut. Ignores editable focus so typing "L" in the command
  // palette isn't hijacked.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || !e.shiftKey || e.altKey) return;
      if (e.key !== "l" && e.key !== "L") return;
      const el = document.activeElement as HTMLElement | null;
      const tag = el?.tagName?.toLowerCase();
      if (tag === "input" || tag === "textarea" || el?.isContentEditable) {
        return;
      }
      e.preventDefault();
      // The strip renders with role=listbox; focus the first option.
      const firstRow = document.querySelector<HTMLButtonElement>(
        '[aria-label="Live Claude sessions"] [role="option"]',
      );
      firstRow?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

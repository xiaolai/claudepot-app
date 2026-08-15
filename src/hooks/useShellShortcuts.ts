import { useEffect } from "react";
import { readDevMode, writeDevMode } from "./useDevMode";
import { isShortcutContextBlocked } from "./useGlobalShortcuts";
import { i18n } from "../lib/i18n";
import { toggleSection } from "../lib/optionalSections";

/**
 * Shell-level keyboard shortcuts, extracted from AppShell:
 *
 *   - ⌘,  — open Settings
 *   - ⌘K  — open the command palette
 *   - ⌘/  — open the shortcuts reference
 *   - ⌃⌥⌘L — toggle developer mode (hidden deep-system toggle)
 *   - ⌃⌥⌘B — show/hide the Boards section
 *   - ⌘⇧L — focus the first SidebarLiveStrip row
 *
 * All except ⌃⌥⌘L go through `isShortcutContextBlocked()` — the
 * shared gate from `useGlobalShortcuts` — so they don't fire while an
 * input has focus OR while a modal is open, per
 * `.claude/rules/design.md` → Shortcuts. Each hook used to fork its
 * own weaker predicate: ⌘K checked only for editable focus, so it
 * opened the palette on top of an already-open dialog (two live focus
 * traps), and ⌘, checked nothing at all, so it navigated the section
 * out from under an open modal and fired mid-typing.
 *
 * ⌃⌥⌘L is deliberately ungated. It is a hidden diagnostic toggle with
 * no visible control anywhere, and its whole value is being reachable
 * when something is wrong — including from a stuck modal. Its
 * four-modifier combo makes accidental firing while typing a
 * non-issue.
 *
 * ⌘1..⌘9 section switching lives in `useSection`, not here.
 */
export function useShellShortcuts(args: {
  setSection: (id: string) => void;
  openPalette: () => void;
  openShortcuts: () => void;
  pushToast: (kind: "info" | "error", text: string) => void;
  /** Expand the sidebar. ⌘⇧L needs it: the live strip is not mounted
   *  while collapsed, so the shortcut had nothing to focus and did
   *  nothing at all in that state. */
  expandSidebar?: () => void;
}): void {
  const { setSection, openPalette, openShortcuts, pushToast, expandSidebar } = args;

  // ⌘, / ⌘K / ⌘/.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.altKey || e.shiftKey) return;
      if (isShortcutContextBlocked()) return;

      if (e.key === ",") {
        e.preventDefault();
        setSection("settings");
        return;
      }
      if (e.key === "k" || e.key === "K") {
        e.preventDefault();
        openPalette();
        return;
      }
      if (e.key === "/") {
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
      pushToast(
        "info",
        i18n.t(next ? "shortcuts.devModeOn" : "shortcuts.devModeOff"),
      );
    };
    window.addEventListener("keydown", onDevKey);
    return () => window.removeEventListener("keydown", onDevKey);
  }, [pushToast]);

  // ⌃⌥⌘B — show or hide the Boards section.
  //
  // Boards is on trial, so it ships off and this is the fast way to
  // flip it while evaluating. Unlike ⌃⌥⌘L this is NOT a hidden
  // toggle — Settings → General carries the same switch — so it is
  // gated by `isShortcutContextBlocked()` like every ordinary
  // shortcut. `design.md` requires a written justification for each
  // ungated exception, and this one does not need the exception.
  //
  // Hiding the active section is reconciled centrally by `useSection`.
  useEffect(() => {
    const onBoardsKey = (e: KeyboardEvent) => {
      if (!e.metaKey || !e.ctrlKey || !e.altKey) return;
      if (e.key !== "b" && e.key !== "B") return;
      if (isShortcutContextBlocked()) return;
      e.preventDefault();
      const next = toggleSection("boards");
      // Only the ENABLE direction navigates. Hiding is reconciled by
      // `useSection`, which falls back whenever the active id leaves
      // the enabled list — reading the active section from
      // localStorage here duplicated that logic and could disagree
      // with React's actual state.
      if (next) setSection("boards");
      pushToast(
        "info",
        i18n.t(next ? "shortcuts.boardsShown" : "shortcuts.boardsHidden"),
      );
    };
    window.addEventListener("keydown", onBoardsKey);
    return () => window.removeEventListener("keydown", onBoardsKey);
  }, [pushToast, setSection]);

  // ⌘⇧L — focus the first SidebarLiveStrip row.
  //
  // The sidebar is now the canonical in-window live list, so this
  // shortcut has to be able to reach it from either state. The strip is
  // unmounted while the sidebar is collapsed (previews need width), so
  // the query below matched nothing and the shortcut silently did
  // nothing — expand first, then focus once the strip has mounted.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || !e.shiftKey || e.altKey) return;
      if (e.key !== "l" && e.key !== "L") return;
      if (isShortcutContextBlocked()) return;
      e.preventDefault();
      // The strip renders with role=listbox; focus the first option.
      // Keyed on `data-live-strip` rather than the strip's aria-label,
      // which is translated — matching on it worked in English and
      // matched nothing in every other locale.
      const focusFirst = () =>
        document
          .querySelector<HTMLButtonElement>("[data-live-strip] [role='option']")
          ?.focus();
      if (document.querySelector("[data-live-strip]")) {
        focusFirst();
        return;
      }
      expandSidebar?.();
      // One frame for React to commit the newly-mounted strip.
      requestAnimationFrame(focusFirst);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [expandSidebar]);
}

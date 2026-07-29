import { useEffect } from "react";

/**
 * Shared shortcut gates (design.md → Shortcuts: "Never fire while a
 * modal is open or an input is focused"). Exported so section-local
 * shortcut effects reuse the exact predicates instead of forking
 * them (audit 2026-07 F3).
 */
export function isEditable(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  return (el as HTMLElement).isContentEditable === true;
}

/** True when a modal is open or an editable surface has focus. */
export function isShortcutContextBlocked(): boolean {
  if (document.querySelector('[role="dialog"]')) return true;
  return isEditable(document.activeElement);
}

/**
 * Section-scoped keyboard shortcuts. Each handler is optional so the
 * consumer can opt out (⌘N is Accounts-specific today; ⌘R refreshes
 * any section that provides a handler).
 *
 * Gated by `isShortcutContextBlocked` — nothing fires while the user
 * is typing or while a modal is open.
 *
 * This hook used to accept `onPalette` and `onFilter` as well. No
 * caller ever passed either. `onPalette` was the more dangerous of
 * the two: ⌘K belongs to `useShellShortcuts`, so wiring it here would
 * have given one shortcut two owners — and this one predates the
 * modal gate, so the palette could still have opened over a dialog
 * through the second path.
 */
export function useGlobalShortcuts(handlers: {
  onRefresh?: () => void;
  onAdd?: () => void;
}): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.shiftKey || e.altKey) return;
      // Don't hijack typing (⌘A / ⌘F inside an input), and don't act
      // on the section behind an open modal — a ⌘R refresh or ⌘N add
      // fired from under a dialog leaves the user staring at a stale
      // modal over changed content.
      if (isShortcutContextBlocked()) return;
      if (e.key === "r" && handlers.onRefresh) {
        e.preventDefault();
        handlers.onRefresh();
        return;
      }
      if (e.key === "n" && handlers.onAdd) {
        e.preventDefault();
        handlers.onAdd();
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handlers]);
}

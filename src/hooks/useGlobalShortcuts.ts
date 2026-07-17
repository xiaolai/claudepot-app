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
 * App-scoped keyboard shortcuts. Each handler is optional so the
 * consumer can opt out (e.g. ⌘N is Accounts-specific today but ⌘R
 * refreshes any section that provides a handler).
 *
 * Respects the "don't fire while user is typing" rule by checking
 * `document.activeElement` — shortcuts bail if the focus is on an
 * editable surface (input, textarea, contenteditable).
 */
export function useGlobalShortcuts(handlers: {
  onRefresh?: () => void;
  onAdd?: () => void;
  onPalette?: () => void;
  onFilter?: () => void;
}): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.shiftKey || e.altKey) return;
      // Don't hijack typing — especially ⌘A / ⌘F inside an input.
      if (isEditable(document.activeElement)) {
        // ...except ⌘F which we still forward so the user can jump
        // to the app's own filter from an input (e.g. re-focus the
        // sidebar filter while typing in the command palette would
        // be surprising — keep this rule). We skip all editable-focus
        // events for now.
        return;
      }
      if (e.key === "r" && handlers.onRefresh) {
        e.preventDefault();
        handlers.onRefresh();
        return;
      }
      if (e.key === "n" && handlers.onAdd) {
        e.preventDefault();
        handlers.onAdd();
        return;
      }
      if (e.key === "k" && handlers.onPalette) {
        e.preventDefault();
        handlers.onPalette();
        return;
      }
      if (e.key === "f" && handlers.onFilter) {
        e.preventDefault();
        handlers.onFilter();
        return;
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handlers]);
}

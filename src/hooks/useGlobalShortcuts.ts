import { useEffect, useRef } from "react";

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
  // `isContentEditable` is the authoritative test in a real browser —
  // it is computed, so it also catches editability inherited from an
  // ancestor. It is NOT sufficient on its own: jsdom does not
  // implement the property, so under test a `contenteditable` element
  // reads as non-editable.
  //
  // That gap is why this union exists rather than the single line it
  // used to be. `useSection` carried a local copy that checked the
  // attribute as well, so consolidating onto this "canonical"
  // predicate initially made the gate *weaker* than the fork it
  // replaced, and a test that had been guarding ⌘4-over-contenteditable
  // went red. The shared predicate has to be the strongest of the
  // copies it absorbs, or consolidation is a regression wearing a
  // tidy-up's clothes.
  if ((el as HTMLElement).isContentEditable === true) return true;
  const attr = el.getAttribute("contenteditable");
  // Per HTML, a bare `contenteditable` and `contenteditable=""` both
  // mean true; `plaintext-only` is editable too.
  return attr === "" || attr === "true" || attr === "plaintext-only";
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
  // Callers pass an object literal, so depending on `handlers` itself
  // tore down and reinstalled the window listener on every render of
  // every section using this hook. The latest callbacks live in a ref
  // behind one stable listener instead.
  const latest = useRef(handlers);
  useEffect(() => {
    latest.current = handlers;
  });

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.shiftKey || e.altKey) return;
      // Don't hijack typing, and don't act on the section behind an
      // open modal — a ⌘R refresh or ⌘N add fired from under a dialog
      // leaves the user staring at a stale modal over changed content.
      if (isShortcutContextBlocked()) return;
      const { onRefresh, onAdd } = latest.current;
      if (e.key === "r" && onRefresh) {
        e.preventDefault();
        onRefresh();
        return;
      }
      if (e.key === "n" && onAdd) {
        e.preventDefault();
        onAdd();
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

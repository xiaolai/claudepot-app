import { useEffect, useRef } from "react";

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function useFocusTrap<T extends HTMLElement>() {
  const ref = useRef<T>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

    // Audit T4-7: capture the previously-focused element BEFORE we
    // move focus into the trap. On cleanup (modal/palette closes),
    // restore focus there so keyboard users land back on the trigger
    // element they activated, not at the document root. Mirrors the
    // behaviour of `Modal.tsx`'s prevFocusRef.
    const previouslyFocused = document.activeElement as HTMLElement | null;

    // Focus the autofocus element or the first focusable on mount
    const initialNodes = Array.from(el.querySelectorAll<HTMLElement>(FOCUSABLE));
    const auto = el.querySelector<HTMLElement>("[autofocus]");
    (auto ?? initialNodes[0])?.focus();

    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      // Query fresh on each Tab press so dynamic content is captured
      const items = Array.from(el.querySelectorAll<HTMLElement>(FOCUSABLE));
      if (items.length === 0) return;
      const first = items[0];
      const last = items[items.length - 1];

      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };

    el.addEventListener("keydown", onKey);
    return () => {
      el.removeEventListener("keydown", onKey);
      // Best-effort restore: skip if the previous element is gone
      // from the DOM (e.g. component unmounted) or never existed.
      if (previouslyFocused && document.contains(previouslyFocused)) {
        previouslyFocused.focus?.();
      }
    };
  }, []);

  return ref;
}

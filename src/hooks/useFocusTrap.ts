import { useEffect, useRef } from "react";

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function useFocusTrap<T extends HTMLElement>() {
  const ref = useRef<T>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;

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
    return () => el.removeEventListener("keydown", onKey);
  }, []);

  return ref;
}

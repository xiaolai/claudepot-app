import { useCallback, useEffect, useState } from "react";

/**
 * Persisted sidebar-collapse preference. Sidebar is the 240/260-px
 * left column; "collapsed" shrinks it to the rail width (≤52 px) and
 * hides labels + the swap-targets, activity, and sync strips that
 * need horizontal space to read.
 *
 * Persisted in localStorage under `cp-sidebar-collapsed`. Default is
 * expanded — a returning user keeps whatever state they last chose.
 *
 * Pattern mirrors `useTheme` so the keys and lifecycle are familiar.
 */
const KEY = "cp-sidebar-collapsed";

function read(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

export function useSidebarCollapsed(): {
  collapsed: boolean;
  toggle: () => void;
  setCollapsed: (next: boolean) => void;
} {
  const [collapsed, setState] = useState<boolean>(read);

  const setCollapsed = useCallback((next: boolean) => {
    setState(next);
    try {
      if (next) localStorage.setItem(KEY, "1");
      else localStorage.removeItem(KEY);
    } catch {
      // ignore — localStorage unavailable
    }
  }, []);

  const toggle = useCallback(() => {
    setCollapsed(!collapsed);
  }, [collapsed, setCollapsed]);

  // Cmd/Ctrl + \ — VSCode convention for "toggle sidebar". Bypasses
  // the global-shortcut hook because this is shell-level chrome, not
  // a per-section action, and uses a punctuation key rather than a
  // letter so it never conflicts with letter-based section shortcuts.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.shiftKey || e.altKey) return;
      if (e.key !== "\\") return;
      const el = document.activeElement;
      const tag = el?.tagName;
      const editable =
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        (el as HTMLElement | null)?.isContentEditable === true;
      if (editable) return;
      e.preventDefault();
      toggle();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggle]);

  return { collapsed, toggle, setCollapsed };
}

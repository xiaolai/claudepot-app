import { useCallback, useEffect, useState } from "react";

/**
 * Persisted sidebar-collapse preference. Sidebar is the 240/260-px
 * left column; "collapsed" shrinks it to the rail width (≤52 px) and
 * hides labels + the swap-targets, activity, and sync strips that
 * need horizontal space to read.
 *
 * Persisted in localStorage under `cp-sidebar-collapsed`, three-state:
 * `"1"` collapsed, `"0"` expanded, absent = never chosen. **The default
 * is collapsed** — the rail is the resting state — and a returning user
 * keeps whatever they last chose. There is no Settings toggle; ⌘\ and
 * the two chevrons (sidebar, status bar) are the controls.
 *
 * Expanded is stored explicitly rather than by absence. It used to BE
 * absence, which only worked while the default was expanded too:
 * flipping the default under that encoding would have made an expanded
 * choice unrememberable, so every launch re-collapsed a sidebar the
 * user had just opened. Anything other than the two sentinels reads as
 * "never chosen" — localStorage is user-mutable, and a stray value
 * must not wedge the reader.
 *
 * Pattern mirrors `useTheme` so the keys and lifecycle are familiar.
 */
import { SIDEBAR_COLLAPSED_KEY as KEY } from "../lib/storageKeys";
import { isShortcutContextBlocked } from "./useGlobalShortcuts";

const DEFAULT_COLLAPSED = true;

function read(): boolean {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "1") return true;
    if (v === "0") return false;
    return DEFAULT_COLLAPSED;
  } catch {
    return DEFAULT_COLLAPSED;
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
      localStorage.setItem(KEY, next ? "1" : "0");
    } catch {
      // ignore — localStorage unavailable
    }
  }, []);

  const toggle = useCallback(() => {
    setCollapsed(!collapsed);
  }, [collapsed, setCollapsed]);

  // Cmd/Ctrl + \ — VSCode convention for "toggle sidebar". Bypasses
  // the global-shortcut *hook* because this is shell-level chrome, not
  // a per-section action, and uses a punctuation key rather than a
  // letter so it never conflicts with letter-based section shortcuts.
  //
  // It does NOT bypass the shared gate. This handler used to re-derive
  // the check inline and tested editable focus only — never
  // `[role="dialog"]` — so ⌘\ collapsed the sidebar out from under an
  // open modal. That is precisely the bug class
  // `isShortcutContextBlocked` was extracted to kill, reintroduced by
  // the newest shortcut in the app. Bypassing the hook is fine;
  // bypassing the predicate is not.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || e.shiftKey || e.altKey) return;
      if (e.key !== "\\") return;
      if (isShortcutContextBlocked()) return;
      e.preventDefault();
      toggle();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggle]);

  return { collapsed, toggle, setCollapsed };
}

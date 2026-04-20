import { useCallback, useEffect, useState } from "react";

/**
 * Explicit theme preference. `null` means "follow prefers-color-scheme"
 * — nothing is written to the root and the cascade picks it up via the
 * `@media (prefers-color-scheme: dark)` block in tokens.css.
 */
export type ThemeMode = "light" | "dark" | null;

const CP_THEME_KEY = "cp-theme";

function readTheme(): ThemeMode {
  try {
    const v = localStorage.getItem(CP_THEME_KEY);
    return v === "light" || v === "dark" ? v : null;
  } catch {
    return null;
  }
}

function applyTheme(mode: ThemeMode): void {
  const html = document.documentElement;
  if (mode === null) {
    delete html.dataset.theme;
  } else {
    html.dataset.theme = mode;
  }
}

/**
 * Explicit-or-OS theme, persisted in localStorage under `cp-theme`.
 * Components use the current *effective* theme via `resolved` (useful
 * for icon choice — sun vs moon). `toggle()` flips light ↔ dark and
 * persists an explicit preference.
 */
export function useTheme(): {
  mode: ThemeMode;
  resolved: "light" | "dark";
  setMode: (next: ThemeMode) => void;
  toggle: () => void;
} {
  const [mode, setModeState] = useState<ThemeMode>(readTheme);

  const [osDark, setOsDark] = useState<boolean>(() => {
    if (typeof window.matchMedia !== "function") return false;
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  });

  useEffect(() => {
    applyTheme(mode);
  }, [mode]);

  useEffect(() => {
    if (typeof window.matchMedia !== "function") return;
    const mql = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setOsDark(mql.matches);
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, []);

  const setMode = useCallback((next: ThemeMode) => {
    setModeState(next);
    try {
      if (next === null) localStorage.removeItem(CP_THEME_KEY);
      else localStorage.setItem(CP_THEME_KEY, next);
    } catch {
      // ignore — localStorage unavailable
    }
  }, []);

  const resolved: "light" | "dark" =
    mode ?? (osDark ? "dark" : "light");

  const toggle = useCallback(() => {
    setMode(resolved === "dark" ? "light" : "dark");
  }, [resolved, setMode]);

  return { mode, resolved, setMode, toggle };
}

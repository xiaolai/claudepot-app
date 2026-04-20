import { useEffect, useState } from "react";

/**
 * Developer-mode toggle. When ON, <DevBadge> components surface
 * backend command names, raw paths, and internal identifiers next to
 * their human-facing labels. Persisted in localStorage under
 * `cp-dev-mode`; changes fire a window event so every mounted
 * `<DevBadge>` updates live.
 */
const CP_DEV_MODE_KEY = "cp-dev-mode";
const CP_DEV_MODE_EVENT = "cp-dev-mode-change";

export function readDevMode(): boolean {
  try {
    return localStorage.getItem(CP_DEV_MODE_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeDevMode(on: boolean): void {
  try {
    localStorage.setItem(CP_DEV_MODE_KEY, on ? "1" : "0");
  } catch {
    // ignore — localStorage unavailable (tests, sandboxed contexts)
  }
  window.dispatchEvent(
    new CustomEvent(CP_DEV_MODE_EVENT, { detail: !!on }),
  );
}

export function useDevMode(): [boolean, (on: boolean) => void] {
  const [on, setOn] = useState(readDevMode);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<boolean>).detail;
      setOn(!!detail);
    };
    const storage = (e: StorageEvent) => {
      if (e.key === CP_DEV_MODE_KEY) setOn(readDevMode());
    };
    window.addEventListener(CP_DEV_MODE_EVENT, handler);
    window.addEventListener("storage", storage);
    return () => {
      window.removeEventListener(CP_DEV_MODE_EVENT, handler);
      window.removeEventListener("storage", storage);
    };
  }, []);

  return [on, writeDevMode];
}

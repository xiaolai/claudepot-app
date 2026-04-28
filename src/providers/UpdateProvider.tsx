/**
 * UpdateProvider — owns the auto-update state machine and the in-app
 * "Settings → About" surface. The plugin lives in Rust
 * (`tauri-plugin-updater`); this provider drives its lifecycle from
 * the renderer and persists user preferences to localStorage.
 *
 * State machine:
 *
 *   idle ──── checkForUpdate() ────► checking
 *   checking ──────► up-to-date | available | error
 *   available ── downloadAndInstall() ────► downloading
 *   downloading ──── (progress emits) ────► ready | error
 *   ready ──── relaunch() ────► (process exits)
 *
 * Persistence model: settings live in localStorage under
 * `claudepot.update.*` keys. They're per-install UI state, not user
 * preferences — they don't need to follow a user across machines.
 *
 * Why a Context provider, not a global zustand-style store: claudepot
 * already standardizes on Context + custom hooks for shared state
 * (`AppStateProvider`, `useToasts`). Adding zustand for one feature
 * would fragment the conventions.
 */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { check, type Update, type DownloadEvent } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "../api";

const ONE_DAY_MS = 24 * 60 * 60 * 1000;
const ONE_WEEK_MS = 7 * ONE_DAY_MS;
/**
 * Wait 2 s after first paint before checking for updates. Two reasons:
 *   1. The webview is still hydrating; a network request stealing
 *      cycles during that window noticeably slows the first frame.
 *   2. Smooths over transient connectivity hiccups on cold start
 *      (e.g. waking from sleep, Wi-Fi captive portals re-handshaking).
 */
const STARTUP_DELAY_MS = 2_000;
/**
 * Exponential backoff base for retrying a failed background check.
 * Sequence is 5 s → 10 s → 20 s, then we give up. A flapping network
 * never auto-toasts the user — the status indicator inside Settings
 * → About reflects the failure, but no banner / toast disturbs them.
 */
const RETRY_BASE_MS = 5_000;
const RETRY_MAX = 3;

export type UpdateStatus =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "downloading"
  | "ready"
  | "error";

export type CheckFrequency = "startup" | "daily" | "weekly" | "manual";

export interface UpdateInfo {
  version: string;
  notes: string;
  pubDate: string | null;
  currentVersion: string;
}

export interface DownloadProgress {
  /** Bytes downloaded so far. */
  downloaded: number;
  /** Total bytes — `null` if the server didn't send a Content-Length. */
  total: number | null;
}

interface UpdateContextValue {
  /**
   * `null` while the platform-support probe is in flight, then `true`
   * if the install can be updated in-place (macOS, Windows, Linux
   * AppImage), `false` otherwise (Linux .deb / system install). When
   * false, the auto-check is skipped and the Settings → About pane
   * shows a "use your package manager" hint instead of the controls.
   */
  supported: boolean | null;
  status: UpdateStatus;
  updateInfo: UpdateInfo | null;
  downloadProgress: DownloadProgress | null;
  error: string | null;
  /** True iff the user pressed "Skip this version". Resets on next check. */
  isSkipped: boolean;

  // Settings.
  autoCheckEnabled: boolean;
  setAutoCheckEnabled: (v: boolean) => void;
  checkFrequency: CheckFrequency;
  setCheckFrequency: (v: CheckFrequency) => void;
  /** Last time we *successfully* completed a check. Drives "shouldCheckNow". */
  lastCheckedAt: number | null;

  // Actions.
  /** Run a check immediately. Sets `manual=true` to surface errors as toasts. */
  checkNow: () => Promise<void>;
  /** Download the available update and stage it for install. */
  downloadAndInstall: () => Promise<void>;
  /** Re-launch the app to apply a staged update. */
  applyUpdate: () => Promise<void>;
  /** Mark the available version as skipped — the UI hides the prompt. */
  skipThisVersion: () => void;
  /** Clear any "skip" the user previously set. */
  resetSkip: () => void;
}

const UpdateContext = createContext<UpdateContextValue | null>(null);

const LS = {
  autoCheck: "claudepot.update.autoCheckEnabled",
  freq: "claudepot.update.checkFrequency",
  lastCheckedAt: "claudepot.update.lastCheckedAt",
  skipVersion: "claudepot.update.skipVersion",
} as const;

function readLocalString(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

function readLocalBool(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    if (v === "true") return true;
    if (v === "false") return false;
    return fallback;
  } catch {
    return fallback;
  }
}

function readLocalNumber(key: string): number | null {
  try {
    const v = localStorage.getItem(key);
    if (v == null) return null;
    const n = Number(v);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

function writeLocal(key: string, value: string | null) {
  try {
    if (value == null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // localStorage may be disabled in some webviews — settings will
    // simply not persist across launches. Not fatal.
  }
}

function shouldCheckNow(
  enabled: boolean,
  freq: CheckFrequency,
  lastCheckedAt: number | null,
): boolean {
  if (!enabled) return false;
  if (freq === "manual") return false;
  if (freq === "startup") return true;
  if (lastCheckedAt == null) return true;
  const elapsed = Date.now() - lastCheckedAt;
  if (freq === "daily") return elapsed >= ONE_DAY_MS;
  if (freq === "weekly") return elapsed >= ONE_WEEK_MS;
  return false;
}

export function UpdateProvider({ children }: { children: ReactNode }) {
  // Probed once via Tauri command. `null` until the probe lands so
  // the auto-check effect doesn't fire prematurely on the wrong
  // assumption.
  const [supported, setSupported] = useState<boolean | null>(null);
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [downloadProgress, setDownloadProgress] =
    useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [skipVersion, setSkipVersion] = useState<string | null>(() =>
    readLocalString(LS.skipVersion, "") || null,
  );

  // Settings — initialized from localStorage so refresh restores prefs.
  const [autoCheckEnabled, setAutoCheckEnabledState] = useState<boolean>(
    () => readLocalBool(LS.autoCheck, true),
  );
  const [checkFrequency, setCheckFrequencyState] = useState<CheckFrequency>(
    () => {
      const raw = readLocalString(LS.freq, "daily");
      return raw === "startup" || raw === "daily" || raw === "weekly" ||
        raw === "manual"
        ? (raw as CheckFrequency)
        : "daily";
    },
  );
  const [lastCheckedAt, setLastCheckedAt] = useState<number | null>(() =>
    readLocalNumber(LS.lastCheckedAt),
  );

  // Refs that don't need to trigger renders.
  const pendingUpdateRef = useRef<Update | null>(null);
  const hasAutoCheckedRef = useRef(false);
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const retryCountRef = useRef(0);

  const setAutoCheckEnabled = useCallback((v: boolean) => {
    setAutoCheckEnabledState(v);
    writeLocal(LS.autoCheck, v ? "true" : "false");
  }, []);

  const setCheckFrequency = useCallback((v: CheckFrequency) => {
    setCheckFrequencyState(v);
    writeLocal(LS.freq, v);
  }, []);

  const skipThisVersion = useCallback(() => {
    if (!updateInfo) return;
    setSkipVersion(updateInfo.version);
    writeLocal(LS.skipVersion, updateInfo.version);
  }, [updateInfo]);

  const resetSkip = useCallback(() => {
    setSkipVersion(null);
    writeLocal(LS.skipVersion, null);
  }, []);

  // Cancel any pending retry timer. Used by every code path that
  // would invalidate it: success, manual check, download, unmount.
  // Also resets retry count so the next failure starts fresh — a
  // single 5 s back-off is closer to user expectation than picking
  // up the previous exponential staircase.
  const cancelRetry = useCallback(() => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current);
      retryTimerRef.current = null;
    }
    retryCountRef.current = 0;
  }, []);

  // Close the currently held Update handle, if any. The plugin
  // returns a Rust-backed Resource on every `check()`; we must
  // explicitly close it or the resources table grows unbounded.
  const closePendingUpdate = useCallback(async () => {
    const upd = pendingUpdateRef.current;
    pendingUpdateRef.current = null;
    if (upd) {
      try {
        await upd.close();
      } catch {
        // Resource may already be invalidated by a finished install;
        // best-effort close.
      }
    }
  }, []);

  // The actual check. `manual=true` means the user clicked the button;
  // we surface errors and don't suppress any state transitions.
  const doCheck = useCallback(async (): Promise<void> => {
    setStatus("checking");
    setError(null);
    try {
      const result = await check();
      // Mark a successful round-trip — even if there's no update.
      const now = Date.now();
      setLastCheckedAt(now);
      writeLocal(LS.lastCheckedAt, String(now));

      // A successful round-trip means any in-flight retry is now
      // redundant — cancel it before it fires and yanks state back to
      // `checking`. Same on the no-update path below.
      cancelRetry();

      if (!result) {
        // Close the previous Update handle (if any) before dropping
        // the ref. Each `check()` returns a Rust-backed `Resource`;
        // forgetting them leaks entries in Tauri's resources table
        // until the app exits.
        await closePendingUpdate();
        setUpdateInfo(null);
        setStatus("up-to-date");
        return;
      }

      // If a previous check produced a different Update handle (older
      // version still pending in some weird race), close it first.
      await closePendingUpdate();
      pendingUpdateRef.current = result;
      const info: UpdateInfo = {
        version: result.version,
        notes: result.body ?? "",
        pubDate: result.date ?? null,
        currentVersion: result.currentVersion,
      };
      setUpdateInfo(info);
      setStatus("available");
    } catch (e) {
      // The plugin throws strings or Errors depending on the path. We
      // normalize to a single message string.
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStatus("error");
      throw e;
    }
  }, []);

  const checkNow = useCallback(async () => {
    // A manual check supersedes any background retry that's queued.
    cancelRetry();
    try {
      await doCheck();
    } catch {
      // doCheck already set status=error + the error message; the
      // caller's UI reads these. Swallow so callers don't have to
      // wrap in try/catch every time.
    }
  }, [doCheck, cancelRetry]);

  const downloadAndInstall = useCallback(async () => {
    const upd = pendingUpdateRef.current;
    if (!upd) {
      setError("No update available to download.");
      setStatus("error");
      return;
    }
    // The user committed; any background retry would now be noise.
    cancelRetry();
    setStatus("downloading");
    setDownloadProgress({ downloaded: 0, total: null });
    try {
      let total: number | null = null;
      let downloaded = 0;
      await upd.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? null;
          setDownloadProgress({ downloaded: 0, total });
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setDownloadProgress({ downloaded, total });
        } else if (event.event === "Finished") {
          setDownloadProgress({ downloaded: total ?? downloaded, total });
        }
      });
      setStatus("ready");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStatus("error");
    }
  }, []);

  const applyUpdate = useCallback(async () => {
    try {
      await relaunch();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStatus("error");
    }
  }, []);

  // Probe platform support exactly once. The result drives both the
  // auto-check effect (skip on .deb) and the About pane (hide
  // controls). Failure is treated as "not supported" — better to
  // silence the UI than to surface a check that won't work.
  useEffect(() => {
    let cancelled = false;
    api
      .updaterSupported()
      .then((ok) => {
        if (!cancelled) setSupported(ok);
      })
      .catch(() => {
        if (!cancelled) setSupported(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Auto-check on startup. Runs once per app launch, after the
  // platform probe lands. The 2 s delay lets the renderer hydrate
  // before the network call.
  useEffect(() => {
    if (supported !== true) return;
    if (hasAutoCheckedRef.current) return;
    if (!shouldCheckNow(autoCheckEnabled, checkFrequency, lastCheckedAt)) {
      return;
    }
    hasAutoCheckedRef.current = true;
    const t = setTimeout(() => {
      void doCheck().catch(() => {
        // Background failure → schedule a retry.
        scheduleRetry();
      });
    }, STARTUP_DELAY_MS);
    return () => clearTimeout(t);
    // We deliberately do NOT depend on settings — the check fires at
    // most once per launch. Toggling autoCheck mid-session shouldn't
    // re-fire the startup check.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [supported]);

  const scheduleRetry = useCallback(() => {
    // Defensively cancel any timer the previous round may have left
    // behind. With the cleanup-on-status-change effect removed (see
    // below), this is the single place that arms a retry, so a
    // duplicate setTimeout would otherwise quietly stack up.
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current);
      retryTimerRef.current = null;
    }
    if (retryCountRef.current >= RETRY_MAX) return;
    const delay = RETRY_BASE_MS * Math.pow(2, retryCountRef.current);
    retryCountRef.current += 1;
    retryTimerRef.current = setTimeout(() => {
      void doCheck().catch(() => scheduleRetry());
    }, delay);
  }, [doCheck]);

  // Cancel any pending retry on unmount. We do NOT cancel on every
  // status change — the previous version did, which races with the
  // catch-handler that *just* armed a retry: the cleanup ran after
  // `setStatus("error")` flushed and clobbered the freshly-set timer
  // before it could fire. Cancellation now happens explicitly inside
  // `doCheck()` and `downloadAndInstall()`.
  useEffect(() => {
    return () => {
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current);
        retryTimerRef.current = null;
      }
      // Best-effort: drop the Update handle if the provider unmounts
      // mid-cycle. Fire-and-forget; we have no `await` budget here.
      void closePendingUpdate();
    };
  }, [closePendingUpdate]);

  const isSkipped = useMemo(
    () => !!(updateInfo && skipVersion && updateInfo.version === skipVersion),
    [updateInfo, skipVersion],
  );

  const value = useMemo<UpdateContextValue>(
    () => ({
      supported,
      status,
      updateInfo,
      downloadProgress,
      error,
      isSkipped,
      autoCheckEnabled,
      setAutoCheckEnabled,
      checkFrequency,
      setCheckFrequency,
      lastCheckedAt,
      checkNow,
      downloadAndInstall,
      applyUpdate,
      skipThisVersion,
      resetSkip,
    }),
    [
      supported,
      status,
      updateInfo,
      downloadProgress,
      error,
      isSkipped,
      autoCheckEnabled,
      setAutoCheckEnabled,
      checkFrequency,
      setCheckFrequency,
      lastCheckedAt,
      checkNow,
      downloadAndInstall,
      applyUpdate,
      skipThisVersion,
      resetSkip,
    ],
  );

  return (
    <UpdateContext.Provider value={value}>{children}</UpdateContext.Provider>
  );
}

/**
 * Read the update state machine + actions. Throws if used outside an
 * `<UpdateProvider>` — the provider is mounted at the App root, so a
 * thrown error here means a test or a stray Storybook story missed
 * the wrapper.
 */
export function useUpdater(): UpdateContextValue {
  const ctx = useContext(UpdateContext);
  if (!ctx) {
    throw new Error("useUpdater must be used inside <UpdateProvider>");
  }
  return ctx;
}

// Exported for unit tests.
export { shouldCheckNow };

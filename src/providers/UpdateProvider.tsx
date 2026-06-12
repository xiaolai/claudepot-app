/**
 * UpdateProvider — owns the auto-update state machine and the in-app
 * "Settings → About" surface.
 *
 * # Channel-aware: why this drives Rust commands, not the JS plugin
 *
 * Claudepot offers a user-selectable release channel (stable / beta).
 * The JavaScript `@tauri-apps/plugin-updater` `check()` *cannot*
 * override the manifest endpoint — `CheckOptions` has no `endpoints`
 * field — so a runtime channel toggle has to drive check/download/
 * install from Rust, where `UpdaterBuilder::endpoints()` is the one
 * runtime override point. This provider therefore calls the Rust
 * `release_*` commands (`api.releaseUpdateCheck` /
 * `releaseUpdateInstall` / `releaseChannelGet` / `releaseChannelSet`)
 * instead of the JS plugin's `check()` / `downloadAndInstall()`. The
 * relaunch still goes through `@tauri-apps/plugin-process`.
 *
 * State machine (unchanged shape):
 *
 *   idle ──── checkForUpdate() ────► checking
 *   checking ──────► up-to-date | available | error
 *   available ── downloadAndInstall() ────► downloading
 *   downloading ──── (progress events) ────► ready | error
 *   ready ──── relaunch() ────► (process exits)
 *
 * Persistence model:
 *   - Check-frequency / auto-check / skip-version stay in
 *     localStorage under `claudepot.update.*` — per-install UI state.
 *   - The release channel lives in the Rust-side `preferences.json`
 *     (`release_channel`) because the Rust check command must read it
 *     each call; localStorage is not reachable from Rust.
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
import { relaunch } from "@tauri-apps/plugin-process";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../api";
import {
  RELEASE_DOWNLOAD_EVENT,
  type ReleaseChannelName,
  type ReleaseDownloadProgress,
} from "../api/releaseUpdate";
import { ConfirmDialog } from "../components/ConfirmDialog";
import {
  UPDATE_AUTO_CHECK_KEY,
  UPDATE_CHECK_FREQ_KEY,
  UPDATE_LAST_CHECKED_KEY,
  UPDATE_SKIP_VERSION_KEY,
} from "../lib/storageKeys";

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

/** The release channel the in-app updater reads. */
export type ReleaseChannel = ReleaseChannelName;

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
  /**
   * Non-null when the last check found the running build to be a
   * prerelease *newer* than the Stable channel's current release
   * (the Beta → Stable switch case). The status is "up-to-date" at
   * the state-machine level, but the About pane must render a
   * stranded explanation instead of "you're on the latest version".
   */
  stranded: { stableVersion: string | null } | null;

  // Settings.
  autoCheckEnabled: boolean;
  setAutoCheckEnabled: (v: boolean) => void;
  checkFrequency: CheckFrequency;
  setCheckFrequency: (v: CheckFrequency) => void;
  /** Last time we *successfully* completed a check. Drives "shouldCheckNow". */
  lastCheckedAt: number | null;
  /**
   * The release channel. `null` until the Rust preference loads.
   * Persisted Rust-side; switching it takes effect on the next check.
   */
  releaseChannel: ReleaseChannel | null;
  setReleaseChannel: (v: ReleaseChannel) => void;

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
  autoCheck: UPDATE_AUTO_CHECK_KEY,
  freq: UPDATE_CHECK_FREQ_KEY,
  lastCheckedAt: UPDATE_LAST_CHECKED_KEY,
  skipVersion: UPDATE_SKIP_VERSION_KEY,
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
  const [stranded, setStranded] = useState<{
    stableVersion: string | null;
  } | null>(null);
  // Labels of in-flight background ops blocking a relaunch — non-null
  // while the "restart anyway?" confirm dialog is up.
  const [pendingRelaunchOps, setPendingRelaunchOps] = useState<
    string[] | null
  >(null);

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
  // Release channel lives in the Rust preferences file (the Rust
  // check command reads it each call). `null` until it loads.
  const [releaseChannel, setReleaseChannelState] =
    useState<ReleaseChannel | null>(null);

  // Refs that don't need to trigger renders.
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

  // Persist the channel Rust-side. Optimistically update the local
  // state so the `<select>` is responsive; the Rust call normalizes
  // and we re-sync to its return value. The new channel takes effect
  // on the *next* check — the Rust check command reads the pref each
  // call — so no restart is needed.
  const setReleaseChannel = useCallback((v: ReleaseChannel) => {
    setReleaseChannelState(v);
    // A channel switch invalidates any prior check result: the Rust
    // side clears its stashed Update handle (`release_channel_set`),
    // and the renderer must not keep offering the *other* channel's
    // update. Reset to idle until the next check runs against the
    // new channel.
    setStatus("idle");
    setUpdateInfo(null);
    setDownloadProgress(null);
    setStranded(null);
    void api
      .releaseChannelSet(v)
      .then((normalized) => setReleaseChannelState(normalized))
      .catch((e) => {
        // Surface the failure but don't tear down the UI — the
        // local state already shows the user's intent; the next
        // check will simply use whatever Rust actually persisted.
        const msg = e instanceof Error ? e.message : String(e);
        setError(`Couldn't save release channel: ${msg}`);
      });
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

  // The actual check. `manual=true` means the user clicked the button;
  // we surface errors and don't suppress any state transitions.
  //
  // The Rust `release_update_check` reads the persisted channel,
  // checks that channel's manifest, and stashes the resulting update
  // handle Rust-side — there is no JS-side `Update` resource to close
  // (Rust owns the handle for the subsequent install).
  const doCheck = useCallback(async (): Promise<void> => {
    setStatus("checking");
    setError(null);
    try {
      const result = await api.releaseUpdateCheck();
      // Mark a successful round-trip — even if there's no update.
      const now = Date.now();
      setLastCheckedAt(now);
      writeLocal(LS.lastCheckedAt, String(now));
      // Keep the channel state in sync with whatever the check ran
      // against (cheap, and covers a first-check-before-load race).
      setReleaseChannelState(result.channel);

      // A successful round-trip means any in-flight retry is now
      // redundant — cancel it before it fires and yanks state back to
      // `checking`.
      cancelRetry();

      // Stranded-on-prerelease: the running build is a prerelease
      // newer than the Stable channel's current release. Carried
      // alongside "up-to-date" so the badge can tell the truth.
      setStranded(
        result.strandedOnPrerelease
          ? { stableVersion: result.stableVersion }
          : null,
      );

      if (!result.updateAvailable) {
        setUpdateInfo(null);
        setStatus("up-to-date");
        return;
      }

      const info: UpdateInfo = {
        version: result.version ?? "",
        notes: result.notes ?? "",
        pubDate: result.pubDate,
        currentVersion: result.currentVersion,
      };
      setUpdateInfo(info);
      setStatus("available");
    } catch (e) {
      // The command rejects with a string or Error; normalize.
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStatus("error");
      throw e;
    }
    // `cancelRetry` is stable (useCallback, no deps); omitting it
    // keeps `doCheck` itself stable so the auto-check effect's
    // single-fire contract holds.
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    // The user committed; any background retry would now be noise.
    cancelRetry();
    setStatus("downloading");
    setDownloadProgress({ downloaded: 0, total: null });

    // Subscribe to the Rust download-progress event for the duration
    // of this install. The Rust `release_update_install` emits
    // `started` / `progress` / `finished` ticks on
    // `RELEASE_DOWNLOAD_EVENT`. We unsubscribe in `finally` so a
    // failed/aborted install can't leak the listener.
    let unlisten: UnlistenFn | null = null;
    try {
      unlisten = await listen<ReleaseDownloadProgress>(
        RELEASE_DOWNLOAD_EVENT,
        (ev) => {
          const p = ev.payload;
          if (p.event === "started") {
            setDownloadProgress({ downloaded: 0, total: p.contentLength });
          } else if (p.event === "progress") {
            setDownloadProgress({
              downloaded: p.downloaded,
              total: p.contentLength,
            });
          } else if (p.event === "finished") {
            setDownloadProgress((prev) => ({
              downloaded: prev?.total ?? prev?.downloaded ?? 0,
              total: prev?.total ?? null,
            }));
          }
        },
      );
      await api.releaseUpdateInstall();
      setStatus("ready");
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStatus("error");
    } finally {
      if (unlisten) unlisten();
    }
  }, [cancelRetry]);

  const doRelaunch = useCallback(async () => {
    try {
      await relaunch();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setStatus("error");
    }
  }, []);

  const applyUpdate = useCallback(async () => {
    // Quiesce probe before the restart: relaunch kills the process
    // immediately, and a half-completed background op (credential
    // swap, CC auto-install) is not crash-protected the way repair
    // ops are. When anything is mid-flight, warn-confirm via the
    // dialog rendered below — mirrors the quit gate
    // (`app_menu::attempt_quit`). Zero overhead when idle.
    let busy: string[] = [];
    try {
      busy = await api.relaunchBusyOps();
    } catch {
      // A failed probe must not strand the user behind a dead
      // "Restart to update" button — proceed as before.
    }
    if (busy.length > 0) {
      setPendingRelaunchOps(busy);
      return;
    }
    await doRelaunch();
  }, [doRelaunch]);

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

  // Load the persisted release channel once on mount. Independent of
  // the platform probe — the channel selector renders even while the
  // support probe is in flight, and the value is harmless to hold
  // regardless of platform.
  useEffect(() => {
    let cancelled = false;
    api
      .releaseChannelGet()
      .then((c) => {
        if (!cancelled) setReleaseChannelState(c);
      })
      .catch(() => {
        // Default to "stable" on a read failure — matches the Rust
        // preference default, so the UI shows the true effective
        // value rather than a blank select.
        if (!cancelled) setReleaseChannelState("stable");
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const scheduleRetry = useCallback(() => {
    // Defensively cancel any timer the previous round may have left
    // behind. This is the single place that arms a retry, so a
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

  // Cancel any pending retry on unmount. We do NOT cancel on every
  // status change — cancellation happens explicitly inside
  // `doCheck()` and `downloadAndInstall()`.
  useEffect(() => {
    return () => {
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current);
        retryTimerRef.current = null;
      }
    };
  }, []);

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
      stranded,
      autoCheckEnabled,
      setAutoCheckEnabled,
      checkFrequency,
      setCheckFrequency,
      lastCheckedAt,
      releaseChannel,
      setReleaseChannel,
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
      stranded,
      autoCheckEnabled,
      setAutoCheckEnabled,
      checkFrequency,
      setCheckFrequency,
      lastCheckedAt,
      releaseChannel,
      setReleaseChannel,
      checkNow,
      downloadAndInstall,
      applyUpdate,
      skipThisVersion,
      resetSkip,
    ],
  );

  return (
    <UpdateContext.Provider value={value}>
      {children}
      {/* Pre-relaunch quiesce confirm. Rendered by the provider (not a
          section) because applyUpdate can be invoked from any surface
          and the warning must never depend on which pane is mounted. */}
      {pendingRelaunchOps && (
        <ConfirmDialog
          title="Operations still running"
          body={
            <>
              <p style={{ margin: 0 }}>
                Restarting now will abandon the work below. Repairable
                operations leave a journal entry you can resume later;
                one-shot operations will need to be restarted.
              </p>
              <ul style={{ margin: "var(--sp-8) 0 0", paddingLeft: "var(--sp-20)" }}>
                {pendingRelaunchOps.map((label, i) => (
                  // Index keys are fine: the list is a static snapshot
                  // and two ops can legitimately share a label.
                  <li key={i}>{label}</li>
                ))}
              </ul>
            </>
          }
          confirmLabel="Restart anyway"
          confirmDanger
          onConfirm={() => {
            setPendingRelaunchOps(null);
            void doRelaunch();
          }}
          onCancel={() => setPendingRelaunchOps(null)}
        />
      )}
    </UpdateContext.Provider>
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

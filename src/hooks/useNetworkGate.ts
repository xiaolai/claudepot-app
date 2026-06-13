import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type {
  FirstRunNetworkStatus,
  NetworkDiagnosis,
} from "../api/service-status";

/**
 * First-run network reachability gate. See
 * `dev-docs/network-detection-panel.md`.
 *
 * Probes `api.anthropic.com` once on mount (after a short delay to
 * let the webview settle). The panel renders when `state ===
 * "unreachable"` and not dismissed. Dismissal is per-session
 * (`sessionStorage`) — re-arms on next launch but not on a tab
 * switch within the same session.
 *
 * The hook deliberately does NOT block app interaction. The panel is
 * informational + remediation; the user can navigate to offline
 * surfaces (Sessions, Memory, Cleanup) regardless.
 */

import { NETWORK_GATE_DISMISSED_KEY as SESSION_DISMISS_KEY } from "../lib/storageKeys";

/** Time to wait after mount before firing the probe. Mirrors
 *  `service_status_watcher::FIRST_TICK_DELAY` philosophy: long enough
 *  for the webview to mount and the listener to be installed, short
 *  enough that fresh data lands within seconds of opening the app.
 *  Tests pass `{ probeDelayMs: 0 }` to skip the wait. */
const DEFAULT_PROBE_DELAY_MS = 1500;

export interface UseNetworkGateOptions {
  /** Override the post-mount probe delay. Production callers should
   *  use the default; only tests need this. */
  probeDelayMs?: number;
}

export type NetworkGateState =
  | { kind: "unknown" }
  | { kind: "probing" }
  | { kind: "ok"; latencyMs: number | null }
  | {
      kind: "unreachable";
      diagnosis: NetworkDiagnosis;
      message: string | null;
    };

export interface UseNetworkGate {
  state: NetworkGateState;
  /** True iff the unreachable panel should render right now. Combines
   *  state.kind === "unreachable" with the per-session dismissal. */
  shouldShowPanel: boolean;
  /** Hide the panel for the rest of this session. Re-arms next launch. */
  dismiss: () => void;
  /** Re-run the probe. The panel disappears if reachable. */
  retry: () => void;
}

function readDismissed(): boolean {
  try {
    return sessionStorage.getItem(SESSION_DISMISS_KEY) === "1";
  } catch {
    // Private browsing / quota / disabled storage — treat as not
    // dismissed. Worst case the panel re-shows; never silently hide
    // the network problem.
    return false;
  }
}

function writeDismissed(): void {
  try {
    sessionStorage.setItem(SESSION_DISMISS_KEY, "1");
  } catch {
    // Same swallow as above — losing the dismissal is benign.
  }
}

export function useNetworkGate(opts: UseNetworkGateOptions = {}): UseNetworkGate {
  const probeDelayMs = opts.probeDelayMs ?? DEFAULT_PROBE_DELAY_MS;
  const [state, setState] = useState<NetworkGateState>({ kind: "unknown" });
  const [dismissed, setDismissed] = useState<boolean>(() => readDismissed());

  // Monotonic request generation. Each `runProbe()` call captures the
  // current value, then bumps it. The resolve/reject handlers compare
  // their captured gen to the live ref; a mismatch means a newer
  // probe (or unmount) has superseded this one and we drop the
  // result. Closes the audit-flagged race where retry()'s success
  // could be overwritten by a slow original probe, AND the
  // setState-after-unmount warning. */
  const reqGenRef = useRef(0);
  // Tracks whether the hook is still mounted. setState after unmount
  // is a React warning at minimum and a memory leak at worst.
  const aliveRef = useRef(true);

  const runProbe = useCallback(() => {
    const myGen = ++reqGenRef.current;
    setState({ kind: "probing" });
    void api
      .networkFirstRunCheck()
      .then((res: FirstRunNetworkStatus) => {
        if (!aliveRef.current || reqGenRef.current !== myGen) return;
        if (res.diagnosis === "reachable") {
          setState({ kind: "ok", latencyMs: res.latencyMs });
        } else {
          setState({
            kind: "unreachable",
            diagnosis: res.diagnosis,
            message: res.message,
          });
        }
      })
      .catch((e: unknown) => {
        if (!aliveRef.current || reqGenRef.current !== myGen) return;
        // Probe command itself failed (Tauri bridge issue, not a
        // network issue). Treat as "unknown reachability" rather
        // than triggering the panel — a Tauri-bridge fault should
        // not be presented to the user as "Anthropic unreachable".
        // The Settings → Network pane covers manual diagnostics.
        // eslint-disable-next-line no-console
        console.warn("network gate probe failed:", e);
        setState({ kind: "unknown" });
      });
  }, []);

  // Track mount status across the hook's lifetime so probe handlers
  // can short-circuit after unmount.
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
      // Bump the generation so any in-flight probe drops its result.
      reqGenRef.current += 1;
    };
  }, []);

  // One-shot probe on mount, after a short delay.
  useEffect(() => {
    const t = setTimeout(runProbe, probeDelayMs);
    return () => clearTimeout(t);
  }, [runProbe, probeDelayMs]);

  const dismiss = useCallback(() => {
    writeDismissed();
    setDismissed(true);
  }, []);

  const retry = useCallback(() => {
    runProbe();
  }, [runProbe]);

  const shouldShowPanel = !dismissed && state.kind === "unreachable";

  return { state, shouldShowPanel, dismiss, retry };
}

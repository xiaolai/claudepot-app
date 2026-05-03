import { useEffect, useState } from "react";
import { listen, type Event as TauriEvent } from "@tauri-apps/api/event";
import { api } from "../api";
import { useServiceStatus } from "../hooks/useServiceStatus";
import { tierColor, tierLabel } from "../api/service-status";
import type { Preferences } from "../types";
import { formatRelative } from "../lib/formatRelative";

/**
 * Small color-coded dot in the StatusBar's right cluster. Shows the
 * worst-of (status-page tier × latency tier) so a green dot really
 * means "Anthropic is healthy AND your path to Anthropic is fast."
 *
 * Hidden when the user has both `poll_status_page` and
 * `probe_latency_on_focus` toggled off — there's nothing to show. A
 * single toggle ON keeps the dot visible because partial information
 * is still useful (e.g. polling enabled but probing off renders the
 * page tier alone).
 *
 * Click triggers a re-probe; tooltip carries the per-host latency
 * table and incident summary so the dot is self-explanatory without
 * having to dive into Settings → Network.
 */
export function ServiceStatusDot() {
  const [prefs, setPrefs] = useState<Preferences | null>(null);
  const [hovering, setHovering] = useState(false);

  // Fetch on mount + listen for `cp-prefs-changed` (the same channel
  // every other prefs-aware hook uses — see `src/api/activity.ts`'s
  // `broadcastPrefsChanged`). The event payload IS the new
  // Preferences snapshot, so we don't have to round-trip a second
  // `preferencesGet`.
  useEffect(() => {
    let cancelled = false;
    api
      .preferencesGet()
      .then((p) => {
        if (!cancelled) setPrefs(p);
      })
      .catch(() => {});

    let unlisten: (() => void) | undefined;
    listen<Preferences>("cp-prefs-changed", (ev: TauriEvent<Preferences>) => {
      if (!cancelled) setPrefs(ev.payload);
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // Defensive: tests + older callers may omit `service_status`. Treat
  // a missing block as "feature off" instead of crashing the shell.
  const ss = prefs?.service_status;
  const enabled = ss != null && (ss.poll_status_page || ss.probe_latency_on_focus);
  const pollStatusPage = ss?.poll_status_page ?? false;
  const probeOnFocus = ss?.probe_latency_on_focus ?? false;

  const { summary, latency, probing, tier, probeNow } = useServiceStatus({
    enabled,
    pollStatusPage,
    probeOnFocus,
  });

  if (!enabled) return null;

  const color = tierColor(tier);
  const label = tierLabel(tier);

  const probeAge =
    latency && latency.probedAtMs > 0
      ? formatRelative(latency.probedAtMs)
      : "never";
  const summaryAge = summary?.fetchedAtMs
    ? formatRelative(summary.fetchedAtMs)
    : "never";

  return (
    <div
      style={{ position: "relative" }}
      onMouseEnter={() => setHovering(true)}
      onMouseLeave={() => setHovering(false)}
    >
      <button
        type="button"
        onClick={() => void probeNow()}
        aria-label={`${label}. Click to refresh.`}
        title={`${label} — click to refresh.`}
        style={{
          width: "var(--icon-btn-sm)",
          height: "var(--icon-btn-sm)",
          padding: 0,
          background: "transparent",
          border: 0,
          cursor: "pointer",
          display: "inline-flex",
          alignItems: "center",
          justifyContent: "center",
          fontFamily: "inherit",
        }}
      >
        <span
          style={{
            display: "inline-block",
            width: "var(--sp-8)",
            height: "var(--sp-8)",
            borderRadius: "50%",
            background: color,
            // Pulse while a probe is in flight so the user knows the
            // dot is updating rather than stale.
            animation: probing ? "service-status-pulse 1.2s ease-in-out infinite" : undefined,
            boxShadow: `0 0 0 var(--bw-hair) color-mix(in oklch, ${color} 40%, transparent)`,
          }}
        />
      </button>

      {hovering && (
        <div
          role="tooltip"
          style={{
            position: "absolute",
            right: 0,
            bottom: "calc(100% + var(--sp-6))",
            minWidth: "tokens.config.menu.min.width",
            padding: "var(--sp-10) var(--sp-12)",
            background: "var(--bg-raised)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            boxShadow: "var(--shadow-popover)",
            color: "var(--fg)",
            fontSize: "var(--fs-2xs)",
            letterSpacing: "var(--ls-normal)",
            textTransform: "none",
            zIndex: 10,
            pointerEvents: "none",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-6)",
              fontWeight: 600,
            }}
          >
            <span
              style={{
                display: "inline-block",
                width: "var(--sp-8)",
                height: "var(--sp-8)",
                borderRadius: "50%",
                background: color,
                flexShrink: 0,
              }}
            />
            {summary?.description ?? label}
          </div>
          <div style={{ color: "var(--fg-faint)", marginTop: "var(--sp-2)" }}>
            Status page · checked {summaryAge}
          </div>

          {summary?.incidents && summary.incidents.length > 0 && (
            <div style={{ marginTop: "var(--sp-8)" }}>
              {summary.incidents.slice(0, 3).map((inc) => (
                <div
                  key={inc.id}
                  style={{
                    color: "var(--warn)",
                    marginBottom: "var(--sp-2)",
                  }}
                >
                  • {inc.name}
                </div>
              ))}
            </div>
          )}

          {latency && latency.hosts.length > 0 && (
            <>
              <div
                style={{
                  marginTop: "var(--sp-10)",
                  borderTop: "var(--bw-hair) solid var(--line)",
                  paddingTop: "var(--sp-8)",
                  color: "var(--fg-faint)",
                }}
              >
                Path latency · probed {probeAge}
              </div>
              <table
                style={{
                  width: "100%",
                  borderCollapse: "collapse",
                  marginTop: "var(--sp-4)",
                }}
              >
                <tbody>
                  {latency.hosts.map((h) => (
                    <tr key={h.name}>
                      <td
                        style={{
                          color: "var(--fg-muted)",
                          paddingRight: "var(--sp-8)",
                          // The redacted error message from Rust
                          // (sk-ant tokens stripped) lives in
                          // `h.message` for kind === "error". Surface
                          // it as a native title so the user can hover
                          // to read "DNS lookup failed", "connection
                          // refused", etc., instead of just "error".
                        }}
                        title={h.kind === "error" ? (h.message ?? undefined) : undefined}
                      >
                        {h.name}
                      </td>
                      <td
                        style={{
                          textAlign: "right",
                          fontVariantNumeric: "tabular-nums",
                          color: hostColor(h.kind),
                        }}
                        title={h.kind === "error" ? (h.message ?? undefined) : undefined}
                      >
                        {hostValue(h)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}

          {summary?.lastError && (
            <div
              style={{
                marginTop: "var(--sp-8)",
                color: "var(--warn)",
              }}
            >
              Last poll failed: {summary.lastError}
            </div>
          )}
        </div>
      )}

      <style>{`
        @keyframes service-status-pulse {
          0%, 100% { opacity: 1; transform: scale(1); }
          50% { opacity: 0.55; transform: scale(0.85); }
        }
        @media (prefers-reduced-motion: reduce) {
          @keyframes service-status-pulse {
            0%, 100% { opacity: 0.85; }
          }
        }
      `}</style>
    </div>
  );
}

function hostColor(kind: "ok" | "timeout" | "error"): string {
  switch (kind) {
    case "ok":
      return "var(--fg)";
    case "timeout":
    case "error":
      return "var(--danger)";
  }
}

function hostValue(h: {
  kind: "ok" | "timeout" | "error";
  ms: number | null;
  message: string | null;
}): string {
  switch (h.kind) {
    case "ok":
      return h.ms != null ? `${h.ms} ms` : "—";
    case "timeout":
      return "timeout";
    case "error":
      return "error";
  }
}

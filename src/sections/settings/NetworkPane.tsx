import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import { tierColor, tierLabel } from "../../api/service-status";
import { Button } from "../../components/primitives/Button";
import { ExternalLink } from "../../components/primitives/ExternalLink";
import { useServiceStatus } from "../../hooks/useServiceStatus";
import { formatRelative } from "../../lib/formatRelative";
import { renderError } from "../../lib/i18n-error";
import type { Preferences } from "../../types";

interface Props {
  pushToast: (kind: "info" | "error", text: string) => void;
}

/**
 * Settings → Network pane. Configure the network-status feature
 * (status-page polling + on-focus latency probes). See
 * `dev-docs/network-status.md` for the cost / cadence rationale
 * behind each default.
 *
 * The pane shares its data source with the StatusBar dot via
 * `useServiceStatus`; toggling here immediately refreshes the dot
 * because the api setter emits `cp-prefs-changed`.
 */
export function NetworkPane({ pushToast }: Props) {
  const { t } = useTranslation("settings");
  const [prefs, setPrefs] = useState<Preferences | null>(null);
  // Monotonic save token. Each `setField` call increments and captures
  // its own value; the response is only applied if the captured token
  // still matches the latest. Guards against fast toggles where save N
  // resolves after save N+1 and would otherwise stomp the newer state.
  const saveTokenRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    api
      .preferencesGet()
      .then((p) => {
        if (!cancelled) setPrefs(p);
      })
      .catch((e) => {
        if (!cancelled)
          pushToast("error", renderError(e, t("shared.prefsLoadFailed")));
      });
    return () => {
      cancelled = true;
    };
  }, [pushToast, t]);

  // Defensive: tests / partial fixtures may omit `service_status`.
  const ss = prefs?.service_status;
  const enabled = ss != null && (ss.poll_status_page || ss.probe_latency_on_focus);
  const pollStatusPage = ss?.poll_status_page ?? false;
  const probeOnFocus = ss?.probe_latency_on_focus ?? false;

  const { summary, latency, probing, refresh, probeNow } = useServiceStatus({
    enabled,
    pollStatusPage,
    probeOnFocus,
  });

  const setField = useCallback(
    async (
      patch: {
        pollStatusPage?: boolean;
        pollIntervalMinutes?: number;
        osNotifyOnStatusChange?: boolean;
        probeLatencyOnFocus?: boolean;
      },
    ) => {
      const myToken = ++saveTokenRef.current;

      // Optimistic local update via functional setState so back-to-back
      // toggles compose correctly even when their async saves overlap.
      // Revert tracked through `prevSnapshot` for the failure path.
      let prevSnapshot: Preferences | null = null;
      setPrefs((p) => {
        if (!p || !p.service_status) return p;
        prevSnapshot = p;
        return {
          ...p,
          service_status: {
            ...p.service_status,
            ...(patch.pollStatusPage !== undefined && {
              poll_status_page: patch.pollStatusPage,
            }),
            ...(patch.pollIntervalMinutes !== undefined && {
              poll_interval_minutes: patch.pollIntervalMinutes,
            }),
            ...(patch.osNotifyOnStatusChange !== undefined && {
              os_notify_on_status_change: patch.osNotifyOnStatusChange,
            }),
            ...(patch.probeLatencyOnFocus !== undefined && {
              probe_latency_on_focus: patch.probeLatencyOnFocus,
            }),
          },
        };
      });

      try {
        const fresh = await api.preferencesSetServiceStatus(patch);
        // Drop the response if a newer save has been issued — the
        // newer optimistic state is more correct than this stale
        // snapshot, and applying it would clobber that newer change.
        if (saveTokenRef.current === myToken) setPrefs(fresh);
      } catch (e) {
        // Same staleness guard: a later save has already been issued,
        // its optimistic update is now the truth.
        if (saveTokenRef.current === myToken) setPrefs(prevSnapshot);
        pushToast("error", renderError(e, t("shared.saveFailed")));
      }
    },
    [pushToast, t],
  );

  if (!prefs || !ss) {
    return (
      <div style={{ color: "var(--fg-faint)" }}>{t("network.loadingPrefs")}</div>
    );
  }

  const dotColor = tierColor(summary?.tier ?? "unknown");

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-20)" }}>
      <p style={{ margin: 0, color: "var(--fg-muted)", fontSize: "var(--fs-sm)" }}>
        <Trans
          ns="settings"
          i18nKey="network.intro"
          components={{
            code: <code />,
            link: (
              <ExternalLink href="https://github.com/xiaolai/anthropic-claude-surge-rules-set">
                xiaolai/anthropic-claude-surge-rules-set
              </ExternalLink>
            ),
          }}
        />
      </p>

      {/* Status-page polling -------------------------------------- */}
      <Section title={t("network.serviceStatus")}>
        <ToggleRow
          checked={ss.poll_status_page}
          onChange={(v) => void setField({ pollStatusPage: v })}
          label={t("network.poll.label")}
          detail={t("network.poll.detail", {
            minutes: ss.poll_interval_minutes,
          })}
        />

        <div
          style={{
            marginLeft: "var(--sp-24)",
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-10)",
            opacity: ss.poll_status_page ? 1 : 0.5,
          }}
        >
          <label
            htmlFor="poll-interval"
            style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}
          >
            {t("network.poll.every")}
          </label>
          <input
            id="poll-interval"
            type="number"
            min={2}
            max={60}
            disabled={!ss.poll_status_page}
            value={ss.poll_interval_minutes}
            onChange={(e) => {
              const v = Number(e.target.value);
              if (!Number.isFinite(v)) return;
              void setField({ pollIntervalMinutes: v });
            }}
            style={{
              width: "var(--sp-64)",
              padding: "var(--sp-3) var(--sp-6)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-1)",
              background: "var(--bg-raised)",
              color: "var(--fg)",
              fontFamily: "inherit",
              fontSize: "var(--fs-sm)",
              fontVariantNumeric: "tabular-nums",
            }}
          />
          <span style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}>
            {t("network.poll.minutes")}
          </span>
        </div>

        <ToggleRow
          checked={ss.os_notify_on_status_change}
          onChange={(v) => void setField({ osNotifyOnStatusChange: v })}
          label={t("network.osNotify.label")}
          detail={t("network.osNotify.detail")}
        />
      </Section>

      {/* Latency probe -------------------------------------------- */}
      <Section title={t("network.latency.title")}>
        <ToggleRow
          checked={ss.probe_latency_on_focus}
          onChange={(v) => void setField({ probeLatencyOnFocus: v })}
          label={t("network.latency.label")}
          detail={t("network.latency.detail")}
        />
      </Section>

      {/* Live data -------------------------------------------- */}
      {enabled && (
        <Section title={t("network.current.title")}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-8)",
              fontSize: "var(--fs-sm)",
            }}
          >
            <span
              style={{
                display: "inline-block",
                width: "var(--sp-10)",
                height: "var(--sp-10)",
                borderRadius: "50%",
                background: dotColor,
                flexShrink: 0,
              }}
            />
            <span style={{ fontWeight: 600 }}>
              {summary?.description ?? tierLabel(summary?.tier ?? "unknown")}
            </span>
            <span style={{ color: "var(--fg-faint)" }}>
              {summary?.fetchedAtMs
                ? t("network.current.checked", {
                    when: formatRelative(summary.fetchedAtMs),
                  })
                : t("network.current.noData")}
            </span>
          </div>

          {summary?.lastError && (
            <div
              style={{
                marginTop: "var(--sp-6)",
                fontSize: "var(--fs-xs)",
                color: "var(--warn)",
              }}
            >
              {t("network.current.lastPollFailed", {
                error: summary.lastError,
              })}
            </div>
          )}

          {summary?.incidents && summary.incidents.length > 0 && (
            <div style={{ marginTop: "var(--sp-10)" }}>
              <div style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)", marginBottom: "var(--sp-4)" }}>
                {t("network.current.incidents")}
              </div>
              {summary.incidents.map((inc) => (
                <div key={inc.id} style={{ fontSize: "var(--fs-sm)", color: "var(--warn)" }}>
                  • {inc.name} ({inc.status})
                </div>
              ))}
            </div>
          )}

          <div
            style={{
              marginTop: "var(--sp-12)",
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-10)",
            }}
          >
            <Button
              variant="ghost"
              onClick={() => {
                void refresh();
                void probeNow();
              }}
              disabled={probing}
            >
              {probing
                ? t("network.current.probing")
                : t("network.current.probeNow")}
            </Button>
            {latency && latency.probedAtMs > 0 && (
              <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
                {t("network.current.probedAt", {
                  when: formatRelative(latency.probedAtMs),
                })}
              </span>
            )}
          </div>

          {latency && latency.hosts.length > 0 && (
            <table
              style={{
                marginTop: "var(--sp-10)",
                width: "100%",
                maxWidth: "var(--project-detail-width)",
                borderCollapse: "collapse",
                fontSize: "var(--fs-sm)",
              }}
            >
              <tbody>
                {latency.hosts.map((h) => (
                  <tr key={h.name}>
                    <td
                      style={{
                        padding: "var(--sp-3) var(--sp-8) var(--sp-3) 0",
                        color: "var(--fg-muted)",
                      }}
                    >
                      <code>{h.name}</code>
                    </td>
                    <td
                      style={{
                        padding: "var(--sp-3) 0",
                        textAlign: "right",
                        fontVariantNumeric: "tabular-nums",
                        color:
                          h.kind === "ok"
                            ? "var(--fg)"
                            : "var(--danger)",
                      }}
                    >
                      {h.kind === "ok" && h.ms != null
                        ? `${h.ms} ms`
                        : h.kind === "timeout"
                          ? t("network.current.timeout")
                          : t("network.current.error")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Section>
      )}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-10)",
      }}
    >
      <h3
        style={{
          margin: 0,
          fontSize: "var(--fs-sm)",
          fontWeight: 600,
          letterSpacing: "var(--ls-tight)",
          color: "var(--fg)",
        }}
      >
        {title}
      </h3>
      {children}
    </section>
  );
}

/**
 * A checkbox with a label and an explanatory line under it.
 *
 * **`htmlFor` + `aria-describedby`, not a wrapping `<label>`.** A label
 * that wraps both the control and the explanation takes its accessible
 * name from ALL of its text, so this announced as "Probe latency on
 * open Runs a HEAD request against each endpoint…" — the whole
 * paragraph read out as the control's name. Name and description are
 * different relationships and assistive tech treats them differently:
 * the name identifies, the description elaborates on request.
 *
 * `SettingToggleRow` is the canonical version of this row and had it
 * right; this is the same contract on a checkbox.
 */
function ToggleRow({
  checked,
  onChange,
  label,
  detail,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  detail?: string;
}) {
  const id = useId();
  const detailId = `${id}-detail`;
  return (
    <div
      style={{
        display: "flex",
        alignItems: "flex-start",
        gap: "var(--sp-10)",
      }}
    >
      <input
        id={id}
        type="checkbox"
        checked={checked}
        aria-describedby={detail ? detailId : undefined}
        onChange={(e) => onChange(e.target.checked)}
        style={{
          marginTop: "var(--sp-3)",
          flexShrink: 0,
          accentColor: "var(--accent)",
        }}
      />
      <span style={{ display: "flex", flexDirection: "column", gap: "var(--sp-2)" }}>
        <label htmlFor={id} style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}>
          {label}
        </label>
        {detail && (
          <span id={detailId} style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
            {detail}
          </span>
        )}
      </span>
    </div>
  );
}

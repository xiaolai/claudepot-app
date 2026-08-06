import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import type {
  ArtifactUsageStatsDto,
  ConfigEffectiveSettingsDto,
} from "../../types";
import { HookUsageLine, hookArtifactKey } from "./HookUsageLine";
import {
  extractHookRows,
  type HookRow,
} from "./hooksData";

// Re-export so existing import sites (`./HooksRenderer`) keep working.
export { countHooksInMergedSettings } from "./hooksData";

/**
 * Hooks view — first-class surface over the `hooks` field inside the
 * merged effective settings. In Claude Code, hooks aren't stand-alone
 * files; they live as arrays under a per-event key
 * (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, …) inside any
 * settings.json in the cascade. This renderer extracts, flattens, and
 * displays them with a per-row event badge and a deterministic path
 * index into the merged JSON so provenance can be looked up.
 *
 * One row per (event, matcher, command). No editing — this is a
 * read-only view of what CC would execute.
 */

export function HooksRenderer({ cwd }: { cwd: string | null }) {
  const { t } = useTranslation("config");
  const [data, setData] = useState<ConfigEffectiveSettingsDto | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setError(null);
    setData(null);
    void api
      .configEffectiveSettings(cwd)
      .then((d) => {
        if (!cancelled) setData(d);
      })
      .catch((e) => {
        if (!cancelled) setError(renderError(e));
      });
    return () => {
      cancelled = true;
    };
  }, [cwd]);

  const rows = useMemo<HookRow[]>(
    () => (data ? extractHookRows(data) : []),
    [data],
  );

  // Per-hook usage stats keyed by the canonical artifact key
  // (`<hookName>|<command>`) so two hooks sharing a command but firing
  // on different events get their own counts. Must stay in sync with
  // `claudepot_core::artifact_usage::extract::hook_artifact_key`.
  const [usageByKey, setUsageByKey] = useState<
    Map<string, ArtifactUsageStatsDto>
  >(new Map());
  useEffect(() => {
    if (rows.length === 0) {
      setUsageByKey(new Map());
      return;
    }
    let cancelled = false;
    const seen = new Set<string>();
    const keys: Array<["hook", string]> = [];
    for (const r of rows) {
      // hookName mirrors backend convention: "<event>:<matcher>".
      const hookName = r.matcher ? `${r.event}:${r.matcher}` : r.event;
      const key = hookArtifactKey(hookName, r.command);
      if (!key || seen.has(key)) continue;
      seen.add(key);
      keys.push(["hook", key]);
    }
    api
      .artifactUsageBatch(keys)
      .then((result) => {
        if (cancelled) return;
        const map = new Map<string, ArtifactUsageStatsDto>();
        for (const r of result) map.set(r.artifact_key, r.stats);
        setUsageByKey(map);
      })
      .catch(() => {
        if (cancelled) return;
        // Clear stale state on failure so the next successful fetch
        // doesn't render decorations from a previous unrelated set.
        setUsageByKey(new Map());
      });
    return () => {
      cancelled = true;
    };
  }, [rows]);

  if (error) {
    return (
      <div
        style={{
          padding: "var(--sp-20)",
          color: "var(--danger)",
          fontSize: "var(--fs-sm)",
        }}
      >
        {t("hooks.loadFailed", { error })}
      </div>
    );
  }

  if (!data) {
    return (
      <div
        style={{
          padding: "var(--sp-20)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-sm)",
        }}
      >
        {t("state.loading")}
      </div>
    );
  }

  if (rows.length === 0) {
    return (
      <div
        style={{
          padding: "var(--sp-32)",
          color: "var(--fg-faint)",
          fontSize: "var(--fs-sm)",
          textAlign: "center",
        }}
      >
        {t("hooks.empty")}
      </div>
    );
  }

  // Group rows by event so the view reads "what fires on PreToolUse →
  // what fires on PostToolUse …" rather than interleaved matchers.
  const byEvent = new Map<string, HookRow[]>();
  for (const r of rows) {
    const bucket = byEvent.get(r.event) ?? [];
    bucket.push(r);
    byEvent.set(r.event, bucket);
  }
  const eventOrder = Array.from(byEvent.keys()).sort();

  return (
    <div
      style={{
        flex: 1,
        overflow: "auto",
        padding: "var(--sp-16) var(--sp-20) var(--sp-24)",
      }}
    >
      {eventOrder.map((event) => (
        <section key={event} style={{ marginBottom: "var(--sp-24)" }}>
          <div
            style={{
              display: "flex",
              alignItems: "baseline",
              gap: "var(--sp-8)",
              marginBottom: "var(--sp-8)",
            }}
          >
            <h3
              style={{
                margin: 0,
                fontSize: "var(--fs-sm)",
                fontWeight: 600,
                fontFamily: "var(--font-mono)",
              }}
            >
              {event}
            </h3>
            <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-faint)" }}>
              {t("hooks.count", { count: byEvent.get(event)!.length })}
            </span>
          </div>
          <ol
            style={{
              listStyle: "none",
              margin: 0,
              padding: 0,
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-8)",
            }}
          >
            {byEvent.get(event)!.map((row, i) => (
              <li
                key={`${row.mergedPath}#${i}`}
                style={{
                  border: "var(--bw-hair) solid var(--line)",
                  borderRadius: "var(--r-2)",
                  padding: "var(--sp-10) var(--sp-12)",
                  background: "var(--bg-sunken)",
                  display: "flex",
                  flexDirection: "column",
                  gap: "var(--sp-4)",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    alignItems: "baseline",
                    gap: "var(--sp-8)",
                    fontSize: "var(--fs-xs)",
                    color: "var(--fg-muted)",
                  }}
                >
                  <span style={{ fontFamily: "var(--font-mono)" }}>
                    {t("hooks.matcher")}
                  </span>
                  <code
                    style={{
                      fontFamily: "var(--font-mono)",
                      color: "var(--fg)",
                    }}
                  >
                    {row.matcher || t("hooks.anyMatcher")}
                  </code>
                  {row.winner && (
                    <span
                      style={{
                        marginLeft: "auto",
                        fontSize: "var(--fs-2xs)",
                        color: "var(--fg-faint)",
                      }}
                      title={t("hooks.winnerTitle")}
                    >
                      {row.winner}
                    </span>
                  )}
                </div>
                <pre
                  style={{
                    margin: 0,
                    fontFamily: "var(--font-mono)",
                    fontSize: "var(--fs-xs)",
                    color: "var(--fg)",
                    whiteSpace: "pre-wrap",
                    overflowWrap: "anywhere",
                  }}
                >
                  {row.command}
                </pre>
                <HookUsageLine
                  stats={usageByKey.get(
                    hookArtifactKey(
                      row.matcher ? `${row.event}:${row.matcher}` : row.event,
                      row.command,
                    ) ?? "",
                  )}
                />
                {row.timeoutMs != null && (
                  <div
                    style={{
                      fontSize: "var(--fs-2xs)",
                      color: "var(--fg-faint)",
                    }}
                  >
                    {t("hooks.timeout", { ms: row.timeoutMs })}
                  </div>
                )}
              </li>
            ))}
          </ol>
        </section>
      ))}
    </div>
  );
}

// Data extraction (extractHookRows, countHooksInMergedSettings, plus
// the provenance and shape helpers) lives in `./hooksData.ts` —
// imported above. This file is now renderer-only.

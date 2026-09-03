import { useCallback, useEffect, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { api } from "../../api";
import {
  GRANT_DURATION_PRESETS,
  permissionModeLabel,
  type PermissionRevertedEvent,
  type ProjectPermission,
} from "../../api/permission";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import { i18n } from "../../lib/i18n";
import { renderError } from "../../lib/i18n-error";
import { useAppState } from "../../providers/AppStateProvider";

/** "1h 47m", "47m", or "<1m" for a positive millisecond span. */
function formatRemaining(ms: number): string {
  const t = i18n.getFixedT(null, "projects");
  if (ms <= 0) return t("permission.expired");
  const totalMin = Math.floor(ms / 60_000);
  if (totalMin < 1) return t("permission.underMinute");
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  return h > 0
    ? t("permission.hoursMinutes", { h, m })
    : t("permission.minutesOnly", { m });
}

/**
 * Per-project permission control. A grant makes Claudepot answer
 * Claude Code's `PreToolUse` check with `allow` for tool calls whose
 * session is inside this project, until the grant lapses. It writes
 * nothing into the project's settings: Claude Code has ignored
 * `bypassPermissions` from project files since 2.1.257, and a value
 * left there by an older Claudepot is rendered here as *ignored*, with
 * a one-click removal.
 *
 * The dangerous part of unattended approval isn't switching it on —
 * it's forgetting to switch it off. Time-boxed grants lapse on their
 * own and a live countdown keeps the state visible; the sticky preset
 * stays visible in the same place with the same Revoke button.
 *
 * A project running in `bypassPermissions` from the user's own
 * `~/.claude/settings.json` shows as elevated but *not*
 * Claudepot-managed — we surface it, but don't offer to change
 * someone else's deliberate choice.
 */
export function PermissionPanel({
  projectPath,
  onError,
}: {
  projectPath: string;
  onError?: (msg: string) => void;
}) {
  const { t } = useTranslation("projects");
  const { pushToast } = useAppState();
  const [perm, setPerm] = useState<ProjectPermission | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  // `null` represents the "Never" preset → sticky grant. Initial
  // value uses the first preset so common-case (time-boxed) is
  // pre-selected; users opt into sticky deliberately.
  const [durationSecs, setDurationSecs] = useState<number | null>(
    GRANT_DURATION_PRESETS[0].secs,
  );
  // Re-render tick so the countdown stays fresh without refetching.
  const [, setNowTick] = useState(0);
  // Bumped to force a refetch — by the `permission-reverted` event
  // (a grant lapsed on the orchestrator's tick) without a manual
  // reselect.
  const [reloadTick, setReloadTick] = useState(0);

  const fail = useCallback(
    (msg: string) => {
      if (onError) onError(msg);
      else pushToast("error", msg);
    },
    [onError, pushToast],
  );

  // Single-project fetch (not the full-tree `permissionList`). Re-runs
  // on `projectPath` change and on `reloadTick` bumps; the `cancelled`
  // guard keeps a slow in-flight fetch from clobbering a newer one.
  //
  // Stale-while-revalidate: don't flip loading=true on refetches when
  // we already have data. The `loading || !perm` render branch below
  // then only fires on first mount; later refetches swap the content
  // in atomically. Same defect-class as the Env panel one step over.
  useEffect(() => {
    let cancelled = false;
    api
      .permissionGet(projectPath)
      .then((p) => {
        if (!cancelled) setPerm(p);
      })
      .catch((e) => {
        if (!cancelled) {
          fail(renderError(e, i18n.t("projects:permission.loadFailedScope")));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [projectPath, reloadTick, fail]);

  // The orchestrator drops lapsed grants on its 5-min tick and emits
  // `permission-reverted`. Refetch when that fires for THIS project so
  // the countdown doesn't linger on a stale "expired".
  useTauriEvent<PermissionRevertedEvent>("permission-reverted", (e) => {
    if (e.payload.projectPath === projectPath) {
      setReloadTick((n) => n + 1);
    }
  });

  // Tick once a minute while a TIME-BOXED grant is active so the
  // countdown re-renders. Sticky grants have no countdown — no
  // ticker needed.
  const timeBoxedGrantActive =
    !!perm?.activeGrant && perm.activeGrant.expiresAtMs != null;
  const tickRef = useRef<number | null>(null);
  useEffect(() => {
    if (!timeBoxedGrantActive) return;
    tickRef.current = window.setInterval(
      () => setNowTick((n) => n + 1),
      30_000,
    );
    return () => {
      if (tickRef.current != null) window.clearInterval(tickRef.current);
    };
  }, [timeBoxedGrantActive]);

  const run = useCallback(
    async (
      action: () => Promise<ProjectPermission>,
      okToast: string,
      failScope: string,
    ) => {
      setBusy(true);
      try {
        setPerm(await action());
        pushToast("info", okToast);
      } catch (e) {
        fail(renderError(e, failScope));
      } finally {
        setBusy(false);
      }
    },
    [pushToast, fail],
  );

  const grant = () =>
    run(
      () => api.permissionGrant(projectPath, durationSecs),
      durationSecs == null
        ? t("permission.grantStickyToast")
        : t("permission.grantTimedToast"),
      t("permission.grantFailedScope"),
    );

  const extend = () =>
    run(
      () => api.permissionExtend(projectPath, durationSecs),
      durationSecs == null
        ? t("permission.neverExpireToast")
        : t("permission.extendedToast"),
      t("permission.extendFailedScope"),
    );

  const revert = () =>
    run(
      () => api.permissionRevert(projectPath),
      t("permission.revertedToast"),
      t("permission.revertFailedScope"),
    );

  const clearIgnored = () =>
    run(
      () => api.permissionClearIgnored(projectPath),
      t("permission.clearIgnoredToast"),
      t("permission.clearIgnoredFailedScope"),
    );

  if (loading || !perm) {
    return (
      <section className="detail-section">
        <h3>{t("permission.heading")}</h3>
        <p className="muted small">
          {loading ? t("shared.loading") : t("permission.noData")}
        </p>
      </section>
    );
  }

  const g = perm.activeGrant;
  const remainingMs =
    g && g.expiresAtMs != null ? g.expiresAtMs - Date.now() : 0;
  const isSticky = g != null && g.expiresAtMs == null;
  // Elevated with no Claudepot grant — the user set this in their own
  // settings file. We surface it; we don't manage it.
  const elevatedByHand = perm.isElevated && !g;
  const ignored = perm.ignoredValue;

  return (
    <section className="detail-section">
      <h3 style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)" }}>
        {t("permission.heading")}
        {perm.isElevated ? (
          <Tag tone="danger" glyph={NF.unlock} title={t("permission.elevatedTitle")}>
            {t("permission.elevatedTag")}
          </Tag>
        ) : ignored ? (
          <Tag tone="warn" glyph={NF.lock} title={t("permission.ignoredTitle")}>
            {t("permission.ignoredTag")}
          </Tag>
        ) : (
          <Tag tone="ghost" glyph={NF.lock}>
            {permissionModeLabel(perm.effectiveMode)}
          </Tag>
        )}
      </h3>

      {ignored && (
        <div className="permission-ignored" role="note">
          <p className="muted small">
            <Trans
              ns="projects"
              i18nKey={
                ignored.layer === "local_project"
                  ? "permission.ignoredLocal"
                  : "permission.ignoredProject"
              }
              values={{
                mode: ignored.mode,
                since: perm.projectScopeIgnoresSince,
              }}
              components={{ b: <strong />, f: <code /> }}
            />
          </p>
          {ignored.layer === "local_project" && (
            <Button variant="outline" onClick={clearIgnored} disabled={busy} glyph={NF.trash}>
              {t("permission.clearIgnored")}
            </Button>
          )}
        </div>
      )}

      {g ? (
        <div className="permission-grant-active" role="status">
          <span>
            {isSticky ? (
              <Trans
                ns="projects"
                i18nKey="permission.activeSticky"
                components={{ hold: <strong /> }}
              />
            ) : (
              <Trans
                ns="projects"
                i18nKey="permission.activeTimed"
                components={{
                  time: <strong>{formatRemaining(remainingMs)}</strong>,
                }}
              />
            )}
          </span>
          {!perm.hookInstalled && (
            <p className="small permission-hook-missing" role="alert">
              {t("permission.hookMissing")}
            </p>
          )}
          <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center" }}>
            <DurationSelect
              value={durationSecs}
              onChange={setDurationSecs}
              disabled={busy}
            />
            <Button variant="outline" onClick={extend} disabled={busy} glyph={NF.clock}>
              {durationSecs == null
                ? t("permission.makeSticky")
                : isSticky
                  ? t("permission.setDeadline")
                  : t("permission.extend")}
            </Button>
            <Button variant="solid" onClick={revert} disabled={busy} glyph={NF.lock}>
              {t("permission.revertNow")}
            </Button>
          </div>
        </div>
      ) : elevatedByHand ? (
        <p className="muted small">
          <Trans
            ns="projects"
            i18nKey="permission.byHand"
            values={{
              mode: perm.effectiveMode,
              source: decisionLabel(perm.decidedBy),
            }}
            components={{ b: <strong /> }}
          />
        </p>
      ) : (
        <div className="permission-grant-form">
          <p className="muted small">
            <Trans
              ns="projects"
              i18nKey="permission.grantIntro"
              components={{ b1: <code />, b2: <strong /> }}
            />
          </p>
          <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center" }}>
            <DurationSelect
              value={durationSecs}
              onChange={setDurationSecs}
              disabled={busy}
            />
            <Button variant="solid" onClick={grant} disabled={busy} glyph={NF.unlock}>
              {t("permission.grantButton")}
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

// Sentinel for the "Never" preset in the <select>'s string-typed
// value space — `<option value>` can't carry `null` directly.
const NEVER_SENTINEL = "never";

function DurationSelect({
  value,
  onChange,
  disabled,
}: {
  value: number | null;
  onChange: (secs: number | null) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation("projects");
  return (
    <select
      className="mono"
      value={value == null ? NEVER_SENTINEL : String(value)}
      disabled={disabled}
      onChange={(e) =>
        onChange(e.target.value === NEVER_SENTINEL ? null : Number(e.target.value))
      }
      aria-label={t("permission.durationAria")}
      style={{
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line-strong)",
        borderRadius: "var(--r-2)",
        color: "var(--fg)",
        font: "inherit",
        fontSize: "var(--fs-sm)",
        padding: "var(--sp-4) var(--sp-8)",
      }}
    >
      {GRANT_DURATION_PRESETS.map((d) => (
        <option
          key={d.secs == null ? NEVER_SENTINEL : d.secs}
          value={d.secs == null ? NEVER_SENTINEL : String(d.secs)}
        >
          {d.label}
        </option>
      ))}
    </select>
  );
}

function decisionLabel(src: ProjectPermission["decidedBy"]): string {
  switch (src) {
    case "local_project_settings":
      return ".claude/settings.local.json";
    case "project_settings":
      return ".claude/settings.json";
    case "user_settings":
      return "~/.claude/settings.json";
    case "project_scope_ignored":
    case "default":
      return i18n.t("projects:permission.ccDefault");
  }
}

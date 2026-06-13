import { useCallback, useEffect, useRef, useState } from "react";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { api } from "../../api";
import {
  GRANT_DURATION_PRESETS,
  permissionModeLabel,
  type PermissionBreakerTrippedEvent,
  type PermissionRevertedEvent,
  type ProjectPermission,
} from "../../api/permission";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import { useAppState } from "../../providers/AppStateProvider";

/** "1h 47m", "47m", or "<1m" for a positive millisecond span. */
function formatRemaining(ms: number): string {
  if (ms <= 0) return "expired";
  const totalMin = Math.floor(ms / 60_000);
  if (totalMin < 1) return "<1m";
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/**
 * Per-project permission control. The dangerous part of
 * `bypassPermissions` isn't toggling it on — it's forgetting to turn
 * it off. So this panel never grants an open-ended bypass: every
 * grant is time-boxed and the orchestrator auto-reverts it. A live
 * countdown keeps the elevated state visible while it lasts.
 *
 * A project elevated by the user's own hand-edited settings shows as
 * elevated but *not* Claudepot-managed — we surface it, but don't
 * offer to revert someone else's deliberate choice.
 */
export function PermissionPanel({
  projectPath,
  onError,
}: {
  projectPath: string;
  onError?: (msg: string) => void;
}) {
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
  // (orchestrator auto-revert) without a manual reselect.
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
        if (!cancelled) fail(`Couldn't load permission state: ${e}`);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [projectPath, reloadTick, fail]);

  // The orchestrator auto-reverts expired grants server-side on its
  // 5-min tick and emits `permission-reverted`. Refetch when that
  // fires for THIS project so the countdown doesn't linger on a
  // stale "expired".
  useTauriEvent<PermissionRevertedEvent>("permission-reverted", (e) => {
    if (e.payload.projectPath === projectPath) {
      setReloadTick((n) => n + 1);
    }
  });

  // The orchestrator quarantines a grant whose auto-revert keeps
  // failing — its consecutive-failure circuit breaker trips and
  // `permission-breaker-tripped` fires once. Surface it for THIS
  // project so the user knows the grant is stuck (not silently
  // reverting) and may need a manual revert. Refetch too — the
  // breaker counter is part of the grant state.
  useTauriEvent<PermissionBreakerTrippedEvent>(
    "permission-breaker-tripped",
    (e) => {
      if (e.payload.projectPath === projectPath) {
        pushToast(
          "error",
          `Auto-revert for this project was paused after ${e.payload.consecutiveFailures} consecutive failures. The bypass grant is still active — revert it manually, then check the project's settings file.`,
        );
        setReloadTick((n) => n + 1);
      }
    },
  );

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

  const grant = useCallback(async () => {
    setBusy(true);
    try {
      const next = await api.permissionGrant(
        projectPath,
        "bypassPermissions",
        durationSecs,
      );
      setPerm(next);
      pushToast(
        "info",
        durationSecs == null
          ? "Bypass granted — stays until you revert."
          : "Bypass granted — auto-reverts on schedule.",
      );
    } catch (e) {
      fail(`Grant failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }, [projectPath, durationSecs, pushToast, fail]);

  const extend = useCallback(async () => {
    setBusy(true);
    try {
      const next = await api.permissionExtend(projectPath, durationSecs);
      setPerm(next);
      pushToast(
        "info",
        durationSecs == null
          ? "Grant set to never expire."
          : "Grant extended.",
      );
    } catch (e) {
      fail(`Extend failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }, [projectPath, durationSecs, pushToast, fail]);

  const revert = useCallback(async () => {
    setBusy(true);
    try {
      const next = await api.permissionRevert(projectPath);
      setPerm(next);
      pushToast("info", "Reverted to the prior permission mode.");
    } catch (e) {
      fail(`Revert failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }, [projectPath, pushToast, fail]);

  if (loading || !perm) {
    return (
      <section className="detail-section">
        <h3>Permissions</h3>
        <p className="muted small">
          {loading ? "Loading…" : "No permission data for this project."}
        </p>
      </section>
    );
  }

  const g = perm.activeGrant;
  const remainingMs =
    g && g.expiresAtMs != null ? g.expiresAtMs - Date.now() : 0;
  const isSticky = g != null && g.expiresAtMs == null;
  // Elevated, but no Claudepot grant — the user set this in their own
  // settings file. We surface it; we don't manage it.
  const elevatedByHand = perm.isElevated && !g;

  return (
    <section className="detail-section">
      <h3 style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)" }}>
        Permissions
        {perm.isElevated ? (
          <Tag tone="danger" glyph={NF.unlock} title="permissions.defaultMode is bypassPermissions">
            elevated
          </Tag>
        ) : (
          <Tag tone="ghost" glyph={NF.lock}>
            {permissionModeLabel(perm.effectiveMode)}
          </Tag>
        )}
      </h3>

      {g ? (
        <div className="permission-grant-active" role="status">
          <span>
            {isSticky ? (
              <>
                Bypass active — <strong>stays until you revert</strong>
              </>
            ) : (
              <>
                Bypass active — reverts in{" "}
                <strong>{formatRemaining(remainingMs)}</strong>
              </>
            )}
            {g.previousMode != null && (
              <span className="muted">
                {" "}
                (to {permissionModeLabel(g.previousMode)})
              </span>
            )}
          </span>
          <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center" }}>
            <DurationSelect
              value={durationSecs}
              onChange={setDurationSecs}
              disabled={busy}
            />
            <Button variant="outline" onClick={extend} disabled={busy} glyph={NF.clock}>
              {durationSecs == null ? "Make sticky" : isSticky ? "Set deadline" : "Extend"}
            </Button>
            <Button variant="solid" onClick={revert} disabled={busy} glyph={NF.lock}>
              Revert now
            </Button>
          </div>
        </div>
      ) : elevatedByHand ? (
        <p className="muted small">
          This project is in <strong>bypassPermissions</strong> from your own
          settings ({decisionLabel(perm.decidedBy)}) — not a Claudepot grant.
          Edit that file directly to change it.
        </p>
      ) : (
        <div className="permission-grant-form">
          <p className="muted small">
            Grant <strong>bypassPermissions</strong> for this project.
            Time-boxed grants auto-revert when the timer ends; the{" "}
            <strong>Never</strong> preset is a sticky grant — Claudepot
            won't auto-revert, you remove it with the Revert button when
            you're done.
          </p>
          <div style={{ display: "flex", gap: "var(--sp-8)", alignItems: "center" }}>
            <DurationSelect
              value={durationSecs}
              onChange={setDurationSecs}
              disabled={busy}
            />
            <Button variant="solid" onClick={grant} disabled={busy} glyph={NF.unlock}>
              Grant bypass
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
  return (
    <select
      className="mono"
      value={value == null ? NEVER_SENTINEL : String(value)}
      disabled={disabled}
      onChange={(e) =>
        onChange(e.target.value === NEVER_SENTINEL ? null : Number(e.target.value))
      }
      aria-label="Grant duration"
      style={{
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line-strong)",
        borderRadius: "var(--radius-sm)",
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
    case "default":
      return "CC default";
  }
}

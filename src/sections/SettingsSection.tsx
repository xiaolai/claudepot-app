import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type { NfIcon } from "../icons";
import { api } from "../api";
import { Button } from "../components/primitives/Button";
import { ExternalLink } from "../components/primitives/ExternalLink";
import { Glyph } from "../components/primitives/Glyph";
import { Tag } from "../components/primitives/Tag";
import { useDevMode } from "../hooks/useDevMode";
import { useSettingsActions } from "../hooks/useSettingsActions";
import { useTheme, type ThemeMode } from "../hooks/useTheme";
import { useToasts } from "../hooks/useToasts";
import { NF } from "../icons";
import { ToastContainer } from "../components/ToastContainer";
import { ScreenHeader } from "../shell/ScreenHeader";
import { ProtectedPathsPane } from "./settings/ProtectedPathsPane";
import type { AppStatus, CcIdentity } from "../types";
import { APP_VERSION } from "../version";

type Tab =
  | "general"
  | "appearance"
  | "activity"
  | "protected"
  | "github"
  | "locks"
  | "diagnostics"
  | "about";

// Cleanup was removed in the C-1 E consolidation: GC moved to
// Projects → Maintenance (GcCard), Rebuild session index moved to
// Sessions → Cleanup (SessionIndexRebuild). Settings no longer owns
// any repair/cleanup surfaces.
const TAB_DEFS: ReadonlyArray<{
  id: Tab;
  label: string;
  glyph: NfIcon;
  group: "core" | "prefs" | "advanced";
}> = [
  { id: "general",     label: "General",        glyph: NF.sliders,  group: "core" },
  { id: "appearance",  label: "Appearance",     glyph: NF.sun,      group: "core" },
  { id: "activity",    label: "Activity",       glyph: NF.bolt,     group: "core" },
  { id: "protected",   label: "Protected paths", glyph: NF.shield,  group: "advanced" },
  { id: "github",      label: "GitHub",         glyph: NF.key,      group: "advanced" },
  { id: "locks",       label: "Locks",          glyph: NF.lock,     group: "advanced" },
  { id: "diagnostics", label: "Diagnostics",    glyph: NF.wrench,   group: "advanced" },
  { id: "about",       label: "About",          glyph: NF.info,     group: "advanced" },
];

const SECTION_OPTIONS = [
  { value: "accounts", label: "Accounts" },
  { value: "projects", label: "Projects" },
  { value: "sessions", label: "Sessions" },
  { value: "activity", label: "Activity" },
  { value: "keys",     label: "Keys"     },
  { value: "settings", label: "Settings" },
] as const;

export function SettingsSection() {
  const { toasts, pushToast, dismissToast } = useToasts();
  const [tab, setTab] = useState<Tab>("general");
  const active = TAB_DEFS.find((t) => t.id === tab) ?? TAB_DEFS[0];

  return (
    <>
      <ScreenHeader
        title="Settings"
        subtitle="Preferences, data, and diagnostics."
      />

      <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
        <SettingsNav active={tab} onSelect={setTab} />

        <main
          style={{
            flex: 1,
            minWidth: 0,
            overflow: "auto",
            padding: "var(--sp-24) var(--sp-32) var(--sp-40)",
          }}
        >
          <h2
            style={{
              fontSize: "var(--fs-lg)",
              fontWeight: 600,
              letterSpacing: "var(--ls-tight)",
              margin: 0,
              marginBottom: "var(--sp-16)",
            }}
          >
            {active.label}
          </h2>

          {tab === "general" && <GeneralPane pushToast={pushToast} />}
          {tab === "appearance" && <AppearancePane />}
          {tab === "activity" && <ActivityPane pushToast={pushToast} />}
          {tab === "protected" && <ProtectedPathsPane pushToast={pushToast} />}
          {tab === "github" && <GithubPane pushToast={pushToast} />}
          {tab === "locks" && <LocksPane pushToast={pushToast} />}
          {tab === "diagnostics" && <DiagnosticsPane pushToast={pushToast} />}
          {tab === "about" && <AboutPane />}
        </main>
      </div>

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                         Nav                                 */
/* ──────────────────────────────────────────────────────────── */

function SettingsNav({
  active,
  onSelect,
}: {
  active: Tab;
  onSelect: (t: Tab) => void;
}) {
  const groups: { label: string; items: typeof TAB_DEFS }[] = useMemo(
    () => [
      { label: "", items: TAB_DEFS.filter((t) => t.group === "core") },
      {
        label: "Advanced",
        items: TAB_DEFS.filter((t) => t.group === "advanced"),
      },
    ],
    [],
  );

  return (
    <aside
      style={{
        width: "var(--settings-nav-width)",
        flexShrink: 0,
        borderRight: "var(--bw-hair) solid var(--line)",
        background: "var(--bg-sunken)",
        padding: "var(--sp-16) 0",
        overflow: "auto",
      }}
    >
      {groups.map((group, gi) => (
        <div key={gi} style={{ marginBottom: "var(--sp-16)" }}>
          {group.label && (
            <div
              className="mono-cap"
              style={{
                padding: "var(--sp-6) var(--sp-16)",
                color: "var(--fg-ghost)",
              }}
            >
              {group.label}
            </div>
          )}
          {group.items.map((t) => {
            const isActive = t.id === active;
            return (
              <button
                key={t.id}
                type="button"
                onClick={() => onSelect(t.id)}
                aria-current={isActive ? "page" : undefined}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--sp-10)",
                  width: "100%",
                  padding: "var(--sp-7) var(--sp-16)",
                  fontSize: "var(--fs-sm)",
                  fontWeight: isActive ? 600 : 500,
                  color: isActive ? "var(--fg)" : "var(--fg-muted)",
                  background: isActive ? "var(--bg-active)" : "transparent",
                  border: "none",
                  borderLeft: isActive
                    ? "var(--bw-strong) solid var(--accent)"
                    : "var(--bw-strong) solid transparent",
                  textAlign: "left",
                  cursor: "pointer",
                }}
              >
                <Glyph
                  g={t.glyph}
                  color={isActive ? "var(--accent)" : "currentColor"}
                  style={{ fontSize: "var(--fs-base)" }}
                />
                <span>{t.label}</span>
              </button>
            );
          })}
        </div>
      ))}
    </aside>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                       General pane                          */
/* ──────────────────────────────────────────────────────────── */

function GeneralPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const [startSection, setStartSection] = useState<string>(() => {
    try {
      return localStorage.getItem("claudepot.startSection") ?? "accounts";
    } catch {
      return "accounts";
    }
  });
  const [devMode, setDevMode] = useDevMode();
  const [hideDock, setHideDock] = useState<boolean | null>(null);
  const [launchAtLogin, setLaunchAtLogin] = useState<boolean | null>(null);
  const [isMac, setIsMac] = useState<boolean>(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [prefs, status, { isEnabled }] = await Promise.all([
          api.preferencesGet(),
          api.appStatus(),
          import("@tauri-apps/plugin-autostart"),
        ]);
        if (cancelled) return;
        setHideDock(prefs.hide_dock_icon);
        setIsMac(status.platform === "macos");
        try {
          setLaunchAtLogin(await isEnabled());
        } catch {
          setLaunchAtLogin(false);
        }
      } catch (e) {
        if (!cancelled) pushToast("error", `Preferences load failed: ${e}`);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pushToast]);

  const changeStart = useCallback(
    (v: string) => {
      setStartSection(v);
      try {
        localStorage.setItem("claudepot.startSection", v);
        pushToast("info", `Open on launch: ${v}`);
      } catch {
        // best-effort persistence
      }
    },
    [pushToast],
  );

  const toggleHideDock = useCallback(
    async (next: boolean) => {
      const prev = hideDock;
      setHideDock(next);
      try {
        await api.preferencesSetHideDockIcon(next);
        pushToast(
          "info",
          next ? "Dock icon hidden — tray-only mode." : "Dock icon restored.",
        );
      } catch (e) {
        setHideDock(prev);
        pushToast("error", `Toggle failed: ${e}`);
      }
    },
    [hideDock, pushToast],
  );

  const toggleLaunchAtLogin = useCallback(
    async (next: boolean) => {
      const prev = launchAtLogin;
      setLaunchAtLogin(next);
      try {
        const mod = await import("@tauri-apps/plugin-autostart");
        if (next) await mod.enable();
        else await mod.disable();
      } catch (e) {
        setLaunchAtLogin(prev);
        pushToast("error", `Launch-at-login toggle failed: ${e}`);
      }
    },
    [launchAtLogin, pushToast],
  );

  return (
    <SettingsGroup
      desc="Behavior that runs when Claudepot starts up, plus the diagnostic overlays you can opt into."
    >
      <Row label="Open on launch">
        <select
          value={startSection}
          onChange={(e) => changeStart(e.target.value)}
          style={selectStyle}
        >
          {SECTION_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </Row>
      <Row
        label="Launch at login"
        hint="Claudepot starts automatically when you log in."
      >
        <Toggle
          on={launchAtLogin === true}
          onChange={toggleLaunchAtLogin}
        />
      </Row>
      {isMac && (
        <Row
          label="Hide dock icon"
          hint="Tray-only mode: no dock icon, no Cmd+Tab, no app menu bar. The window still opens from the tray."
        >
          <Toggle on={hideDock === true} onChange={toggleHideDock} />
        </Row>
      )}
      <Row
        label="Developer mode"
        hint="Reveals backend command names, raw paths, and internal identifiers next to their human-facing labels."
      >
        <Toggle on={devMode} onChange={setDevMode} />
      </Row>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                     Appearance pane                         */
/* ──────────────────────────────────────────────────────────── */

function AppearancePane() {
  const { mode, resolved, setMode } = useTheme();
  const options: { value: ThemeMode; label: string; glyph?: NfIcon }[] = [
    { value: null, label: "System", glyph: NF.cpu },
    { value: "light", label: "Light", glyph: NF.sun },
    { value: "dark", label: "Dark", glyph: NF.moon },
  ];
  return (
    <SettingsGroup desc="Theme controls flow through CSS variables on the html element; no component code is aware of the active mode.">
      <Row label="Theme">
        <div
          style={{
            display: "flex",
            gap: "var(--sp-2)",
            padding: "var(--sp-2)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
          }}
        >
          {options.map((opt) => {
            const current = mode === opt.value;
            return (
              <button
                key={String(opt.value ?? "system")}
                type="button"
                onClick={() => setMode(opt.value)}
                aria-pressed={current}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--sp-6)",
                  padding: "var(--sp-4) var(--sp-10)",
                  fontSize: "var(--fs-xs)",
                  fontWeight: 500,
                  letterSpacing: "var(--ls-wide)",
                  textTransform: "uppercase",
                  color: current ? "var(--fg)" : "var(--fg-muted)",
                  background: current
                    ? "var(--bg-raised)"
                    : "transparent",
                  border: current
                    ? "var(--bw-hair) solid var(--line)"
                    : "var(--bw-hair) solid transparent",
                  borderRadius: "var(--r-1)",
                  cursor: "pointer",
                }}
              >
                {opt.glyph && <Glyph g={opt.glyph} />}
                {opt.label}
              </button>
            );
          })}
        </div>
      </Row>
      <Row label="Effective" hint="Which palette the app is rendering right now.">
        <Tag tone="accent" glyph={resolved === "dark" ? NF.moon : NF.sun}>
          {resolved}
        </Tag>
      </Row>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                        Locks pane                           */
/* ──────────────────────────────────────────────────────────── */

function LocksPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const gc = useSettingsActions(pushToast);
  return (
    <SettingsGroup desc="Force-break a stale lock file left behind by a crashed rename. Generates an audit trail.">
      <Row label="Lock file path">
        <input
          type="text"
          placeholder="/absolute/path/to/lockfile"
          value={gc.lockPath}
          onChange={(e) => gc.setLockPath(e.target.value)}
          style={{
            ...inputStyle,
            minWidth: "var(--filter-input-width)",
            width: "100%",
          }}
        />
      </Row>
      <div style={actionsStyle}>
        <Button
          variant="solid"
          danger
          onClick={gc.breakLock}
          disabled={gc.lockBusy || !gc.lockPath.trim()}
          title="Force-break the lock file and create an audit trail"
        >
          Break lock
        </Button>
      </div>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                    Diagnostics pane                         */
/* ──────────────────────────────────────────────────────────── */

function DiagnosticsPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const [appStatus, setAppStatus] = useState<AppStatus | null>(null);
  const [ccIdentity, setCcIdentity] = useState<CcIdentity | null>(null);
  const [diagBusy, setDiagBusy] = useState(false);

  const diagTokenRef = useRef(0);
  const diagMountedRef = useRef(true);
  useEffect(() => {
    diagMountedRef.current = true;
    return () => {
      diagMountedRef.current = false;
    };
  }, []);

  const loadDiagnostics = useCallback(async () => {
    const myToken = ++diagTokenRef.current;
    setDiagBusy(true);
    try {
      const [s, cc] = await Promise.all([
        api.appStatus(),
        api.currentCcIdentity(),
      ]);
      if (!diagMountedRef.current || myToken !== diagTokenRef.current)
        return;
      setAppStatus(s);
      setCcIdentity(cc);
    } catch (e) {
      if (!diagMountedRef.current || myToken !== diagTokenRef.current)
        return;
      pushToast("error", `Diagnostics failed: ${e}`);
    } finally {
      if (diagMountedRef.current && myToken === diagTokenRef.current) {
        setDiagBusy(false);
      }
    }
  }, [pushToast]);

  useEffect(() => {
    loadDiagnostics();
  }, [loadDiagnostics]);

  const copyDiagnostics = useCallback(() => {
    if (!appStatus) return;
    const lines = [
      `Claudepot diagnostics`,
      `Platform:          ${appStatus.platform}/${appStatus.arch}`,
      `CLI active:        ${appStatus.cli_active_email ?? "—"}`,
      `Desktop active:    ${appStatus.desktop_active_email ?? "—"}`,
      `Desktop installed: ${appStatus.desktop_installed ? "yes" : "no"}`,
      `Accounts:          ${appStatus.account_count}`,
      `Data dir:          ${appStatus.data_dir}`,
      `CC identity:       ${ccIdentity?.email ?? "(not signed in)"}`,
      ...(ccIdentity?.error ? [`CC identity error: ${ccIdentity.error}`] : []),
      ...(ccIdentity?.verified_at
        ? [`CC verified at:    ${ccIdentity.verified_at}`]
        : []),
    ];
    void navigator.clipboard
      .writeText(lines.join("\n"))
      .then(() => pushToast("info", "Diagnostics copied."))
      .catch((err) => pushToast("error", `Copy failed: ${err}`));
  }, [appStatus, ccIdentity, pushToast]);

  return (
    <SettingsGroup desc="Read-only view of platform, active slots, and the identity Claude Code is currently authenticated as.">
      {appStatus ? (
        <dl style={gridStyle}>
          <Kv label="Platform" value={`${appStatus.platform}/${appStatus.arch}`} mono />
          <Kv label="CLI active" value={appStatus.cli_active_email ?? "—"} />
          <Kv label="Desktop active" value={appStatus.desktop_active_email ?? "—"} />
          <Kv label="Desktop installed" value={appStatus.desktop_installed ? "yes" : "no"} />
          <Kv label="Accounts" value={String(appStatus.account_count)} />
          <Kv label="Data dir" value={appStatus.data_dir} mono />
          <Kv label="CC identity" value={ccIdentity?.email ?? "(not signed in)"} />
          {ccIdentity?.error && (
            <Kv label="CC error" value={ccIdentity.error} mono tone="warn" />
          )}
        </dl>
      ) : (
        <p className="mono-muted" style={{ fontSize: "var(--fs-xs)" }}>
          Loading…
        </p>
      )}
      <div style={actionsStyle}>
        <Button
          variant="subtle"
          glyph={NF.refresh}
          onClick={loadDiagnostics}
          disabled={diagBusy}
        >
          Refresh
        </Button>
        <Button
          variant="ghost"
          glyph={NF.copy}
          onClick={copyDiagnostics}
          disabled={!appStatus}
        >
          Copy
        </Button>
      </div>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                        About pane                           */
/* ──────────────────────────────────────────────────────────── */

function AboutPane() {
  return (
    <SettingsGroup>
      <dl style={gridStyle}>
        <Kv label="App" value="Claudepot" />
        <Kv label="Version" value={APP_VERSION} mono />
        <Kv
          label="Author"
          value={
            <ExternalLink href="https://github.com/xiaolai">
              @xiaolai
            </ExternalLink>
          }
        />
        <Kv
          label="Website"
          value={
            <ExternalLink href="https://claudepot.com">
              claudepot.com
            </ExternalLink>
          }
        />
        <Kv
          label="Design"
          value="paper-mono — JetBrains Mono NF, OKLCH palette"
        />
      </dl>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                        Activity pane                        */
/* ──────────────────────────────────────────────────────────── */

function ActivityPane({
  pushToast,
}: {
  pushToast: (k: "info" | "error", t: string) => void;
}) {
  const [loaded, setLoaded] = useState(false);
  const [enabled, setEnabled] = useState(false);
  const [hideThinking, setHideThinking] = useState(true);
  const [notifyError, setNotifyError] = useState(false);
  const [notifyIdleDone, setNotifyIdleDone] = useState(false);
  const [notifyStuckMin, setNotifyStuckMin] = useState<number | null>(null);
  const [notifySpendUsd, setNotifySpendUsd] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    api
      .preferencesGet()
      .then((p) => {
        if (cancelled) return;
        setEnabled(p.activity_enabled);
        setHideThinking(p.activity_hide_thinking);
        setNotifyError(p.notify_on_error);
        setNotifyIdleDone(p.notify_on_idle_done);
        setNotifyStuckMin(p.notify_on_stuck_minutes);
        setNotifySpendUsd(p.notify_on_spend_usd);
        setLoaded(true);
      })
      .catch((e) => {
        if (cancelled) return;
        pushToast("error", `Preferences load failed: ${e}`);
        // Flip loaded anyway — otherwise the pane is stuck on
        // "Loading…" forever after one backend hiccup. Toggles stay
        // at their safe defaults (all off) until the user interacts.
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [pushToast]);

  const toggleEnabled = useCallback(
    async (next: boolean) => {
      const prev = enabled;
      setEnabled(next);
      try {
        await api.preferencesSetActivity({ enabled: next });
        if (next) await api.sessionLiveStart();
        else await api.sessionLiveStop();
        pushToast(
          "info",
          next ? "Activity feature enabled." : "Activity feature disabled.",
        );
      } catch (e) {
        setEnabled(prev);
        pushToast("error", `Toggle failed: ${e}`);
      }
    },
    [enabled, pushToast],
  );

  const toggleHideThinking = useCallback(
    async (next: boolean) => {
      const prev = hideThinking;
      setHideThinking(next);
      try {
        await api.preferencesSetActivity({ hideThinking: next });
      } catch (e) {
        setHideThinking(prev);
        pushToast("error", `Toggle failed: ${e}`);
      }
    },
    [hideThinking, pushToast],
  );

  const setNotifyBool = useCallback(
    async (
      key: "onError" | "onIdleDone",
      setter: (v: boolean) => void,
      prev: boolean,
      next: boolean,
    ) => {
      setter(next);
      try {
        await api.preferencesSetNotifications({ [key]: next });
      } catch (e) {
        setter(prev);
        pushToast("error", `Toggle failed: ${e}`);
      }
    },
    [pushToast],
  );

  const setStuckMin = useCallback(
    async (raw: string) => {
      const parsed = raw === "" ? null : Number(raw);
      const normalized =
        parsed !== null && Number.isFinite(parsed) && parsed > 0
          ? Math.floor(parsed)
          : null;
      const prev = notifyStuckMin;
      setNotifyStuckMin(normalized);
      try {
        await api.preferencesSetNotifications({
          onStuckMinutes: normalized,
        });
      } catch (e) {
        setNotifyStuckMin(prev);
        pushToast("error", `Save failed: ${e}`);
      }
    },
    [notifyStuckMin, pushToast],
  );

  const setSpendUsd = useCallback(
    async (raw: string) => {
      const parsed = raw === "" ? null : Number(raw);
      const normalized =
        parsed !== null && Number.isFinite(parsed) && parsed > 0
          ? parsed
          : null;
      const prev = notifySpendUsd;
      setNotifySpendUsd(normalized);
      try {
        await api.preferencesSetNotifications({
          onSpendUsd: normalized,
        });
      } catch (e) {
        setNotifySpendUsd(prev);
        pushToast("error", `Save failed: ${e}`);
      }
    },
    [notifySpendUsd, pushToast],
  );

  if (!loaded) {
    return (
      <div style={{ color: "var(--fg-faint)", fontSize: "var(--fs-sm)" }}>
        Loading…
      </div>
    );
  }

  return (
    <>
      <SettingsGroup desc="The live Activity feature watches the transcript files Claude Code already writes, so you can see which of your sessions are busy, waiting, or idle at a glance. Nothing is sent anywhere; all data stays on this Mac.">
        <Row
          label="Enable Activity"
          hint="Start the live runtime. Turning this off stops all polling and clears the LIVE strip."
        >
          <Toggle on={enabled} onChange={toggleEnabled} />
        </Row>
        <Row
          label="Hide thinking by default"
          hint="Thinking blocks render as 'redacted · N chars' until you click to reveal. Privacy-forward."
        >
          <Toggle
            on={hideThinking}
            onChange={toggleHideThinking}
          />
        </Row>
      </SettingsGroup>

      <SettingsGroup desc="Toast alerts when a live session crosses one of these thresholds. One per session per minute, hard-capped. All default off.">
        <Row
          label="Alert on error burst"
          hint="At least two tool results with is_error=true inside a 60-second window."
        >
          <Toggle
            on={notifyError}
            onChange={(next) =>
              setNotifyBool("onError", setNotifyError, notifyError, next)
            }
          />
        </Row>
        <Row
          label="Alert when work completes"
          hint="A session that was busy for 2+ minutes transitions to idle."
        >
          <Toggle
            on={notifyIdleDone}
            onChange={(next) =>
              setNotifyBool(
                "onIdleDone",
                setNotifyIdleDone,
                notifyIdleDone,
                next,
              )
            }
          />
        </Row>
        <Row
          label="Alert when stuck"
          hint="Empty = off. Fires 10 minutes after the last tool result; this value is shown in the toast copy only."
        >
          <input
            type="number"
            min="1"
            step="1"
            inputMode="numeric"
            placeholder="off"
            value={notifyStuckMin ?? ""}
            onChange={(e) => setStuckMin(e.target.value)}
            style={{
              ...selectStyle,
              width: "var(--input-width-compact)",
              textAlign: "right",
              fontVariantNumeric: "tabular-nums",
            }}
          />
        </Row>
        <Row
          label="Send test notification"
          hint="Fire a sample OS notification to verify permissions and that the runtime path is wired. Doesn't toggle any preference."
        >
          <Button
            variant="ghost"
            onClick={async () => {
              try {
                const granted = await isPermissionGranted();
                if (!granted) {
                  const perm = await requestPermission();
                  if (perm !== "granted") {
                    pushToast(
                      "error",
                      "OS notification permission denied. Check System Settings.",
                    );
                    return;
                  }
                }
                sendNotification({
                  title: "Claudepot test",
                  body: "If you see this, notifications are working.",
                });
                pushToast("info", "Test notification sent.");
              } catch (e) {
                const msg = e instanceof Error ? e.message : String(e);
                pushToast("error", `Couldn't send notification: ${msg}`);
              }
            }}
          >
            Send test
          </Button>
        </Row>
        <Row
          label="Alert on spend"
          hint="Empty = off. Dollar amount to alert at when a session exceeds that cumulative spend. Requires the pricing module (ships with estimated rates for Opus / Sonnet / Haiku 4.x)."
        >
          <input
            type="number"
            min="0.01"
            step="0.01"
            inputMode="decimal"
            placeholder="off"
            value={notifySpendUsd ?? ""}
            onChange={(e) => setSpendUsd(e.target.value)}
            style={{
              ...selectStyle,
              width: "var(--input-width-compact)",
              textAlign: "right",
              fontVariantNumeric: "tabular-nums",
            }}
          />
        </Row>
      </SettingsGroup>
    </>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                         GitHub                              */
/* ──────────────────────────────────────────────────────────── */

function GithubPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const [status, setStatus] = useState<{
    present: boolean;
    last4: string | null;
    env_override: boolean;
  } | null>(null);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.settingsGithubTokenGet());
    } catch (e) {
      pushToast("error", String(e));
    }
  }, [pushToast]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const save = async () => {
    if (!input.trim()) return;
    setBusy(true);
    try {
      // Never retain the raw token in React state; pass once, clear
      // the input, fetch the status back.
      await api.settingsGithubTokenSet(input.trim());
      setInput("");
      await refresh();
      pushToast("info", "GitHub token saved.");
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      await api.settingsGithubTokenClear();
      setInput("");
      await refresh();
      pushToast("info", "GitHub token cleared.");
    } catch (e) {
      pushToast("error", String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <SettingsGroup desc="Personal Access Token for publishing session exports as gists. Stored in the Keychain; only the last four characters are ever shown.">
      <Row label="Status">
        {status?.present ? (
          <code data-testid="github-token-last4">
            …{status.last4 ?? "????"}
          </code>
        ) : (
          <span style={{ color: "var(--fg-muted)" }}>No token stored</span>
        )}
      </Row>
      {status?.env_override && (
        <Row label="Override">
          <span
            data-testid="github-env-override-note"
            style={{ color: "var(--warn)", fontSize: "var(--fs-xs)" }}
          >
            GITHUB_TOKEN env var is set — it overrides the stored token for
            uploads. Unset the env var if you want Save/Clear to take effect.
          </span>
        </Row>
      )}
      <Row label="Token">
        <input
          type="password"
          aria-label="GitHub token"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="ghp_…"
          style={inputStyle}
          autoComplete="off"
        />
      </Row>
      <div style={actionsStyle}>
        <Button variant="solid" onClick={save} disabled={busy || !input.trim()}>
          {busy ? "Saving…" : status?.present ? "Replace" : "Save"}
        </Button>
        {status?.present && (
          <Button variant="ghost" onClick={clear} disabled={busy}>
            Clear
          </Button>
        )}
      </div>
    </SettingsGroup>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                        Shared bits                          */
/* ──────────────────────────────────────────────────────────── */

function SettingsGroup({
  desc,
  children,
}: {
  desc?: string;
  children: React.ReactNode;
}) {
  return (
    <section
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-14)",
        maxWidth: "var(--content-cap-md)",
      }}
    >
      {desc && (
        <p
          style={{
            color: "var(--fg-muted)",
            fontSize: "var(--fs-xs)",
            margin: 0,
            lineHeight: "var(--lh-body)",
          }}
        >
          {desc}
        </p>
      )}
      {children}
    </section>
  );
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "var(--settings-label-col) 1fr",
        gap: "var(--sp-16)",
        alignItems: "start",
        padding: "var(--sp-8) 0",
        borderBottom: "var(--bw-hair) solid var(--line)",
      }}
    >
      <div>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}>
          {label}
        </div>
        {hint && (
          <div
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              marginTop: "var(--sp-3)",
              lineHeight: "var(--lh-body)",
            }}
          >
            {hint}
          </div>
        )}
      </div>
      <div style={{ display: "flex", alignItems: "center" }}>{children}</div>
    </div>
  );
}

function Kv({
  label,
  value,
  mono,
  tone,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
  tone?: "warn";
}) {
  return (
    <>
      <dt
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
          textAlign: "right",
        }}
      >
        {label}
      </dt>
      <dd
        style={{
          margin: 0,
          fontSize: "var(--fs-sm)",
          color: tone === "warn" ? "var(--warn)" : "var(--fg)",
          userSelect: "text",
          fontFamily: mono ? "var(--font)" : undefined,
          wordBreak: "break-all",
        }}
      >
        {value}
      </dd>
    </>
  );
}

function Toggle({
  on,
  onChange,
}: {
  on: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      style={{
        width: "var(--toggle-track-w)",
        height: "var(--toggle-track-h)",
        borderRadius: "var(--r-pill)",
        background: on ? "var(--accent)" : "var(--bg-active)",
        border: `var(--bw-hair) solid ${on ? "var(--accent)" : "var(--line-strong)"}`,
        position: "relative",
        cursor: "pointer",
        transition: "background var(--dur-base) var(--ease-linear)",
      }}
    >
      <span
        aria-hidden
        style={{
          position: "absolute",
          top: "var(--toggle-thumb-off)",
          left: on ? "var(--toggle-thumb-on)" : "var(--toggle-thumb-off)",
          width: "var(--toggle-thumb-d)",
          height: "var(--toggle-thumb-d)",
          borderRadius: "50%",
          background: "var(--bg-raised)",
          boxShadow: "var(--shadow-thumb)",
          transition: "left var(--dur-base) var(--ease-linear)",
        }}
      />
    </button>
  );
}

const inputStyle: React.CSSProperties = {
  height: "var(--row-height)",
  padding: "0 var(--sp-10)",
  fontFamily: "var(--font)",
  fontSize: "var(--fs-sm)",
  color: "var(--fg)",
  background: "var(--bg-raised)",
  border: "var(--bw-hair) solid var(--line)",
  borderRadius: "var(--r-2)",
  outline: "none",
};

const selectStyle: React.CSSProperties = {
  ...inputStyle,
  appearance: "auto",
};

const actionsStyle: React.CSSProperties = {
  display: "flex",
  gap: "var(--sp-8)",
  alignItems: "center",
};

const gridStyle: React.CSSProperties = {
  display: "grid",
  gridTemplateColumns: "minmax(var(--settings-kv-col), max-content) 1fr",
  columnGap: "var(--sp-16)",
  rowGap: "var(--sp-10)",
  margin: 0,
};

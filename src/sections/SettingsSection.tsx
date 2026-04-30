import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { NfIcon } from "../icons";
import { api } from "../api";
import { Button } from "../components/primitives/Button";
import { ExternalLink } from "../components/primitives/ExternalLink";
import { Glyph } from "../components/primitives/Glyph";
import { Tag } from "../components/primitives/Tag";
import { useSettingsActions } from "../hooks/useSettingsActions";
import { useTheme, type ThemeMode } from "../hooks/useTheme";
import { useAppState } from "../providers/AppStateProvider";
import { useUpdater, type CheckFrequency } from "../providers/UpdateProvider";
import { toastError } from "../lib/toastError";
import {
  dispatchOsNotification,
  getPermissionStatus,
  requestNotificationPermission,
  subscribePermissionStatus,
  type PermissionStatus,
} from "../lib/notify";
import { NF } from "../icons";
import { ScreenHeader } from "../shell/ScreenHeader";
import { ProtectedPathsPane } from "./settings/ProtectedPathsPane";
import { CleanupPane } from "./sessions/CleanupPane";
import { ArtifactLifecyclePane } from "./settings/ArtifactLifecyclePane";
import { TrashDrawer } from "./sessions/TrashDrawer";
import type { AppStatus, CcIdentity } from "../types";
import { APP_VERSION } from "../version";

type Tab =
  | "general"
  | "appearance"
  | "notifications"
  | "cleanup"
  | "protected"
  | "github"
  | "locks"
  | "diagnostics"
  | "about";

// Cleanup re-landed here from the (now-removed) Sessions section's
// Cleanup sub-tab when the cross-project Sessions firehose was
// folded back into per-project browsing under `Projects`. Hosts the
// session prune flow + the trash drawer + the session-index rebuild
// utility — global maintenance operations on the on-disk transcript
// store. GC of stale projects still lives in Projects → Maintenance.
const TAB_DEFS: ReadonlyArray<{
  id: Tab;
  label: string;
  glyph: NfIcon;
  group: "core" | "prefs" | "advanced";
}> = [
  { id: "general",     label: "General",        glyph: NF.sliders,  group: "core" },
  { id: "appearance",  label: "Appearance",     glyph: NF.sun,      group: "core" },
  { id: "notifications", label: "Notifications", glyph: NF.bell,     group: "core" },
  { id: "cleanup",     label: "Cleanup",        glyph: NF.trash,    group: "advanced" },
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
  { value: "keys",     label: "Keys"     },
  { value: "settings", label: "Settings" },
] as const;

export function SettingsSection() {
  const { pushToast } = useAppState();
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
          {tab === "notifications" && <NotificationsPane pushToast={pushToast} />}
          {tab === "cleanup" && <CleanupTabPane pushToast={pushToast} />}
          {tab === "protected" && <ProtectedPathsPane pushToast={pushToast} />}
          {tab === "github" && <GithubPane pushToast={pushToast} />}
          {tab === "locks" && <LocksPane pushToast={pushToast} />}
          {tab === "diagnostics" && <DiagnosticsPane pushToast={pushToast} />}
          {tab === "about" && <AboutPane />}
        </main>
      </div>
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
  const [hideDock, setHideDock] = useState<boolean | null>(null);
  const [showWindowOnStartup, setShowWindowOnStartup] = useState<
    boolean | null
  >(null);
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
        setShowWindowOnStartup(prefs.show_window_on_startup);
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

  const toggleShowWindowOnStartup = useCallback(
    async (next: boolean) => {
      const prev = showWindowOnStartup;
      setShowWindowOnStartup(next);
      try {
        await api.preferencesSetShowWindowOnStartup(next);
      } catch (e) {
        setShowWindowOnStartup(prev);
        pushToast("error", `Toggle failed: ${e}`);
      }
    },
    [showWindowOnStartup, pushToast],
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
      <Row
        label="Show window on startup"
        hint="Off: Claudepot launches into the tray with no window. Click the tray icon to open it."
      >
        <Toggle
          on={showWindowOnStartup === true}
          onChange={toggleShowWindowOnStartup}
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
      {/* Developer mode: hidden from the UI on purpose. Toggle is
          ⌃⌥⌘L (Ctrl+Alt+Cmd+L). The four-modifier combo is
          unreachable by accident and matches macOS's own
          deep-system-toggle convention (e.g. ⌃⌥⌘8 inverts colors).
          Wired in `App.tsx` so it works from any section. A toast
          confirms the new state since the toggle has no visual
          surface to mirror it. */}
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
      <Row label="Active" hint="Which palette the app is rendering right now.">
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

/** Hand-coded GitHub mark. lucide-react v1+ removed brand icons, so
 *  we ship the Octocat path inline. This is the project's brand-mark
 *  exception — see rules/design.md "Icons → Brand-mark exception".
 *  Lives next to its only call site (the About pane); uses
 *  `currentColor` so it tracks theme tokens. */
function GithubMark({ style }: { style?: React.CSSProperties }) {
  return (
    <svg
      role="img"
      aria-label="GitHub"
      viewBox="0 0 24 24"
      fill="currentColor"
      style={style}
    >
      <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.55 0-.27-.01-1-.02-1.96-3.2.69-3.87-1.54-3.87-1.54-.52-1.32-1.27-1.67-1.27-1.67-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.02 1.75 2.69 1.24 3.34.95.1-.74.4-1.24.72-1.53-2.55-.29-5.24-1.28-5.24-5.7 0-1.26.45-2.29 1.18-3.1-.12-.29-.51-1.46.11-3.05 0 0 .96-.31 3.15 1.18a10.92 10.92 0 0 1 5.74 0c2.19-1.49 3.15-1.18 3.15-1.18.62 1.59.23 2.76.11 3.05.74.81 1.18 1.84 1.18 3.1 0 4.43-2.69 5.41-5.26 5.69.41.36.78 1.06.78 2.13 0 1.54-.01 2.78-.01 3.16 0 .31.21.67.8.55C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z" />
    </svg>
  );
}

function AboutPane() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-24)" }}>
      <SettingsGroup>
        <dl style={gridStyle}>
          <Kv
            label="App"
            value={
              <span>
                clau<span style={{ color: "var(--accent)" }}>depot</span>
              </span>
            }
          />
          <Kv label="Version" value={APP_VERSION} mono />
          <Kv
            label="Author"
            value={
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--sp-12)",
                  flexWrap: "wrap",
                }}
              >
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: "var(--sp-6)",
                  }}
                >
                  <GithubMark
                    style={{
                      width: "var(--fs-base)",
                      height: "var(--fs-base)",
                      color: "var(--fg-muted)",
                    }}
                  />
                  <ExternalLink href="https://github.com/xiaolai">
                    github.com/xiaolai
                  </ExternalLink>
                </span>
                <span
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: "var(--sp-6)",
                  }}
                >
                  <Glyph
                    g={NF.globe}
                    color="var(--fg-muted)"
                    style={{ fontSize: "var(--fs-base)" }}
                  />
                  <ExternalLink href="https://lixiaolai.com">
                    lixiaolai.com
                  </ExternalLink>
                </span>
              </div>
            }
          />
          <Kv label="Design" value="paper-mono" />
        </dl>
      </SettingsGroup>
      <UpdatesPane />
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                     Updates sub-pane                        */
/* ──────────────────────────────────────────────────────────── */

/** Format a number of bytes as MB with one decimal. */
function formatMB(n: number): string {
  return (n / 1024 / 1024).toFixed(1);
}

/** Render the local datetime of a string the updater plugin handed us. */
function formatLastChecked(at: number | null): string {
  if (!at) return "Never";
  const d = new Date(at);
  return d.toLocaleString();
}

function UpdatesPane() {
  const {
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
  } = useUpdater();

  // Platform probe in flight — render nothing rather than a flicker
  // of unavailable controls.
  if (supported === null) return null;

  // Linux .deb / system install: in-place updates would race apt, so
  // the in-app updater is gated off. Surface a single-row hint
  // pointing at the Releases page so the user knows where to go.
  if (supported === false) {
    return (
      <SettingsGroup desc="In-app updates aren't available on this install — your package manager owns this binary. Check the Releases page for new versions.">
        <Row label="Updates">
          <ExternalLink href="https://github.com/xiaolai/claudepot-app/releases">
            github.com/xiaolai/claudepot-app/releases
          </ExternalLink>
        </Row>
      </SettingsGroup>
    );
  }

  const checkDisabled =
    status === "checking" ||
    status === "downloading" ||
    status === "ready";

  const showAvailableCard =
    !!updateInfo &&
    !isSkipped &&
    (status === "available" ||
      status === "downloading" ||
      status === "ready");

  return (
    <SettingsGroup desc="Claudepot checks an authenticated, minisign-signed manifest hosted on GitHub Releases. Your install only updates to versions signed by the project's release key.">
      {/* Status row + manual trigger. */}
      <Row label="Updates">
        <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-12)" }}>
          <UpdateStatusBadge
            status={status}
            updateInfo={updateInfo}
            error={error}
            isSkipped={isSkipped}
          />
          <Button
            variant="ghost"
            onClick={() => void checkNow()}
            disabled={checkDisabled}
          >
            {status === "checking" ? "Checking…" : "Check now"}
          </Button>
        </div>
      </Row>

      {/* Available / downloading / ready card. Single card switches
          its primary action by status; we never stack two cards. */}
      {showAvailableCard && updateInfo && (
        <UpdateAvailableCard
          info={updateInfo}
          status={status}
          progress={downloadProgress}
          onDownload={() => void downloadAndInstall()}
          onSkip={skipThisVersion}
          onApply={() => void applyUpdate()}
        />
      )}

      {/* When the user has skipped a version, surface a small
          inline note + an "undo" so they can change their mind. */}
      {updateInfo && isSkipped && (
        <Row
          label="Skipped"
          hint={`Version ${updateInfo.version} won't prompt again.`}
        >
          <Button variant="ghost" onClick={resetSkip}>
            Show again
          </Button>
        </Row>
      )}

      <Row
        label="Check automatically"
        hint="Background checks run after the chosen interval has elapsed since the last successful check."
      >
        <Toggle on={autoCheckEnabled} onChange={setAutoCheckEnabled} />
      </Row>

      <Row label="Frequency">
        <select
          value={checkFrequency}
          onChange={(e) =>
            setCheckFrequency(e.target.value as CheckFrequency)
          }
          disabled={!autoCheckEnabled}
          style={selectStyle}
        >
          <option value="startup">On every launch</option>
          <option value="daily">Daily</option>
          <option value="weekly">Weekly</option>
          <option value="manual">Only when I click Check now</option>
        </select>
      </Row>

      <Row label="Last checked">
        <span
          style={{ fontSize: "var(--fs-sm)", color: "var(--fg-muted)" }}
        >
          {formatLastChecked(lastCheckedAt)}
        </span>
      </Row>

      <Row label="All releases" hint="Browse the changelog and download installers for any platform.">
        <ExternalLink href="https://github.com/xiaolai/claudepot-app/releases">
          github.com/xiaolai/claudepot-app/releases
        </ExternalLink>
      </Row>
    </SettingsGroup>
  );
}

function UpdateStatusBadge({
  status,
  updateInfo,
  error,
  isSkipped,
}: {
  status: ReturnType<typeof useUpdater>["status"];
  updateInfo: ReturnType<typeof useUpdater>["updateInfo"];
  error: string | null;
  isSkipped: boolean;
}) {
  let glyph: NfIcon = NF.info;
  let color = "var(--fg-muted)";
  let label = "Idle";

  if (status === "checking") {
    glyph = NF.refresh;
    color = "var(--fg-muted)";
    label = "Checking…";
  } else if (status === "up-to-date") {
    glyph = NF.check;
    color = "var(--ok)";
    label = "You're on the latest version";
  } else if (status === "available" && updateInfo) {
    glyph = NF.download;
    color = "var(--accent)";
    label = isSkipped
      ? `v${updateInfo.version} skipped`
      : `Update available — v${updateInfo.version}`;
  } else if (status === "downloading") {
    glyph = NF.download;
    color = "var(--fg-muted)";
    label = "Downloading…";
  } else if (status === "ready") {
    glyph = NF.check;
    color = "var(--ok)";
    label = "Ready to install — restart Claudepot";
  } else if (status === "error") {
    glyph = NF.warn;
    color = "var(--danger)";
    label = error ? `Check failed: ${error}` : "Check failed";
  }

  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "var(--sp-6)",
        fontSize: "var(--fs-sm)",
        color,
      }}
      role="status"
      aria-live="polite"
    >
      <Glyph g={glyph} style={{ fontSize: "var(--fs-base)" }} />
      <span>{label}</span>
    </span>
  );
}

function UpdateAvailableCard({
  info,
  status,
  progress,
  onDownload,
  onSkip,
  onApply,
}: {
  info: NonNullable<ReturnType<typeof useUpdater>["updateInfo"]>;
  status: ReturnType<typeof useUpdater>["status"];
  progress: ReturnType<typeof useUpdater>["downloadProgress"];
  onDownload: () => void;
  onSkip: () => void;
  onApply: () => void;
}) {
  const total = progress?.total ?? 0;
  const downloaded = progress?.downloaded ?? 0;
  const pct = total > 0 ? Math.round((downloaded / total) * 100) : 0;

  return (
    <div
      style={{
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        padding: "var(--sp-14) var(--sp-16)",
        background: "var(--bg-raised)",
      }}
    >
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "flex-start",
          gap: "var(--sp-16)",
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              display: "flex",
              alignItems: "baseline",
              gap: "var(--sp-8)",
            }}
          >
            <span
              style={{
                fontSize: "var(--fs-base)",
                fontWeight: 600,
                color: "var(--fg)",
              }}
            >
              v{info.version}
            </span>
            <span
              style={{
                fontSize: "var(--fs-xs)",
                color: "var(--fg-faint)",
              }}
            >
              current v{info.currentVersion}
            </span>
            {info.pubDate && (
              <span
                style={{
                  fontSize: "var(--fs-xs)",
                  color: "var(--fg-faint)",
                }}
              >
                · {info.pubDate.slice(0, 10)}
              </span>
            )}
          </div>
          {info.notes && (
            <div
              style={{
                marginTop: "var(--sp-8)",
                fontSize: "var(--fs-sm)",
                color: "var(--fg-muted)",
                whiteSpace: "pre-wrap",
                maxHeight: "var(--sp-96, tokens.sp[96])",
                overflow: "auto",
              }}
            >
              {info.notes}
            </div>
          )}
        </div>

        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-6)",
            flexShrink: 0,
          }}
        >
          {status === "available" && (
            <>
              <Button variant="solid" onClick={onDownload}>
                Download
              </Button>
              <Button variant="ghost" onClick={onSkip}>
                Skip this version
              </Button>
            </>
          )}
          {status === "downloading" && (
            <Button variant="solid" disabled>
              Downloading…
            </Button>
          )}
          {status === "ready" && (
            <Button variant="solid" onClick={onApply}>
              Restart to update
            </Button>
          )}
        </div>
      </div>

      {status === "downloading" && (
        <div style={{ marginTop: "var(--sp-10)" }}>
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              marginBottom: "var(--sp-4)",
            }}
          >
            <span>Downloading…</span>
            <span>
              {formatMB(downloaded)} / {total > 0 ? formatMB(total) : "?"} MB
              {total > 0 && ` (${pct}%)`}
            </span>
          </div>
          <div
            style={{
              height: "var(--sp-4)",
              background: "var(--bg-active)",
              borderRadius: "var(--r-pill)",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                width: total > 0 ? `${pct}%` : "33%",
                height: "100%",
                background: "var(--accent)",
                transition: "width var(--dur-base) var(--ease-out)",
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                         Cleanup pane                         */
/* ──────────────────────────────────────────────────────────── */

function CleanupTabPane({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  // Reuse the existing CleanupPane + TrashDrawer that previously
  // lived under Sessions → Cleanup. The two-pane layout (filter +
  // plan on the left, trash on the right) carries over verbatim;
  // the sub-tab outer chrome is gone since this is just a Settings
  // sub-tab now.
  //
  // `setToast` adapts CleanupPane's single-string signature (used
  // primarily by the SessionIndexRebuild subsection for failure
  // reporting) to SettingsSection's (kind, text) API. We classify
  // by message prefix — the rebuild surface emits "rebuild failed:"
  // / "couldn't …" on errors, plain status messages otherwise — so
  // failures route to the error channel rather than getting lost
  // in the info stream.
  const setToast = useCallback(
    (msg: string) => {
      const lower = msg.toLowerCase();
      const looksLikeError =
        lower.startsWith("error") ||
        lower.startsWith("rebuild failed") ||
        lower.includes("failed:") ||
        lower.startsWith("couldn't") ||
        lower.startsWith("could not");
      pushToast(looksLikeError ? "error" : "info", msg);
    },
    [pushToast],
  );
  // Bumped when CleanupPane dispatches a prune so the TrashDrawer
  // re-fetches and shows the newly-trashed entries. We deliberately
  // do NOT pass this tick as the drawer's `key` — that would force
  // a remount + drop the drawer's local state on every action — and
  // we don't bump on the drawer's own onChange (the drawer already
  // refreshes itself after restore/empty).
  const [trashTick, setTrashTick] = useState(0);
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-32)",
      }}
    >
      <div
        style={{
          display: "flex",
          gap: "var(--sp-24)",
          alignItems: "flex-start",
        }}
      >
        <div style={{ flex: 2, minWidth: 0 }}>
          <CleanupPane
            onTrashChanged={() => setTrashTick((n) => n + 1)}
            setToast={setToast}
          />
        </div>
        <div
          style={{
            flex: 1,
            minWidth: 0,
            borderLeft: "var(--bw-hair) solid var(--line)",
            paddingLeft: "var(--sp-16)",
          }}
        >
          {/* `key={trashTick}` forces a remount whenever CleanupPane
              dispatches a prune so the drawer picks up the newly-
              trashed entries. We deliberately do NOT pass `onChange`
              here — the drawer already calls its own refresh() after
              restore / empty actions, so wiring it back to setTrashTick
              would double-bump and remount the drawer mid-action. */}
          <TrashDrawer key={trashTick} />
        </div>
      </div>

      {/* Artifact lifecycle (Disable + Trash for skills/agents/
          commands). Lives below the session cleanup row so it shares
          the same nav slot but doesn't crowd the existing layout. */}
      <div
        style={{
          borderTop: "var(--bw-hair) solid var(--line)",
          paddingTop: "var(--sp-16)",
        }}
      >
        <ArtifactLifecyclePane
          pushToast={pushToast}
          // CleanupTabPane doesn't know about the active project
          // anchor — Settings is global. Project-scoped disabled
          // artifacts still surface when the user opened the same
          // project in Config; for the global Settings view we pass
          // null so only user-scope entries appear.
          projectRoot={null}
        />
      </div>
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                      Notifications pane                      */
/* ──────────────────────────────────────────────────────────── */

function NotificationsPane({
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
  const [notifyOpDone, setNotifyOpDone] = useState(false);
  // Default-true to match the Preferences default; flipped to the
  // backend value once preferencesGet resolves. Avoids a render-flash
  // where the toggle briefly shows "off" for a feature that defaults on.
  const [notifyWaiting, setNotifyWaiting] = useState(true);
  // Mirror of preferences.notify_on_usage_thresholds. Default mirrors
  // the Rust default ([80, 90]) so the chip group renders sensibly
  // before the first preferencesGet round-trips.
  const [usageThresholds, setUsageThresholds] = useState<number[]>([80, 90]);

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
        setNotifyOpDone(p.notify_on_op_done);
        setNotifyWaiting(p.notify_on_waiting);
        setUsageThresholds(p.notify_on_usage_thresholds ?? []);
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
        // Sessions section's transcript viewer + any other consumer
        // of `useActivityPrefs` picks up the change without polling.
        window.dispatchEvent(new CustomEvent("cp-activity-prefs-changed"));
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
        window.dispatchEvent(new CustomEvent("cp-activity-prefs-changed"));
      } catch (e) {
        setHideThinking(prev);
        pushToast("error", `Toggle failed: ${e}`);
      }
    },
    [hideThinking, pushToast],
  );

  const setNotifyBool = useCallback(
    async (
      key: "onError" | "onIdleDone" | "onOpDone" | "onWaiting",
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

  const toggleUsageThreshold = useCallback(
    async (t: number) => {
      const prev = usageThresholds;
      const has = prev.includes(t);
      const next = has
        ? prev.filter((x) => x !== t)
        : [...prev, t].sort((a, b) => a - b);
      setUsageThresholds(next);
      try {
        await api.preferencesSetNotifications({ onUsageThresholds: next });
      } catch (e) {
        setUsageThresholds(prev);
        pushToast("error", `Save failed: ${e}`);
      }
    },
    [usageThresholds, pushToast],
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

      <SettingsGroup desc="OS notifications when a live session or long-running operation crosses one of these thresholds. All default off; OS notifications only fire when the Claudepot window is unfocused.">
        <NotificationPermissionRow pushToast={pushToast} />
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
          label="Alert when waiting for you"
          hint="A session paused for permission, plan-mode approval, or a clarifying answer."
        >
          <Toggle
            on={notifyWaiting}
            onChange={(next) =>
              setNotifyBool(
                "onWaiting",
                setNotifyWaiting,
                notifyWaiting,
                next,
              )
            }
          />
        </Row>
        <Row
          label="Alert when task finished"
          hint="A session busy for 2+ minutes returned to idle. The 2-minute gate filters out drive-by edits."
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
          label="Alert at usage thresholds"
          hint="Fires once per (5h / 7d window × threshold) per reset cycle for the CLI-active account. Polled every 5 min. Click any chip to toggle; empty = feature off."
        >
          <UsageThresholdChips
            thresholds={usageThresholds}
            onToggle={toggleUsageThreshold}
          />
        </Row>
        <Row
          label="Alert when long operations complete"
          hint="Verify-all, project rename, session prune/slim/share/move, account login, or clean projects — fires an OS notification when any of these terminate while the window is unfocused."
        >
          <Toggle
            on={notifyOpDone}
            onChange={(next) =>
              setNotifyBool(
                "onOpDone",
                setNotifyOpDone,
                notifyOpDone,
                next,
              )
            }
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
                const ok = await dispatchOsNotification(
                  "Claudepot test",
                  "If you see this, notifications are working.",
                  { ignoreFocus: true },
                );
                if (ok) {
                  pushToast("info", "Test notification sent.");
                  return;
                }
                // `dispatchOsNotification` returns false for several
                // reasons besides denial — probe failure (Tauri
                // plugin not ready), unknown state, or a swallowed
                // sendNotification throw. Read the live status to
                // give the user the right remediation copy.
                const status = getPermissionStatus();
                if (status === "denied") {
                  pushToast(
                    "error",
                    "OS notification permission denied. Open System Settings to re-enable.",
                  );
                } else if (status === "not-requested") {
                  pushToast(
                    "info",
                    "Notification permission was not granted. Click Request to retry.",
                  );
                } else {
                  pushToast(
                    "error",
                    "Couldn't reach the OS notification system. Try again, or check that Claudepot has Notification permission in System Settings.",
                  );
                }
              } catch (e) {
                const msg = e instanceof Error ? e.message : String(e);
                pushToast("error", `Couldn't send notification: ${msg}`);
              }
            }}
          >
            Send test
          </Button>
        </Row>
      </SettingsGroup>
    </>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                  Usage threshold chip group                  */
/* ──────────────────────────────────────────────────────────── */

/**
 * Multi-select chip group for the `notify_on_usage_thresholds`
 * preference. Empty selection = feature off; the watcher early-exits
 * on an empty list and no events fire. Choices are deliberately
 * coarse (50 / 70 / 80 / 90 / 95) — usage utilization is a slow-moving
 * signal, finer granularity wouldn't change behaviour. Custom
 * thresholds are out of scope for v1; if a user needs one, the
 * preference field accepts arbitrary integers and a future "add
 * custom" affordance can be plugged in here.
 */
const USAGE_THRESHOLD_CHOICES = [50, 70, 80, 90, 95] as const;

function UsageThresholdChips({
  thresholds,
  onToggle,
}: {
  thresholds: number[];
  onToggle: (t: number) => void;
}) {
  return (
    <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
      {USAGE_THRESHOLD_CHOICES.map((t) => {
        const on = thresholds.includes(t);
        return (
          <button
            key={t}
            type="button"
            onClick={() => onToggle(t)}
            aria-pressed={on}
            style={{
              padding: "tokens.sp[2] tokens.sp[8]",
              fontFamily: "inherit",
              fontSize: "var(--fs-xs)",
              fontVariantNumeric: "tabular-nums",
              borderRadius: "var(--radius-sm)",
              border: "tokens.sp.px solid var(--line)",
              background: on ? "var(--accent-soft)" : "transparent",
              color: on ? "var(--accent-ink)" : "var(--fg)",
              cursor: "pointer",
            }}
          >
            {t}%
          </button>
        );
      })}
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*               Notification permission status row             */
/* ──────────────────────────────────────────────────────────── */

/**
 * Surfaces the current OS notification permission state so a user
 * who toggles a notify_* preference with denied permission isn't
 * silently dropped. Subscribes to the singleton in `lib/notify.ts`
 * so the row updates in real time after a Request click.
 */
function NotificationPermissionRow({
  pushToast,
}: {
  pushToast: (k: "info" | "error", t: string) => void;
}) {
  const [status, setStatus] = useState<PermissionStatus>(() =>
    getPermissionStatus(),
  );

  useEffect(() => subscribePermissionStatus(setStatus), []);

  const label = "OS notification permission";
  let hint: string;
  switch (status) {
    case "granted":
      hint = "Granted. Claudepot can post system-level alerts when the window is unfocused.";
      break;
    case "denied":
      hint = "Denied. Re-enable Claudepot in System Settings → Notifications to receive alerts.";
      break;
    case "not-requested":
      hint = "Not yet requested. Click Request to enable system-level alerts.";
      break;
    case "unknown":
    default:
      hint = "Probing OS permission state…";
      break;
  }

  return (
    <Row label={label} hint={hint}>
      {status === "granted" && <Tag>Granted</Tag>}
      {status === "denied" && <Tag tone="danger">Denied</Tag>}
      {status === "not-requested" && (
        <Button
          variant="ghost"
          onClick={async () => {
            const next = await requestNotificationPermission();
            if (next === "granted") {
              pushToast("info", "Notifications enabled.");
            } else if (next === "denied") {
              pushToast(
                "error",
                "Notification permission denied. Open System Settings to re-enable.",
              );
            }
          }}
        >
          Request
        </Button>
      )}
      {status === "unknown" && <Tag tone="ghost">Unknown</Tag>}
    </Row>
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
      // Route through toastError so the redactSecrets pipeline scrubs
      // any `sk-ant-*` / `ghp_*` blob the backend might echo back. The
      // toast lingers in the DOM (and now in the status-bar echo) so
      // raw stringification is a leak surface.
      toastError(pushToast, "GitHub token load failed", e);
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
      toastError(pushToast, "GitHub token save failed", e);
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
      toastError(pushToast, "GitHub token clear failed", e);
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

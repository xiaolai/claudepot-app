import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { NfIcon } from "../icons";
import { api } from "../api";
import { Button } from "../components/primitives/Button";
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

type Tab =
  | "general"
  | "appearance"
  | "claudemd"
  | "mcp"
  | "apikeys"
  | "protected"
  | "cleanup"
  | "locks"
  | "diagnostics"
  | "about";

const TAB_DEFS: ReadonlyArray<{
  id: Tab;
  label: string;
  glyph: NfIcon;
  group: "core" | "prefs" | "advanced";
}> = [
  { id: "general",     label: "General",        glyph: NF.sliders,  group: "core" },
  { id: "appearance",  label: "Appearance",     glyph: NF.sun,      group: "core" },
  { id: "claudemd",    label: "CLAUDE.md",      glyph: NF.fileMd,   group: "core" },
  { id: "mcp",         label: "MCP servers",    glyph: NF.server,   group: "core" },
  { id: "apikeys",     label: "API keys",       glyph: NF.key,      group: "core" },
  { id: "protected",   label: "Protected paths", glyph: NF.shield,  group: "advanced" },
  { id: "cleanup",     label: "Cleanup",        glyph: NF.trash,    group: "advanced" },
  { id: "locks",       label: "Locks",          glyph: NF.lock,     group: "advanced" },
  { id: "diagnostics", label: "Diagnostics",    glyph: NF.wrench,   group: "advanced" },
  { id: "about",       label: "About",          glyph: NF.info,     group: "advanced" },
];

const SECTION_OPTIONS = [
  { value: "accounts", label: "Accounts" },
  { value: "projects", label: "Projects" },
  { value: "sessions", label: "Sessions" },
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
          {tab === "claudemd" && <StubPane name="CLAUDE.md editor" />}
          {tab === "mcp" && <StubPane name="MCP servers" />}
          {tab === "apikeys" && <StubPane name="API keys" />}
          {tab === "protected" && <ProtectedPathsPane pushToast={pushToast} />}
          {tab === "cleanup" && <CleanupPane pushToast={pushToast} />}
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
/*                     Unbacked stubs                          */
/* ──────────────────────────────────────────────────────────── */

function StubPane({ name }: { name: string }) {
  return (
    <div
      role="status"
      style={{
        padding: "var(--sp-48) var(--sp-24)",
        textAlign: "center",
        color: "var(--fg-muted)",
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-8)",
        alignItems: "center",
      }}
    >
      <Glyph g={NF.wrench} size="var(--sp-32)" color="var(--fg-ghost)" />
      <p style={{ margin: 0, fontSize: "var(--fs-base)" }}>
        {name} — not yet implemented.
      </p>
      <p
        style={{
          margin: 0,
          fontSize: "var(--fs-xs)",
          color: "var(--fg-faint)",
          maxWidth: "var(--content-cap-sm)",
          lineHeight: "var(--lh-body)",
        }}
      >
        Reserved for a future phase; the backend doesn't expose the
        underlying surfaces yet.
      </p>
    </div>
  );
}

/* ──────────────────────────────────────────────────────────── */
/*                      Cleanup pane                           */
/* ──────────────────────────────────────────────────────────── */

function CleanupPane({
  pushToast,
}: {
  pushToast: (t: "info" | "error", msg: string) => void;
}) {
  const gc = useSettingsActions(pushToast);
  return (
    <SettingsGroup desc="Remove abandoned rename journals and old recovery snapshots. Preview first — the delete is irreversible.">
      <Row label="Older than">
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--sp-8)",
          }}
        >
          <input
            type="number"
            min={1}
            max={365}
            value={gc.gcDays}
            onChange={(e) => gc.setGcDays(Number(e.target.value))}
            style={{ ...inputStyle, width: "var(--sp-80)" }}
          />
          <span className="mono-faint">days</span>
        </div>
      </Row>
      <div style={actionsStyle}>
        <Button
          variant="subtle"
          onClick={gc.gcDryRun}
          disabled={gc.gcBusy}
          title="Preview what GC would remove"
        >
          Preview
        </Button>
        <Button
          variant="solid"
          danger
          onClick={gc.gcExecute}
          disabled={gc.gcBusy || !gc.gcResult}
          title="Permanently remove abandoned journals and old snapshots"
        >
          Execute GC
        </Button>
      </div>
      {gc.gcResult && (
        <div
          style={{
            padding: "var(--sp-10) var(--sp-12)",
            background: "var(--bg-sunken)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
          }}
        >
          Would remove:{" "}
          <strong style={{ color: "var(--fg)" }}>
            {gc.gcResult.removed_journals}
          </strong>{" "}
          journals,{" "}
          <strong style={{ color: "var(--fg)" }}>
            {gc.gcResult.removed_snapshots}
          </strong>{" "}
          snapshots
        </div>
      )}
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
    navigator.clipboard.writeText(lines.join("\n"));
    pushToast("info", "Diagnostics copied.");
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
        <Kv label="Version" value="0.1.0" mono />
        <Kv
          label="Design"
          value="paper-mono — JetBrains Mono NF, OKLCH palette"
        />
      </dl>
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

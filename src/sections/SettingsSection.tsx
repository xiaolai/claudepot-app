import { useCallback, useEffect, useRef, useState } from "react";
import { Icon } from "../components/Icon";
import { api } from "../api";
import { useToasts } from "../hooks/useToasts";
import { useSettingsActions } from "../hooks/useSettingsActions";
import { ToastContainer } from "../components/ToastContainer";
import type { AppStatus, CcIdentity } from "../types";

const SECTION_OPTIONS = [
  { value: "accounts", label: "Accounts" },
  { value: "projects", label: "Projects" },
  { value: "settings", label: "Settings" },
] as const;

type SettingsPane = "startup" | "cleanup" | "locks" | "diagnostics" | "about";

const SETTINGS_PANES: ReadonlyArray<{
  id: SettingsPane;
  label: string;
  icon: "play" | "trash-2" | "lock" | "stethoscope" | "info";
}> = [
  { id: "startup", label: "Startup", icon: "play" },
  { id: "cleanup", label: "Cleanup", icon: "trash-2" },
  { id: "locks", label: "Locks", icon: "lock" },
  { id: "diagnostics", label: "Diagnostics", icon: "stethoscope" },
  { id: "about", label: "About", icon: "info" },
];

export function SettingsSection() {
  const { toasts, pushToast, dismissToast } = useToasts();
  const [pane, setPane] = useState<SettingsPane>("startup");
  // Separate from `claudepot.activeSection` (last-visited) — this one
  // is the explicit "Open on launch" preference. Normal navigation must
  // not overwrite it, otherwise clicking around the app silently
  // changes what the user set here.
  const [startSection, setStartSection] = useState<string>(() => {
    try { return localStorage.getItem("claudepot.startSection") ?? "accounts"; }
    catch { return "accounts"; }
  });
  const gc = useSettingsActions(pushToast);

  const handleStartChange = useCallback((v: string) => {
    setStartSection(v);
    try { localStorage.setItem("claudepot.startSection", v); } catch { /* best-effort */ }
  }, []);

  // Read-only diagnostics — equivalent of the CLI's `doctor` / `status`.
  // Populated on mount; refresh via the panel's own button.
  const [appStatus, setAppStatus] = useState<AppStatus | null>(null);
  const [ccIdentity, setCcIdentity] = useState<CcIdentity | null>(null);
  const [diagBusy, setDiagBusy] = useState(false);

  // Audit M16: token-sequenced + unmount-guarded reload. Diagnostics
  // can be triggered on mount and again from the Refresh button; a
  // slower earlier Promise.all could resolve after a newer one and
  // replace fresher data. Also protect against setState after unmount.
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
      if (!diagMountedRef.current || myToken !== diagTokenRef.current) return;
      setAppStatus(s);
      setCcIdentity(cc);
    } catch (e) {
      if (!diagMountedRef.current || myToken !== diagTokenRef.current) return;
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

  const activePaneDef = SETTINGS_PANES.find((p) => p.id === pane) ?? SETTINGS_PANES[0];

  return (
    <>
      <aside className="sidebar settings-sidebar">
        <div className="sidebar-header">
          <div className="sidebar-title">Settings</div>
        </div>
        <ul className="sidebar-list" role="listbox" aria-label="Settings panes">
          {SETTINGS_PANES.map((p) => {
            const isActive = p.id === pane;
            return (
              <li
                key={p.id}
                role="option"
                aria-selected={isActive}
                className={`sidebar-item${isActive ? " active" : ""}`}
                tabIndex={0}
                onClick={() => setPane(p.id)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setPane(p.id);
                  }
                }}
              >
                <div className="sidebar-item-row">
                  <Icon name={p.icon} size={12} />
                  <span className="sidebar-item-name">{p.label}</span>
                </div>
              </li>
            );
          })}
        </ul>
      </aside>
      <main className="content settings-view">
        <h2 className="settings-heading">{activePaneDef.label}</h2>

        {pane === "startup" && (
          <section className="settings-group">
            <label className="settings-row">
              <span>Open on launch</span>
              <select className="settings-select" value={startSection}
                onChange={(e) => handleStartChange(e.target.value)}>
                {SECTION_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>{o.label}</option>
                ))}
              </select>
            </label>
          </section>
        )}

        {pane === "cleanup" && (
          <section className="settings-group">
            <p className="muted settings-desc">Remove abandoned journals and old recovery snapshots.</p>
            <label className="settings-row">
              <span>Older than</span>
              <div className="settings-input-group">
                <input type="number" className="settings-input" min={1} max={365}
                  value={gc.gcDays} onChange={(e) => gc.setGcDays(Number(e.target.value))} />
                <span className="muted">days</span>
              </div>
            </label>
            <div className="settings-actions">
              <button onClick={gc.gcDryRun} disabled={gc.gcBusy}
                title="Preview what GC would remove without deleting">Preview</button>
              <button className="danger" onClick={gc.gcExecute} disabled={gc.gcBusy || !gc.gcResult}
                title="Permanently remove abandoned journals and old snapshots">Execute GC</button>
            </div>
            {gc.gcResult && (
              <div className="settings-result">
                Would remove: {gc.gcResult.removed_journals} journals, {gc.gcResult.removed_snapshots} snapshots
              </div>
            )}
          </section>
        )}

        {pane === "locks" && (
          <section className="settings-group">
            <p className="muted settings-desc">Force-break a lock file left by a crashed rename.</p>
            <div className="settings-row">
              <input type="text" className="settings-input wide" placeholder="Lock file path…"
                value={gc.lockPath} onChange={(e) => gc.setLockPath(e.target.value)} />
              <button onClick={gc.breakLock} disabled={gc.lockBusy || !gc.lockPath.trim()}
                title="Force-break the lock file and create an audit trail">Break</button>
            </div>
          </section>
        )}

        {pane === "diagnostics" && (
          <section className="settings-group">
            <p className="muted settings-desc">
              Read-only view of platform, active slots, and the identity
              Claude Code is currently authenticated as. Equivalent of the
              CLI's <code>doctor</code> / <code>status</code> output.
            </p>
            {appStatus ? (
              <dl className="settings-about-grid">
                <dt>Platform</dt>
                <dd className="mono selectable">
                  {appStatus.platform}/{appStatus.arch}
                </dd>
                <dt>CLI active</dt>
                <dd className="selectable">
                  {appStatus.cli_active_email ?? "—"}
                </dd>
                <dt>Desktop active</dt>
                <dd className="selectable">
                  {appStatus.desktop_active_email ?? "—"}
                </dd>
                <dt>Desktop installed</dt>
                <dd>{appStatus.desktop_installed ? "yes" : "no"}</dd>
                <dt>Accounts</dt>
                <dd>{appStatus.account_count}</dd>
                <dt>Data dir</dt>
                <dd className="mono small selectable">{appStatus.data_dir}</dd>
                <dt>CC identity</dt>
                <dd className="selectable">
                  {ccIdentity?.email ?? <em className="muted">not signed in</em>}
                </dd>
                {ccIdentity?.error && (
                  <>
                    <dt>CC error</dt>
                    <dd className="mono small bad">{ccIdentity.error}</dd>
                  </>
                )}
              </dl>
            ) : (
              <p className="muted small">Loading…</p>
            )}
            <div className="settings-actions">
              <button onClick={loadDiagnostics} disabled={diagBusy}
                title="Re-fetch diagnostics">
                Refresh
              </button>
              <button onClick={copyDiagnostics} disabled={!appStatus}
                title="Copy all diagnostics to clipboard">
                <Icon name="copy" size={13} /> Copy
              </button>
            </div>
          </section>
        )}

        {pane === "about" && (
          <section className="settings-group about">
            <dl className="settings-about-grid">
              <dt>App</dt><dd>Claudepot</dd>
              <dt>Version</dt><dd className="mono">0.1.0</dd>
            </dl>
          </section>
        )}
      </main>
      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </>
  );
}


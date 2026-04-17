import { useCallback, useState } from "react";
import { Settings, Trash2, Lock, Info } from "lucide-react";
import { useToasts } from "../hooks/useToasts";
import { useSettingsActions } from "../hooks/useSettingsActions";
import { ToastContainer } from "../components/ToastContainer";

const SECTION_OPTIONS = [
  { value: "accounts", label: "Accounts" },
  { value: "projects", label: "Projects" },
] as const;

export function SettingsSection() {
  const { toasts, pushToast, dismissToast } = useToasts();
  const [startSection, setStartSection] = useState<string>(() => {
    try { return localStorage.getItem("claudepot.activeSection") ?? "accounts"; }
    catch { return "accounts"; }
  });
  const gc = useSettingsActions(pushToast);

  const handleStartChange = useCallback((v: string) => {
    setStartSection(v);
    try { localStorage.setItem("claudepot.activeSection", v); } catch { /* best-effort */ }
  }, []);

  return (
    <>
      <main className="content settings-view">
        <h2 className="settings-heading">Settings</h2>

        <section className="settings-group">
          <h3 className="settings-group-title">Startup</h3>
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

        <section className="settings-group">
          <h3 className="settings-group-title"><Trash2 size={14} /> Garbage Collection</h3>
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

        <section className="settings-group">
          <h3 className="settings-group-title"><Lock size={14} /> Break Stale Lock</h3>
          <p className="muted settings-desc">Force-break a lock file left by a crashed rename.</p>
          <div className="settings-row">
            <input type="text" className="settings-input wide" placeholder="Lock file path…"
              value={gc.lockPath} onChange={(e) => gc.setLockPath(e.target.value)} />
            <button onClick={gc.breakLock} disabled={gc.lockBusy || !gc.lockPath.trim()}
              title="Force-break the lock file and create an audit trail">Break</button>
          </div>
        </section>

        <section className="settings-group about">
          <h3 className="settings-group-title"><Info size={14} /> About</h3>
          <dl className="settings-about-grid">
            <dt>App</dt><dd>Claudepot</dd>
            <dt>Version</dt><dd className="mono">0.1.0</dd>
          </dl>
        </section>
      </main>
      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </>
  );
}

SettingsSection.icon = <Settings />;
SettingsSection.label = "Settings";

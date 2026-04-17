import { useCallback, useState } from "react";
import { Settings, Trash2, Lock, Info } from "lucide-react";
import { api } from "../api";
import type { GcOutcome } from "../types";
import { useToasts } from "../hooks/useToasts";
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

  // GC state
  const [gcDays, setGcDays] = useState(30);
  const [gcBusy, setGcBusy] = useState(false);
  const [gcResult, setGcResult] = useState<GcOutcome | null>(null);

  // Break lock state
  const [lockPath, setLockPath] = useState("");
  const [lockBusy, setLockBusy] = useState(false);

  const handleStartSectionChange = useCallback((value: string) => {
    setStartSection(value);
    try { localStorage.setItem("claudepot.activeSection", value); }
    catch { /* best-effort */ }
  }, []);

  const handleGcDryRun = useCallback(async () => {
    setGcBusy(true);
    try {
      const result = await api.repairGc(gcDays, true);
      setGcResult(result);
    } catch (e) { pushToast("error", `GC preview failed: ${e}`); }
    finally { setGcBusy(false); }
  }, [gcDays, pushToast]);

  const handleGcExecute = useCallback(async () => {
    setGcBusy(true);
    try {
      const result = await api.repairGc(gcDays, false);
      setGcResult(null);
      pushToast("info", `GC: removed ${result.removed_journals} journals, ${result.removed_snapshots} snapshots, freed ${formatBytes(result.bytes_freed)}`);
    } catch (e) { pushToast("error", `GC failed: ${e}`); }
    finally { setGcBusy(false); }
  }, [gcDays, pushToast]);

  const handleBreakLock = useCallback(async () => {
    if (!lockPath.trim()) return;
    setLockBusy(true);
    try {
      const result = await api.repairBreakLock(lockPath.trim());
      pushToast("info", `Lock broken — PID ${result.prior_pid} (${result.prior_hostname}). Audit: ${result.audit_path}`);
      setLockPath("");
    } catch (e) { pushToast("error", `Break lock failed: ${e}`); }
    finally { setLockBusy(false); }
  }, [lockPath, pushToast]);

  return (
    <>
      <main className="content settings-view">
        <h2 className="settings-heading">Settings</h2>

        {/* Start section */}
        <section className="settings-group">
          <h3 className="settings-group-title">Startup</h3>
          <label className="settings-row">
            <span>Open on launch</span>
            <select
              className="settings-select"
              value={startSection}
              onChange={(e) => handleStartSectionChange(e.target.value)}
            >
              {SECTION_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>{o.label}</option>
              ))}
            </select>
          </label>
        </section>

        {/* GC */}
        <section className="settings-group">
          <h3 className="settings-group-title">
            <Trash2 size={14} /> Garbage Collection
          </h3>
          <p className="muted settings-desc">
            Remove abandoned journals and old recovery snapshots.
          </p>
          <label className="settings-row">
            <span>Older than</span>
            <div className="settings-input-group">
              <input
                type="number"
                className="settings-input"
                min={1}
                max={365}
                value={gcDays}
                onChange={(e) => setGcDays(Number(e.target.value))}
              />
              <span className="muted">days</span>
            </div>
          </label>
          <div className="settings-actions">
            <button onClick={handleGcDryRun} disabled={gcBusy}>
              Preview
            </button>
            <button className="danger" onClick={handleGcExecute} disabled={gcBusy || !gcResult}>
              Execute GC
            </button>
          </div>
          {gcResult && (
            <div className="settings-result">
              Would remove: {gcResult.removed_journals} journals, {gcResult.removed_snapshots} snapshots ({formatBytes(gcResult.bytes_freed)})
            </div>
          )}
        </section>

        {/* Break lock */}
        <section className="settings-group">
          <h3 className="settings-group-title">
            <Lock size={14} /> Break Stale Lock
          </h3>
          <p className="muted settings-desc">
            Force-break a lock file left by a crashed rename. Creates an audit trail.
          </p>
          <div className="settings-row">
            <input
              type="text"
              className="settings-input wide"
              placeholder="Lock file path…"
              value={lockPath}
              onChange={(e) => setLockPath(e.target.value)}
            />
            <button onClick={handleBreakLock} disabled={lockBusy || !lockPath.trim()}>
              Break
            </button>
          </div>
        </section>

        {/* About */}
        <section className="settings-group about">
          <h3 className="settings-group-title">
            <Info size={14} /> About
          </h3>
          <dl className="settings-about-grid">
            <dt>App</dt>
            <dd>Claudepot</dd>
            <dt>Version</dt>
            <dd className="mono">0.1.0</dd>
            <dt>Platform</dt>
            <dd className="mono">{navigator.userAgent.includes("Mac") ? "macOS" : navigator.userAgent.includes("Win") ? "Windows" : "Linux"}</dd>
          </dl>
        </section>
      </main>
      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
    </>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

SettingsSection.icon = <Settings />;
SettingsSection.label = "Settings";

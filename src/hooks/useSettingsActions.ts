import { useCallback, useState } from "react";
import { api } from "../api";
import type { GcOutcome } from "../types";

export function useSettingsActions(pushToast: (kind: "info" | "error", text: string) => void) {
  const [gcDays, setGcDays] = useState(30);
  const [gcBusy, setGcBusy] = useState(false);
  const [gcResult, setGcResult] = useState<GcOutcome | null>(null);
  const [lockPath, setLockPath] = useState("");
  const [lockBusy, setLockBusy] = useState(false);

  const gcDryRun = useCallback(async () => {
    setGcBusy(true);
    try {
      setGcResult(await api.repairGc(gcDays, true));
    } catch (e) { pushToast("error", `GC preview failed: ${e}`); }
    finally { setGcBusy(false); }
  }, [gcDays, pushToast]);

  const gcExecute = useCallback(async () => {
    setGcBusy(true);
    try {
      const r = await api.repairGc(gcDays, false);
      setGcResult(null);
      pushToast("info", `GC: removed ${r.removed_journals} journals, ${r.removed_snapshots} snapshots, freed ${formatBytes(r.bytes_freed)}`);
    } catch (e) { pushToast("error", `GC failed: ${e}`); }
    finally { setGcBusy(false); }
  }, [gcDays, pushToast]);

  const breakLock = useCallback(async () => {
    if (!lockPath.trim()) return;
    setLockBusy(true);
    try {
      const r = await api.repairBreakLock(lockPath.trim());
      pushToast("info", `Lock broken — PID ${r.prior_pid} (${r.prior_hostname}). Audit: ${r.audit_path}`);
      setLockPath("");
    } catch (e) { pushToast("error", `Break lock failed: ${e}`); }
    finally { setLockBusy(false); }
  }, [lockPath, pushToast]);

  return { gcDays, setGcDays, gcBusy, gcResult, gcDryRun, gcExecute, lockPath, setLockPath, lockBusy, breakLock };
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

import { useCallback, useState } from "react";
import { api } from "../api";
import { formatSize } from "../lib/format";
import { i18n } from "../lib/i18n";
import { renderError } from "../lib/i18n-error";
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
    } catch (e) {
      pushToast("error", renderError(e, i18n.t("maintenance.gcPreviewFailed")));
    }
    finally { setGcBusy(false); }
  }, [gcDays, pushToast]);

  const gcExecute = useCallback(async () => {
    setGcBusy(true);
    try {
      const r = await api.repairGc(gcDays, false);
      setGcResult(null);
      pushToast(
        "info",
        i18n.t("maintenance.gcDone", {
          journals: r.removed_journals,
          snapshots: r.removed_snapshots,
          size: formatSize(r.bytes_freed),
        }),
      );
    } catch (e) {
      pushToast("error", renderError(e, i18n.t("maintenance.gcFailed")));
    }
    finally { setGcBusy(false); }
  }, [gcDays, pushToast]);

  const breakLock = useCallback(async () => {
    if (!lockPath.trim()) return;
    setLockBusy(true);
    try {
      const r = await api.repairBreakLock(lockPath.trim());
      pushToast(
        "info",
        i18n.t("maintenance.lockBroken", {
          pid: r.prior_pid,
          host: r.prior_hostname,
          auditPath: r.audit_path,
        }),
      );
      setLockPath("");
    } catch (e) {
      pushToast("error", renderError(e, i18n.t("maintenance.breakLockFailed")));
    }
    finally { setLockBusy(false); }
  }, [lockPath, pushToast]);

  return { gcDays, setGcDays, gcBusy, gcResult, gcDryRun, gcExecute, lockPath, setLockPath, lockBusy, breakLock };
}

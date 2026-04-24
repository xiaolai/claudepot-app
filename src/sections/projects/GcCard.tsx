import { useState, useCallback } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import type { GcOutcome } from "../../types";

/**
 * Time-based GC for abandoned rename journals + old recovery
 * snapshots. Moved from the Settings → Cleanup tab as part of the
 * C-1 E consolidation — project-domain cleanup belongs in the
 * project-domain maintenance view. Preview is idempotent and
 * mandatory before Execute. Execute is irreversible.
 */
export function GcCard({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const [days, setDays] = useState(30);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<GcOutcome | null>(null);

  const dryRun = useCallback(async () => {
    setBusy(true);
    try {
      setResult(await api.repairGc(days, true));
    } catch (e) {
      pushToast("error", `GC preview failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }, [days, pushToast]);

  const execute = useCallback(async () => {
    setBusy(true);
    try {
      const r = await api.repairGc(days, false);
      setResult(null);
      pushToast(
        "info",
        `GC: removed ${r.removed_journals} journals, ${r.removed_snapshots} snapshots, freed ${formatBytes(
          r.bytes_freed,
        )}`,
      );
    } catch (e) {
      pushToast("error", `GC failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }, [days, pushToast]);

  return (
    <section className="maintenance-section">
      <div className="maintenance-section-header">
        <h2>Garbage-collect old repair data</h2>
      </div>
      <p className="muted maintenance-desc">
        Remove abandoned rename journals and recovery snapshots older
        than the threshold. Preview first — the execute step is
        irreversible.
      </p>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
          marginBottom: "var(--sp-12)",
        }}
      >
        <label htmlFor="gc-days" className="muted small">
          Older than
        </label>
        <input
          id="gc-days"
          type="number"
          min={1}
          max={365}
          value={days}
          onChange={(e) => setDays(Number(e.target.value))}
          style={{
            width: "var(--sp-72)",
            padding: "var(--sp-4) var(--sp-6)",
            fontSize: "var(--fs-sm)",
            fontFamily: "var(--font)",
            border: "var(--bw-hair) solid var(--line)",
            borderRadius: "var(--r-2)",
            background: "var(--bg)",
            color: "var(--fg)",
            fontVariantNumeric: "tabular-nums",
          }}
        />
        <span className="muted small">days</span>
      </div>
      <div style={{ display: "flex", gap: "var(--sp-8)" }}>
        <Button
          variant="ghost"
          onClick={dryRun}
          disabled={busy}
          title="Preview what GC would remove"
        >
          Preview
        </Button>
        <Button
          variant="solid"
          danger
          onClick={execute}
          disabled={busy || !result}
          title={result ? undefined : "Run Preview first"}
        >
          Execute GC
        </Button>
      </div>
      {result && (
        <div
          style={{
            marginTop: "var(--sp-12)",
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
            {result.removed_journals}
          </strong>{" "}
          journals,{" "}
          <strong style={{ color: "var(--fg)" }}>
            {result.removed_snapshots}
          </strong>{" "}
          snapshots
        </div>
      )}
    </section>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

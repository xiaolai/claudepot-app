import { useCallback, useState } from "react";
import { api } from "../../../api";
import { Button } from "../../../components/primitives/Button";
import { ConfirmDialog } from "../../../components/ConfirmDialog";

/**
 * Truncates the persistent session-index cache at
 * `~/.claudepot/sessions.db`. Moved from Settings → Cleanup as part
 * of the C-1 E consolidation — session cleanup belongs in Sessions.
 */
export function SessionIndexRebuild({
  setToast,
}: {
  /** Sessions-style toast setter — matches the pane's own pattern so
   *  no extra useToasts instance is needed here. */
  setToast: (msg: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);

  const rebuild = useCallback(async () => {
    setConfirming(false);
    setBusy(true);
    try {
      await api.sessionIndexRebuild();
      setToast(
        "Session index cleared. The next load will re-parse every transcript.",
      );
    } catch (e) {
      setToast(`Rebuild failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }, [setToast]);

  return (
    <>
      <section
        style={{
          padding: "var(--sp-16) var(--sp-24)",
          border: "var(--bw-hair) solid var(--line)",
          borderRadius: "var(--r-3)",
          background: "var(--bg-raised)",
          display: "flex",
          flexDirection: "column",
          gap: "var(--sp-10)",
        }}
      >
        <h3
          style={{
            margin: 0,
            fontSize: "var(--fs-sm)",
            fontWeight: 600,
            color: "var(--fg)",
          }}
        >
          Rebuild session index
        </h3>
        <p
          style={{
            margin: 0,
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            lineHeight: "var(--lh-body)",
          }}
        >
          Drops every cached row in <code className="mono">~/.claudepot/sessions.db</code>.
          Safe — no transcripts or credentials are touched; only derived
          rows are removed. The next Sessions open re-parses from cold,
          which can take tens of seconds on a large account.
        </p>
        <div>
          <Button
            variant="ghost"
            onClick={() => setConfirming(true)}
            disabled={busy}
          >
            Rebuild
          </Button>
        </div>
      </section>

      {confirming && (
        <ConfirmDialog
          title="Rebuild session index?"
          body={
            <p style={{ margin: 0 }}>
              This clears the persistent cache at{" "}
              <code className="mono">~/.claudepot/sessions.db</code>.
              The next Sessions open will be slow while every transcript
              is re-parsed.
            </p>
          }
          confirmLabel="Rebuild"
          onCancel={() => setConfirming(false)}
          onConfirm={() => void rebuild()}
        />
      )}
    </>
  );
}

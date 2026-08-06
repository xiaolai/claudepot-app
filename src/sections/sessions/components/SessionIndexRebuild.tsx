import { useCallback, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../../api";
import { renderError } from "../../../lib/i18n-error";
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
   *  no extra useToasts instance is needed here. `kind` defaults to
   *  "info"; the caller states failure explicitly rather than leaving a
   *  downstream adapter to infer it from the wording. */
  setToast: (msg: string, kind?: "info" | "error") => void;
}) {
  const { t } = useTranslation("sessions");
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);

  const rebuild = useCallback(async () => {
    setConfirming(false);
    setBusy(true);
    try {
      await api.sessionIndexRebuild();
      setToast(t("cleanup.rebuildDone"));
    } catch (e) {
      setToast(t("cleanup.rebuildFailed", { error: renderError(e) }), "error");
    } finally {
      setBusy(false);
    }
  }, [setToast, t]);

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
          {t("cleanup.rebuildTitle")}
        </h3>
        <p
          style={{
            margin: 0,
            fontSize: "var(--fs-xs)",
            color: "var(--fg-muted)",
            lineHeight: "var(--lh-body)",
          }}
        >
          <Trans
            ns="sessions"
            i18nKey="cleanup.rebuildBody"
            components={{ code: <code className="mono" /> }}
          />
        </p>
        <div>
          <Button
            variant="ghost"
            onClick={() => setConfirming(true)}
            disabled={busy}
          >
            {t("cleanup.rebuild")}
          </Button>
        </div>
      </section>

      {confirming && (
        <ConfirmDialog
          title={t("cleanup.rebuildConfirmTitle")}
          body={
            <p style={{ margin: 0 }}>
              <Trans
                ns="sessions"
                i18nKey="cleanup.rebuildConfirmBody"
                components={{ code: <code className="mono" /> }}
              />
            </p>
          }
          confirmLabel={t("cleanup.rebuild")}
          onCancel={() => setConfirming(false)}
          onConfirm={() => void rebuild()}
        />
      )}
    </>
  );
}

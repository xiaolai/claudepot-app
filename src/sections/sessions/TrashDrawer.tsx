import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import type { TrashListing } from "../../types";

/**
 * Right-side panel that lists trash batches and offers restore / empty.
 * Self-hosted — the parent just mounts it when the tab is active. Polls
 * the trash only on mount + after each action.
 */
export function TrashDrawer({ onChange }: { onChange?: () => void }) {
  const { t } = useTranslation("sessions");
  const [listing, setListing] = useState<TrashListing | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setErr(null);
    try {
      setListing(await api.sessionTrashList());
    } catch (e) {
      setErr(renderError(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const restore = useCallback(
    async (id: string) => {
      setBusy(id);
      setErr(null);
      try {
        await api.sessionTrashRestore(id);
        onChange?.();
        await refresh();
      } catch (e) {
        setErr(renderError(e));
      } finally {
        setBusy(null);
      }
    },
    [onChange, refresh],
  );

  const empty = useCallback(async () => {
    setBusy("empty");
    setErr(null);
    try {
      await api.sessionTrashEmpty();
      onChange?.();
      await refresh();
    } catch (e) {
      setErr(renderError(e));
    } finally {
      setBusy(null);
      setConfirming(false);
    }
  }, [onChange, refresh]);

  return (
    <aside
      aria-label={t("trash.title")}
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-12)",
        padding: "var(--sp-24)",
      }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "center",
          gap: "var(--sp-8)",
        }}
      >
        <span
          className="mono-cap"
          style={{ color: "var(--fg-faint)" }}
        >
          {t("trash.title")}
        </span>
        {listing && listing.entries.length > 0 && (
          <Tag tone="neutral">{listing.entries.length}</Tag>
        )}
        <div style={{ flex: 1 }} />
        <Button variant="ghost" onClick={refresh} disabled={loading}>
          {t("trash.refresh")}
        </Button>
        {listing && listing.entries.length > 0 && (
          <>
            {confirming ? (
              <>
                <Button
                  variant="ghost"
                  onClick={() => setConfirming(false)}
                  disabled={busy === "empty"}
                >
                  {t("trash.cancel")}
                </Button>
                <Button
                  variant="solid"
                  onClick={empty}
                  disabled={busy === "empty"}
                  data-testid="confirm-empty"
                >
                  {busy === "empty" ? t("trash.emptying") : t("trash.emptyConfirm")}
                </Button>
              </>
            ) : (
              <Button variant="ghost" onClick={() => setConfirming(true)}>
                {t("trash.emptyTrash")}
              </Button>
            )}
          </>
        )}
      </header>

      {err && (
        <div
          role="alert"
          style={{ color: "var(--danger)", fontSize: "var(--fs-xs)" }}
        >
          {err}
        </div>
      )}

      {!loading && listing && listing.entries.length === 0 && (
        <div
          style={{
            fontSize: "var(--fs-sm)",
            color: "var(--fg-muted)",
            padding: "var(--sp-24)",
            textAlign: "center",
          }}
        >
          {t("trash.isEmpty")}
        </div>
      )}

      {listing && listing.entries.length > 0 && (
        <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
          {listing.entries.map((e) => (
            <li
              key={e.id}
              data-testid="trash-entry"
              style={{
                padding: "var(--sp-12)",
                border: "var(--bw-hair) solid var(--line)",
                borderRadius: "var(--r-2)",
                marginBottom: "var(--sp-8)",
                display: "grid",
                // `minmax(0, 1fr)` so the path column can shrink
                // below its content's intrinsic min-content width;
                // a bare `1fr` track would otherwise overflow the
                // row whenever a trash entry's path is longer than
                // the drawer is wide.
                gridTemplateColumns: "auto minmax(0, 1fr) auto",
                gap: "var(--sp-12)",
                alignItems: "center",
                fontSize: "var(--fs-sm)",
              }}
            >
              <Tag tone={e.kind === "prune" ? "warn" : "accent"}>{e.kind}</Tag>
              <span
                title={e.orig_path}
                style={{
                  whiteSpace: "nowrap",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  color: "var(--fg-muted)",
                }}
              >
                {e.orig_path}
              </span>
              <Button
                variant="ghost"
                disabled={busy === e.id}
                onClick={() => restore(e.id)}
              >
                {busy === e.id ? t("trash.restoring") : t("trash.restore")}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </aside>
  );
}

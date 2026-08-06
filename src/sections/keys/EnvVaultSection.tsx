import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { api } from "../../api";
import type { VaultSecret } from "../../api/envSecret";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { IconButton } from "../../components/primitives/IconButton";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { SkeletonRows } from "../../components/primitives/Skeleton";
import { Table, Td, Th, Tr } from "../../components/primitives/Table";
import { NF } from "../../icons";
import { formatDate } from "../../lib/intl";
import { renderError } from "../../lib/i18n-error";
import { useAppState } from "../../providers/AppStateProvider";

/**
 * The local secret vault — named env secrets stored at rest in
 * `~/.claudepot/env-vault.db` (0600). Movement only: a secret is
 * added by paste, copied out via the Rust-side clipboard path
 * (never rendered), injected into a project `.env` from
 * ProjectDetail, or deleted. No cloud, no sync.
 */
export function EnvVaultSection() {
  const { t } = useTranslation("keys");
  const { pushToast } = useAppState();
  const [secrets, setSecrets] = useState<VaultSecret[]>([]);
  const [loading, setLoading] = useState(true);
  const [pendingDelete, setPendingDelete] = useState<VaultSecret | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setSecrets(await api.envVaultList());
    } catch (e) {
      pushToast("error", renderError(e, t("vault.loadFailed")));
    } finally {
      setLoading(false);
    }
  }, [pushToast, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onCopy = useCallback(
    async (name: string) => {
      try {
        const r = await api.envVaultCopy(name);
        pushToast(
          "info",
          t("toasts.copied", { label: r.label, preview: r.preview }),
        );
      } catch (e) {
        pushToast("error", renderError(e, t("toasts.copyFailed")));
      }
    },
    [pushToast, t],
  );

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return;
    const { name } = pendingDelete;
    setPendingDelete(null);
    try {
      await api.envVaultDelete(name);
      pushToast("info", t("vault.deleted", { name }));
      await refresh();
    } catch (e) {
      pushToast("error", renderError(e, t("vault.deleteFailed")));
    }
  }, [pendingDelete, pushToast, refresh, t]);

  return (
    <section>
      <SectionLabel style={{ paddingLeft: 0, paddingRight: 0 }}>
        {t("vault.title")} {secrets.length > 0 ? `· ${secrets.length}` : ""}
      </SectionLabel>
      <p
        style={{
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          margin: "var(--sp-4) 0 var(--sp-14)",
        }}
      >
        <Trans ns="keys" i18nKey="vault.desc" components={{ code: <code /> }} />
      </p>

      <AddVaultSecretForm onAdded={refresh} />

      {loading && secrets.length === 0 ? (
        <SkeletonRows rows={2} />
      ) : secrets.length === 0 ? null : (
        <Table>
          <thead>
            <tr>
              <Th>{t("vault.colName")}</Th>
              <Th>{t("vault.colPreview")}</Th>
              <Th>{t("vault.colUpdated")}</Th>
              <Th align="right" aria-label={t("list.actionsAria")} />
            </tr>
          </thead>
          <tbody>
            {secrets.map((s) => (
              <VaultRow
                key={s.name}
                secret={s}
                onCopy={() => void onCopy(s.name)}
                onUpdated={refresh}
                onDelete={() => setPendingDelete(s)}
              />
            ))}
          </tbody>
        </Table>
      )}

      {pendingDelete && (
        <ConfirmDialog
          title={t("vault.deleteTitle", { name: pendingDelete.name })}
          body={
            <p style={{ margin: 0, lineHeight: "var(--lh-body)" }}>
              <Trans
                ns="keys"
                i18nKey="vault.deleteBody"
                values={{ name: pendingDelete.name }}
                components={{ strong: <strong />, code: <code /> }}
              />
            </p>
          }
          confirmLabel={t("vault.deleteConfirm")}
          confirmDanger
          onCancel={() => setPendingDelete(null)}
          onConfirm={() => void confirmDelete()}
        />
      )}
    </section>
  );
}

function VaultRow({
  secret,
  onCopy,
  onUpdated,
  onDelete,
}: {
  secret: VaultSecret;
  onCopy: () => void;
  onUpdated: () => Promise<void>;
  onDelete: () => void;
}) {
  const { t } = useTranslation("keys");
  const { pushToast } = useAppState();
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      await api.envVaultUpdate(secret.name, value);
      pushToast("info", t("vault.updated", { name: secret.name }));
      setEditing(false);
      await onUpdated();
    } catch (e) {
      pushToast("error", renderError(e, t("vault.updateFailed")));
    } finally {
      setValue("");
      setBusy(false);
    }
  };

  return (
    <Tr>
      <Td>
        <span className="mono" style={{ fontWeight: 600 }}>
          {secret.name}
        </span>
      </Td>
      <Td>
        <span
          className="mono"
          style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)" }}
        >
          {secret.secretPreview}
        </span>
      </Td>
      <Td>
        <span style={{ fontSize: "var(--fs-xs)", color: "var(--fg-muted)" }}>
          {formatDate(secret.updatedAtMs, {
            year: "numeric",
            month: "short",
            day: "numeric",
          })}
        </span>
      </Td>
      <Td align="right">
        {editing ? (
          <form
            className="env-inline-form"
            style={{ justifyContent: "flex-end" }}
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <input
              className="mono"
              type="password"
              placeholder={t("vault.newValuePlaceholder")}
              value={value}
              onChange={(e) => setValue(e.target.value)}
              aria-label={t("vault.newValueAria", { name: secret.name })}
              disabled={busy}
              /* eslint-disable-next-line jsx-a11y/no-autofocus */
              autoFocus
            />
            <Button variant="outline" size="sm" type="submit" disabled={busy}>
              {t("vault.save")}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setValue("");
                setEditing(false);
              }}
              disabled={busy}
            >
              {t("vault.cancel")}
            </Button>
          </form>
        ) : (
          <span
            style={{
              display: "inline-flex",
              gap: "var(--sp-4)",
              justifyContent: "flex-end",
            }}
          >
            <IconButton
              glyph={NF.copy}
              title={t("vault.copyTitle")}
              aria-label={t("vault.copyAria", { name: secret.name })}
              onClick={onCopy}
            />
            <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
              {t("vault.update")}
            </Button>
            <IconButton
              glyph={NF.trash}
              title={t("vault.deleteFromVaultTitle")}
              aria-label={t("vault.deleteAria", { name: secret.name })}
              onClick={onDelete}
            />
          </span>
        )}
      </Td>
    </Tr>
  );
}

/** Add a new named secret. The value input is a password field and is
 *  cleared on every exit path (D-5/6/7). */
function AddVaultSecretForm({ onAdded }: { onAdded: () => Promise<void> }) {
  const { t } = useTranslation("keys");
  const { pushToast } = useAppState();
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!name.trim()) {
      pushToast("error", t("vault.nameRequired"));
      return;
    }
    setBusy(true);
    try {
      await api.envVaultAdd(name.trim(), value);
      pushToast("info", t("vault.added", { name: name.trim() }));
      setName("");
      await onAdded();
    } catch (e) {
      pushToast("error", renderError(e, t("vault.addFailed")));
    } finally {
      setValue("");
      setBusy(false);
    }
  };

  return (
    <form
      className="env-inline-form"
      style={{ marginBottom: "var(--sp-14)" }}
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <input
        className="mono"
        placeholder={t("vault.namePlaceholder")}
        value={name}
        onChange={(e) => setName(e.target.value)}
        aria-label={t("vault.nameAria")}
        disabled={busy}
      />
      <input
        className="mono"
        type="password"
        placeholder={t("vault.valuePlaceholder")}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        aria-label={t("vault.valueAria")}
        disabled={busy}
      />
      <Button
        variant="solid"
        size="sm"
        glyph={NF.plus}
        type="submit"
        disabled={busy}
      >
        {t("vault.addSecret")}
      </Button>
      {busy && <Glyph g={NF.refresh} color="var(--fg-faint)" />}
    </form>
  );
}

import { useCallback, useEffect, useState } from "react";
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
import { useAppState } from "../../providers/AppStateProvider";

/**
 * The local secret vault — named env secrets stored at rest in
 * `~/.claudepot/env-vault.db` (0600). Movement only: a secret is
 * added by paste, copied out via the Rust-side clipboard path
 * (never rendered), injected into a project `.env` from
 * ProjectDetail, or deleted. No cloud, no sync.
 */
export function EnvVaultSection() {
  const { pushToast } = useAppState();
  const [secrets, setSecrets] = useState<VaultSecret[]>([]);
  const [loading, setLoading] = useState(true);
  const [pendingDelete, setPendingDelete] = useState<VaultSecret | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setSecrets(await api.envVaultList());
    } catch (e) {
      pushToast("error", `Vault load failed: ${e}`);
    } finally {
      setLoading(false);
    }
  }, [pushToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onCopy = useCallback(
    async (name: string) => {
      try {
        const r = await api.envVaultCopy(name);
        pushToast(
          "info",
          `Copied ${r.label} (${r.preview}) — clipboard clears in 30s.`,
        );
      } catch (e) {
        pushToast("error", `Copy failed: ${e}`);
      }
    },
    [pushToast],
  );

  const confirmDelete = useCallback(async () => {
    if (!pendingDelete) return;
    const { name } = pendingDelete;
    setPendingDelete(null);
    try {
      await api.envVaultDelete(name);
      pushToast("info", `Deleted ${name} from the vault.`);
      await refresh();
    } catch (e) {
      pushToast("error", `Delete failed: ${e}`);
    }
  }, [pendingDelete, pushToast, refresh]);

  return (
    <section>
      <SectionLabel style={{ paddingLeft: 0, paddingRight: 0 }}>
        Secret vault {secrets.length > 0 ? `· ${secrets.length}` : ""}
      </SectionLabel>
      <p
        style={{
          fontSize: "var(--fs-sm)",
          color: "var(--fg-muted)",
          margin: "var(--sp-4) 0 var(--sp-14)",
        }}
      >
        Local named secrets — stored at rest on this machine only.
        Copy one out, or inject it into a project's <code>.env</code> from
        that project's detail view.
      </p>

      <AddVaultSecretForm onAdded={refresh} />

      {loading && secrets.length === 0 ? (
        <SkeletonRows rows={2} />
      ) : secrets.length === 0 ? null : (
        <Table>
          <thead>
            <tr>
              <Th>Name</Th>
              <Th>Preview</Th>
              <Th>Updated</Th>
              <Th align="right" aria-label="Actions" />
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
          title={`Delete ${pendingDelete.name}?`}
          body={
            <p style={{ margin: 0, lineHeight: "var(--lh-body)" }}>
              Remove <strong>{pendingDelete.name}</strong> from the vault.
              Projects that already have this value in their{" "}
              <code>.env</code> keep it — only the vault copy is deleted.
            </p>
          }
          confirmLabel="Delete"
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
  const { pushToast } = useAppState();
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      await api.envVaultUpdate(secret.name, value);
      pushToast("info", `Updated ${secret.name}.`);
      setEditing(false);
      await onUpdated();
    } catch (e) {
      pushToast("error", `Update failed: ${e}`);
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
          {new Date(secret.updatedAtMs).toLocaleDateString(undefined, {
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
              placeholder="new value"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              aria-label={`New value for ${secret.name}`}
              disabled={busy}
              /* eslint-disable-next-line jsx-a11y/no-autofocus */
              autoFocus
            />
            <Button variant="outline" size="sm" type="submit" disabled={busy}>
              Save
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
              Cancel
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
              title="Copy value to clipboard"
              aria-label={`Copy ${secret.name}`}
              onClick={onCopy}
            />
            <Button variant="ghost" size="sm" onClick={() => setEditing(true)}>
              Update
            </Button>
            <IconButton
              glyph={NF.trash}
              title="Delete from vault"
              aria-label={`Delete ${secret.name}`}
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
  const { pushToast } = useAppState();
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!name.trim()) {
      pushToast("error", "Secret name is required.");
      return;
    }
    setBusy(true);
    try {
      await api.envVaultAdd(name.trim(), value);
      pushToast("info", `Added ${name.trim()} to the vault.`);
      setName("");
      await onAdded();
    } catch (e) {
      pushToast("error", `Add failed: ${e}`);
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
        placeholder="SECRET_NAME"
        value={name}
        onChange={(e) => setName(e.target.value)}
        aria-label="New secret name"
        disabled={busy}
      />
      <input
        className="mono"
        type="password"
        placeholder="value"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        aria-label="New secret value"
        disabled={busy}
      />
      <Button
        variant="solid"
        size="sm"
        glyph={NF.plus}
        type="submit"
        disabled={busy}
      >
        Add secret
      </Button>
      {busy && <Glyph g={NF.refresh} color="var(--fg-faint)" />}
    </form>
  );
}

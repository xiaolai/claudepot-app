import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import type {
  EnvFileEntry,
  EnvFileView,
  ProjectEnv,
} from "../../api/envSecret";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { CopyButton } from "../../components/CopyButton";
import { Button } from "../../components/primitives/Button";
import { IconButton } from "../../components/primitives/IconButton";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import { useAppState } from "../../providers/AppStateProvider";

/**
 * Per-project `.env*` view — the *movement* layer, not an editor.
 * Each key row carries: copy-out, comment⇄uncomment (the value stays
 * on disk, just inactive), delete (with confirm), and a per-file
 * "set key" + "inject from vault" form. Editing arbitrary file text
 * is deliberately out of scope — that's what your editor is for.
 *
 * Values are never rendered: rows show a non-reversible preview, and
 * the real value reaches the clipboard only via the Rust-side copy
 * path.
 */
export function ProjectEnvPanel({
  projectPath,
  onError,
}: {
  projectPath: string;
  onError?: (msg: string) => void;
}) {
  const { pushToast } = useAppState();
  const [env, setEnv] = useState<ProjectEnv | null>(null);
  const [vaultNames, setVaultNames] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [confirmDelete, setConfirmDelete] = useState<{
    fileName: string;
    key: string;
  } | null>(null);

  const fail = useCallback(
    (msg: string) => {
      if (onError) onError(msg);
      else pushToast("error", msg);
    },
    [onError, pushToast],
  );

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    Promise.all([api.envFileList(projectPath), api.envVaultList()])
      .then(([e, vault]) => {
        if (cancelled) return;
        setEnv(e);
        setVaultNames(vault.map((v) => v.name));
        setLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        fail(`Couldn't load .env files: ${err}`);
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [projectPath, fail]);

  const onCopy = useCallback(
    async (fileName: string, key: string) => {
      try {
        const r = await api.envFileCopyValue(projectPath, fileName, key);
        pushToast(
          "info",
          `Copied ${r.label} (${r.preview}) — clipboard clears in 30s.`,
        );
      } catch (e) {
        fail(`Copy failed: ${e}`);
      }
    },
    [projectPath, pushToast, fail],
  );

  const toggleComment = useCallback(
    async (entry: EnvFileEntry, fileName: string) => {
      try {
        const next =
          entry.state === "active"
            ? await api.envFileComment(projectPath, fileName, entry.key)
            : await api.envFileUncomment(projectPath, fileName, entry.key);
        setEnv(next);
      } catch (e) {
        fail(`Couldn't update ${entry.key}: ${e}`);
      }
    },
    [projectPath, fail],
  );

  const doDelete = useCallback(async () => {
    if (!confirmDelete) return;
    const { fileName, key } = confirmDelete;
    setConfirmDelete(null);
    try {
      const next = await api.envFileDelete(projectPath, fileName, key);
      setEnv(next);
      pushToast("info", `Deleted ${key} from ${fileName}.`);
    } catch (e) {
      fail(`Delete failed: ${e}`);
    }
  }, [confirmDelete, projectPath, pushToast, fail]);

  if (loading) {
    return (
      <section className="detail-section">
        <h3>Environment files</h3>
        <p className="muted small">Loading…</p>
      </section>
    );
  }

  const files = env?.files ?? [];

  return (
    <section className="detail-section">
      <h3>Environment files</h3>
      {files.length === 0 ? (
        <>
          <p className="muted small">
            No <code className="mono">.env*</code> files in this project. Add a
            key below to create one.
          </p>
          <EnvFileCard
            projectPath={projectPath}
            file={{ fileName: ".env", path: "", entries: [] }}
            vaultNames={vaultNames}
            onCopy={onCopy}
            onToggleComment={toggleComment}
            onRequestDelete={(key) =>
              setConfirmDelete({ fileName: ".env", key })
            }
            onMutated={setEnv}
            onError={fail}
          />
        </>
      ) : (
        files.map((file) => (
          <EnvFileCard
            key={file.fileName}
            projectPath={projectPath}
            file={file}
            vaultNames={vaultNames}
            onCopy={onCopy}
            onToggleComment={toggleComment}
            onRequestDelete={(key) =>
              setConfirmDelete({ fileName: file.fileName, key })
            }
            onMutated={setEnv}
            onError={fail}
          />
        ))
      )}

      {confirmDelete && (
        <ConfirmDialog
          title={`Delete ${confirmDelete.key}?`}
          body={
            <span>
              This removes <code className="mono">{confirmDelete.key}</code>{" "}
              from <code className="mono">{confirmDelete.fileName}</code>{" "}
              entirely. To keep the value but disable it, use{" "}
              <strong>Comment out</strong> instead.
            </span>
          }
          confirmLabel="Delete"
          confirmDanger
          onCancel={() => setConfirmDelete(null)}
          onConfirm={doDelete}
        />
      )}
    </section>
  );
}

function EnvFileCard({
  projectPath,
  file,
  vaultNames,
  onCopy,
  onToggleComment,
  onRequestDelete,
  onMutated,
  onError,
}: {
  projectPath: string;
  file: EnvFileView;
  vaultNames: string[];
  onCopy: (fileName: string, key: string) => void;
  onToggleComment: (entry: EnvFileEntry, fileName: string) => void;
  onRequestDelete: (key: string) => void;
  onMutated: (env: ProjectEnv) => void;
  onError: (msg: string) => void;
}) {
  return (
    <div className="env-file-card">
      <div className="env-file-card-head">
        <span className="mono selectable" title={file.path || file.fileName}>
          {file.fileName}
        </span>
        {file.path && <CopyButton text={file.path} />}
      </div>

      {file.entries.length === 0 ? (
        <p className="muted small">No keys yet.</p>
      ) : (
        <ul className="env-entry-list" role="list">
          {/* Key by index too: a malformed .env can repeat a key,
              and a bare `entry.key` would collide in React's keyspace. */}
          {file.entries.map((entry, idx) => (
            <li key={`${entry.key}-${idx}`} className="env-entry-row">
              <span className="mono env-entry-key">{entry.key}</span>
              {entry.state === "active" ? (
                <Tag tone="neutral">active</Tag>
              ) : (
                <Tag tone="ghost">commented</Tag>
              )}
              <span className="mono muted env-entry-preview">
                {entry.valuePreview}
              </span>
              <span className="env-entry-actions">
                <IconButton
                  glyph={NF.copy}
                  size="sm"
                  title="Copy value to clipboard"
                  aria-label={`Copy ${entry.key}`}
                  onClick={() => onCopy(file.fileName, entry.key)}
                />
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onToggleComment(entry, file.fileName)}
                >
                  {entry.state === "active" ? "Comment out" : "Uncomment"}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  danger
                  onClick={() => onRequestDelete(entry.key)}
                >
                  Delete
                </Button>
              </span>
            </li>
          ))}
        </ul>
      )}

      <div className="env-file-card-forms">
        <SetKeyForm
          projectPath={projectPath}
          fileName={file.fileName}
          onMutated={onMutated}
          onError={onError}
        />
        {vaultNames.length > 0 && (
          <InjectForm
            projectPath={projectPath}
            fileName={file.fileName}
            vaultNames={vaultNames}
            onMutated={onMutated}
            onError={onError}
          />
        )}
      </div>
    </div>
  );
}

/** Upsert a `key=value` into the file. `set` is an upsert, so this
 *  also re-sets an existing key's value. The value input is a
 *  password field and is cleared on every exit path. */
function SetKeyForm({
  projectPath,
  fileName,
  onMutated,
  onError,
}: {
  projectPath: string;
  fileName: string;
  onMutated: (env: ProjectEnv) => void;
  onError: (msg: string) => void;
}) {
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!key.trim()) {
      onError("Key name is required.");
      return;
    }
    setBusy(true);
    try {
      const next = await api.envFileSet(
        projectPath,
        fileName,
        key.trim(),
        value,
      );
      onMutated(next);
      setKey("");
    } catch (e) {
      onError(`Set failed: ${e}`);
    } finally {
      // Clear the secret from React state regardless of outcome.
      setValue("");
      setBusy(false);
    }
  };

  return (
    <form
      className="env-inline-form"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <input
        className="mono"
        placeholder="KEY"
        value={key}
        onChange={(e) => setKey(e.target.value)}
        aria-label={`Key name for ${fileName}`}
        disabled={busy}
      />
      <input
        className="mono"
        type="password"
        placeholder="value"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        aria-label={`Value for ${fileName}`}
        disabled={busy}
      />
      <Button variant="outline" size="sm" type="submit" disabled={busy}>
        Set key
      </Button>
    </form>
  );
}

/** Inject a named vault secret into the file as `name=<secret>`.
 *  The plaintext never enters the renderer — Rust reveals it and
 *  writes it. */
function InjectForm({
  projectPath,
  fileName,
  vaultNames,
  onMutated,
  onError,
}: {
  projectPath: string;
  fileName: string;
  vaultNames: string[];
  onMutated: (env: ProjectEnv) => void;
  onError: (msg: string) => void;
}) {
  const [vaultName, setVaultName] = useState(vaultNames[0] ?? "");
  const [busy, setBusy] = useState(false);

  const inject = async () => {
    if (!vaultName) return;
    setBusy(true);
    try {
      const next = await api.envFileInject(projectPath, fileName, vaultName);
      onMutated(next);
    } catch (e) {
      onError(`Inject failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="env-inline-form">
      <select
        className="mono"
        value={vaultName}
        onChange={(e) => setVaultName(e.target.value)}
        aria-label={`Vault secret to inject into ${fileName}`}
        disabled={busy}
      >
        {vaultNames.map((n) => (
          <option key={n} value={n}>
            {n}
          </option>
        ))}
      </select>
      <Button
        variant="outline"
        size="sm"
        glyph={NF.package}
        onClick={() => void inject()}
        disabled={busy}
      >
        Inject from vault
      </Button>
    </div>
  );
}

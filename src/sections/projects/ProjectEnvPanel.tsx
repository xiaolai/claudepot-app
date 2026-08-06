import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
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
import { i18n } from "../../lib/i18n";
import { renderError } from "../../lib/i18n-error";
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
  const { t } = useTranslation("projects");
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
    // Stale-while-revalidate: do NOT set loading=true on refetches.
    // If we already have data (env !== null) keep showing it; the
    // refresh swaps the content in atomically when it resolves. The
    // initial mount still flashes the "Loading…" placeholder because
    // `loading` starts true in useState. Without this guard, every
    // parent re-render briefly collapsed the panel to "Loading…",
    // displacing the Sessions section below by ~108 px.
    Promise.all([api.envFileList(projectPath), api.envVaultList()])
      .then(([e, vault]) => {
        if (cancelled) return;
        setEnv(e);
        setVaultNames(vault.map((v) => v.name));
        setLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        fail(renderError(err, i18n.t("projects:env.loadFailedScope")));
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
          t("env.copyToast", { label: r.label, preview: r.preview }),
        );
      } catch (e) {
        fail(renderError(e, t("env.copyFailedScope")));
      }
    },
    [projectPath, pushToast, fail, t],
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
        fail(renderError(e, t("env.updateFailedScope", { key: entry.key })));
      }
    },
    [projectPath, fail, t],
  );

  const doDelete = useCallback(async () => {
    if (!confirmDelete) return;
    const { fileName, key } = confirmDelete;
    setConfirmDelete(null);
    try {
      const next = await api.envFileDelete(projectPath, fileName, key);
      setEnv(next);
      pushToast("info", t("env.deletedToast", { key, file: fileName }));
    } catch (e) {
      fail(renderError(e, t("env.deleteFailedScope")));
    }
  }, [confirmDelete, projectPath, pushToast, fail, t]);

  // Show the loader only on the initial mount when we have no data
  // yet. Refetches keep the prior content visible (stale-while-
  // revalidate) so the section's height stays stable.
  if (loading && env === null) {
    return (
      <section className="detail-section">
        <h3>{t("env.heading")}</h3>
        <p className="muted small">{t("shared.loading")}</p>
      </section>
    );
  }

  const files = env?.files ?? [];

  return (
    <section className="detail-section">
      <h3>{t("env.heading")}</h3>
      {files.length === 0 ? (
        <>
          <p className="muted small">
            <Trans
              ns="projects"
              i18nKey="env.emptyHint"
              components={{ envfiles: <code className="mono">.env*</code> }}
            />
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
          title={t("env.deleteTitle", { key: confirmDelete.key })}
          body={
            <span>
              <Trans
                ns="projects"
                i18nKey="env.deleteBody"
                components={{
                  k: <code className="mono">{confirmDelete.key}</code>,
                  f: <code className="mono">{confirmDelete.fileName}</code>,
                  b: <strong />,
                }}
              />
            </span>
          }
          confirmLabel={t("env.confirmDelete")}
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
  const { t } = useTranslation("projects");
  return (
    <div className="env-file-card">
      <div className="env-file-card-head">
        <span className="mono selectable" title={file.path || file.fileName}>
          {file.fileName}
        </span>
        {file.path && <CopyButton text={file.path} />}
      </div>

      {file.entries.length === 0 ? (
        <p className="muted small">{t("env.noKeys")}</p>
      ) : (
        <ul className="env-entry-list" role="list">
          {/* Key by index too: a malformed .env can repeat a key,
              and a bare `entry.key` would collide in React's keyspace. */}
          {file.entries.map((entry, idx) => (
            <li key={`${entry.key}-${idx}`} className="env-entry-row">
              <span className="mono env-entry-key">{entry.key}</span>
              {entry.state === "active" ? (
                <Tag tone="neutral">{t("env.tagActive")}</Tag>
              ) : (
                <Tag tone="ghost">{t("env.tagCommented")}</Tag>
              )}
              <span className="mono muted env-entry-preview">
                {entry.valuePreview}
              </span>
              <span className="env-entry-actions">
                <IconButton
                  glyph={NF.copy}
                  size="sm"
                  title={t("env.copyTitle")}
                  aria-label={t("env.copyAria", { key: entry.key })}
                  onClick={() => onCopy(file.fileName, entry.key)}
                />
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => onToggleComment(entry, file.fileName)}
                >
                  {entry.state === "active"
                    ? t("env.commentOut")
                    : t("env.uncomment")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  danger
                  onClick={() => onRequestDelete(entry.key)}
                >
                  {t("env.delete")}
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
  const { t } = useTranslation("projects");
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    if (!key.trim()) {
      onError(t("env.keyRequired"));
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
      onError(renderError(e, t("env.setFailedScope")));
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
        placeholder={t("env.keyPlaceholder")}
        value={key}
        onChange={(e) => setKey(e.target.value)}
        aria-label={t("env.keyAria", { file: fileName })}
        disabled={busy}
      />
      <input
        className="mono"
        type="password"
        placeholder={t("env.valuePlaceholder")}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        aria-label={t("env.valueAria", { file: fileName })}
        disabled={busy}
      />
      <Button variant="outline" size="sm" type="submit" disabled={busy}>
        {t("env.setKey")}
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
  const { t } = useTranslation("projects");
  const [vaultName, setVaultName] = useState(vaultNames[0] ?? "");
  const [busy, setBusy] = useState(false);

  const inject = async () => {
    if (!vaultName) return;
    setBusy(true);
    try {
      const next = await api.envFileInject(projectPath, fileName, vaultName);
      onMutated(next);
    } catch (e) {
      onError(renderError(e, t("env.injectFailedScope")));
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
        aria-label={t("env.vaultAria", { file: fileName })}
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
        {t("env.injectFromVault")}
      </Button>
    </div>
  );
}

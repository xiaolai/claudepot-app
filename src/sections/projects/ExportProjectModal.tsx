import { useId, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { migrateApi, type ExportReceipt } from "../../api/migrate";
import { renderError } from "../../lib/i18n-error";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";

/**
 * ExportProjectModal — single-modal export wizard.
 *
 * Per spec §12.2 the design is a 5-step wizard (picker → options →
 * output → summary → progress). v0 collapses that into one modal: the
 * project is pre-selected from the row kebab, options live inline, the
 * output path defaults to the user's home, and progress is replaced by
 * a synchronous error/success toast (the receipt drives the
 * after-modal banner).
 *
 * Multi-project picker, trust-gate per-item review, and conflict
 * preview ride along in the next slice; this surface is enough to
 * unblock the primary "export this one project" flow.
 */
export function ExportProjectModal({
  cwd,
  onClose,
  onCompleted,
  onError,
}: {
  cwd: string;
  onClose: () => void;
  onCompleted: (receipt: ExportReceipt) => void;
  onError: (msg: string) => void;
}) {
  const { t } = useTranslation("projects");
  const headingId = useId();
  const outId = useId();
  const passId = useId();

  const defaultOutput = `${cwd.replace(/[^a-zA-Z0-9]+/g, "-")}.claudepot.tar.zst`;
  const [output, setOutput] = useState<string>(defaultOutput);
  const [includeGlobal, setIncludeGlobal] = useState(false);
  const [includeWorktree, setIncludeWorktree] = useState(false);
  const [includeClaudepotState, setIncludeClaudepotState] = useState(false);
  const [encrypt, setEncrypt] = useState(true);
  const [passphrase, setPassphrase] = useState("");
  const [submitting, setSubmitting] = useState(false);

  async function handleExport() {
    if (encrypt && !passphrase) {
      onError(t("export.passphraseRequired"));
      return;
    }
    setSubmitting(true);
    try {
      const receipt = await migrateApi.export({
        outputPath: output,
        projectPrefixes: [cwd],
        includeGlobal,
        includeWorktree,
        includeClaudepotState,
        encrypt,
        encryptPassphrase: encrypt ? passphrase : undefined,
      });
      onCompleted(receipt);
    } catch (e) {
      onError(renderError(e));
    } finally {
      setSubmitting(false);
      // Best-effort scrub: clear the local React state so the
      // passphrase doesn't outlive the modal in the JS heap.
      setPassphrase("");
    }
  }

  return (
    <Modal open onClose={onClose} aria-labelledby={headingId}>
      <ModalHeader title={t("export.title")} id={headingId} onClose={onClose} />
      <ModalBody>
        <p style={{ marginTop: 0 }}>
          <Trans
            ns="projects"
            i18nKey="export.intro"
            components={{ cwd: <code>{cwd}</code> }}
          />
        </p>

        <label htmlFor={outId} style={{ display: "block", marginTop: "var(--sp-12)" }}>
          {t("export.outputLabel")}
        </label>
        <Input
          id={outId}
          type="text"
          value={output}
          onChange={(e) => setOutput(e.target.value)}
          placeholder="my-project.claudepot.tar.zst"
          style={{ width: "100%" }}
        />

        <fieldset style={{ marginTop: "var(--sp-16)", border: 0, padding: 0 }}>
          <legend style={{ marginBottom: "var(--sp-6)" }}>{t("export.includeLegend")}</legend>
          <label style={{ display: "block", marginBottom: "var(--sp-6)" }}>
            <input
              type="checkbox"
              checked={includeGlobal}
              onChange={(e) => setIncludeGlobal(e.target.checked)}
            />{" "}
            {t("export.optGlobal")}
          </label>
          <label style={{ display: "block", marginBottom: "var(--sp-6)" }}>
            <input
              type="checkbox"
              checked={includeWorktree}
              onChange={(e) => setIncludeWorktree(e.target.checked)}
            />{" "}
            <Trans
              ns="projects"
              i18nKey="export.optWorktree"
              components={{ dir: <code>.claude/</code> }}
            />
          </label>
          <label style={{ display: "block", marginBottom: "var(--sp-6)" }}>
            <input
              type="checkbox"
              checked={includeClaudepotState}
              onChange={(e) => setIncludeClaudepotState(e.target.checked)}
            />{" "}
            {t("export.optState")}
          </label>
          <label style={{ display: "block", marginBottom: "var(--sp-6)" }}>
            <input
              type="checkbox"
              checked={encrypt}
              onChange={(e) => setEncrypt(e.target.checked)}
            />{" "}
            {t("export.optEncrypt")}
          </label>
        </fieldset>

        {encrypt && (
          <>
            <label
              htmlFor={passId}
              style={{ display: "block", marginTop: "var(--sp-12)" }}
            >
              {t("export.passphraseLabel")}
            </label>
            <Input
              id={passId}
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              autoComplete="new-password"
              style={{ width: "100%" }}
            />
          </>
        )}
      </ModalBody>
      <ModalFooter>
        <Button onClick={onClose} disabled={submitting}>
          {t("shared.cancel")}
        </Button>
        <Button
          variant="solid"
          onClick={handleExport}
          disabled={submitting || !output}
        >
          {submitting ? t("export.exporting") : t("export.submit")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

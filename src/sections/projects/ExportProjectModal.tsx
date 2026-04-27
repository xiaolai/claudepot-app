import { useId, useState } from "react";
import { migrateApi, type ExportReceipt } from "../../api/migrate";
import { Button } from "../../components/primitives/Button";
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
      onError("Encryption requires a passphrase (or untick Encrypt).");
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
      onError(String(e));
    } finally {
      setSubmitting(false);
      // Best-effort scrub: clear the local React state so the
      // passphrase doesn't outlive the modal in the JS heap.
      setPassphrase("");
    }
  }

  return (
    <Modal open onClose={onClose} aria-labelledby={headingId}>
      <ModalHeader title="Export project" id={headingId} onClose={onClose} />
      <ModalBody>
        <p style={{ marginTop: 0 }}>
          Bundle the CC state for <code>{cwd}</code> into a portable
          file. Credentials never travel.
        </p>

        <label htmlFor={outId} style={{ display: "block", marginTop: 12 }}>
          Output file
        </label>
        <input
          id={outId}
          type="text"
          value={output}
          onChange={(e) => setOutput(e.target.value)}
          placeholder="my-project.claudepot.tar.zst"
          style={{ width: "100%", padding: "var(--sp-6) var(--sp-8)" }}
        />

        <fieldset style={{ marginTop: 16, border: 0, padding: 0 }}>
          <legend style={{ marginBottom: 6 }}>Include</legend>
          <label style={{ display: "block", marginBottom: 6 }}>
            <input
              type="checkbox"
              checked={includeGlobal}
              onChange={(e) => setIncludeGlobal(e.target.checked)}
            />{" "}
            Global content (CLAUDE.md, agents/, skills/, scrubbed
            settings, plugin registry)
          </label>
          <label style={{ display: "block", marginBottom: 6 }}>
            <input
              type="checkbox"
              checked={includeWorktree}
              onChange={(e) => setIncludeWorktree(e.target.checked)}
            />{" "}
            Worktree (project's <code>.claude/</code> dir + CLAUDE.md;
            local settings excluded)
          </label>
          <label style={{ display: "block", marginBottom: 6 }}>
            <input
              type="checkbox"
              checked={includeClaudepotState}
              onChange={(e) => setIncludeClaudepotState(e.target.checked)}
            />{" "}
            Claudepot state (account stubs only — no credentials)
          </label>
          <label style={{ display: "block", marginBottom: 6 }}>
            <input
              type="checkbox"
              checked={encrypt}
              onChange={(e) => setEncrypt(e.target.checked)}
            />{" "}
            Encrypt with passphrase (age)
          </label>
        </fieldset>

        {encrypt && (
          <>
            <label
              htmlFor={passId}
              style={{ display: "block", marginTop: 12 }}
            >
              Passphrase
            </label>
            <input
              id={passId}
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              autoComplete="new-password"
              style={{ width: "100%", padding: "var(--sp-6) var(--sp-8)" }}
            />
          </>
        )}
      </ModalBody>
      <ModalFooter>
        <Button onClick={onClose} disabled={submitting}>
          Cancel
        </Button>
        <Button
          variant="solid"
          onClick={handleExport}
          disabled={submitting || !output}
        >
          {submitting ? "Exporting…" : "Export"}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

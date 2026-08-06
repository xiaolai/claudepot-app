import { useId, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import {
  migrateApi,
  type ImportPlan,
  type ImportReceipt,
} from "../../api/migrate";
import { renderError } from "../../lib/i18n-error";
import { Button } from "../../components/primitives/Button";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";

/**
 * ImportBundleModal — single-modal import wizard.
 *
 * Per spec §12.2 the design is a 5-step wizard (inspect →
 * conflict-mode → trust-gate → substitution preview → progress).
 * v0 collapses that into one modal: the user types the bundle path
 * (or pastes one dropped from Finder), optionally enters a
 * passphrase, sees the manifest summary inline, picks a conflict
 * mode + acceptance flags, and imports.
 *
 * Trust-gate per-item review and substitution-rule editor land in
 * the next slice; for now `--accept-hooks` is a single checkbox that
 * accepts all bundled hooks.
 */
export function ImportBundleModal({
  onClose,
  onCompleted,
  onError,
}: {
  onClose: () => void;
  onCompleted: (receipt: ImportReceipt) => void;
  onError: (msg: string) => void;
}) {
  const { t } = useTranslation("projects");
  const headingId = useId();
  const bundleId = useId();
  const passId = useId();

  const [bundlePath, setBundlePath] = useState<string>("");
  const [passphrase, setPassphrase] = useState<string>("");
  const [plan, setPlan] = useState<ImportPlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [inspecting, setInspecting] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const [mode, setMode] = useState<"skip" | "merge" | "replace">("skip");
  const [acceptHooks, setAcceptHooks] = useState(false);
  const [acceptMcp, setAcceptMcp] = useState(false);
  const [dryRun, setDryRun] = useState(true);

  const isEncrypted = bundlePath.endsWith(".age");

  const modeLabel = (m: "skip" | "merge" | "replace"): string =>
    m === "skip"
      ? t("import.modeSkip")
      : m === "merge"
        ? t("import.modeMerge")
        : t("import.modeReplace");

  async function handleInspect() {
    if (!bundlePath) return;
    setInspecting(true);
    setPlanError(null);
    setPlan(null);
    try {
      const p = await migrateApi.inspect(
        bundlePath,
        isEncrypted ? passphrase : undefined,
      );
      setPlan(p);
    } catch (e) {
      setPlanError(renderError(e));
    } finally {
      setInspecting(false);
    }
  }

  async function handleImport() {
    setSubmitting(true);
    try {
      const receipt = await migrateApi.import({
        bundlePath,
        mode,
        acceptHooks,
        acceptMcp,
        dryRun,
        passphrase: isEncrypted ? passphrase : undefined,
      });
      onCompleted(receipt);
    } catch (e) {
      onError(renderError(e));
    } finally {
      setSubmitting(false);
      setPassphrase("");
    }
  }

  return (
    <Modal open onClose={onClose} aria-labelledby={headingId}>
      <ModalHeader title={t("import.title")} id={headingId} onClose={onClose} />
      <ModalBody>
        <label htmlFor={bundleId}>{t("import.bundleLabel")}</label>
        <input
          id={bundleId}
          type="text"
          value={bundlePath}
          onChange={(e) => setBundlePath(e.target.value)}
          placeholder="/path/to/file.claudepot.tar.zst[.age]"
          style={{ width: "100%", padding: "var(--sp-6) var(--sp-8)" }}
        />

        {isEncrypted && (
          <>
            <label htmlFor={passId} style={{ display: "block", marginTop: "var(--sp-8)" }}>
              {t("import.passphraseLabel")}
            </label>
            <input
              id={passId}
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              autoComplete="off"
              style={{ width: "100%", padding: "var(--sp-6) var(--sp-8)" }}
            />
          </>
        )}

        <div style={{ marginTop: "var(--sp-12)" }}>
          <Button onClick={handleInspect} disabled={inspecting || !bundlePath}>
            {inspecting ? t("import.inspecting") : t("import.inspect")}
          </Button>
        </div>

        {planError && (
          <p style={{ color: "var(--danger)", marginTop: "var(--sp-12)" }}>
            {planError}
          </p>
        )}

        {plan && (
          <section
            style={{
              marginTop: "var(--sp-16)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
              padding: "var(--sp-12)",
            }}
          >
            <h3 style={{ margin: 0, marginBottom: "var(--sp-8)" }}>{t("import.manifestTitle")}</h3>
            <p style={{ margin: 0, marginBottom: "var(--sp-4)" }}>
              {t("import.manifestMeta", {
                schema: plan.schemaVersion,
                version: plan.claudepotVersion,
                os: plan.sourceOs,
                arch: plan.sourceArch,
              })}
            </p>
            <p style={{ margin: 0, marginBottom: "var(--sp-4)" }}>
              {t("import.createdAt", { date: plan.createdAt })}
            </p>
            <p style={{ margin: 0, marginBottom: "var(--sp-8)" }}>
              {t("import.flagsLine", {
                global: String(plan.flags.includeGlobal),
                worktree: String(plan.flags.includeWorktree),
                live: String(plan.flags.includeLive),
                state: String(plan.flags.includeClaudepotState),
                enc: String(plan.flags.encrypted),
                sig: String(plan.flags.signed),
              })}
            </p>
            <p style={{ margin: 0, marginBottom: "var(--sp-4)" }}>
              {t("import.projectsCount", { n: plan.projects.length })}
            </p>
            <ul style={{ marginTop: "var(--sp-4)" }}>
              {plan.projects.map((p) => (
                <li key={p.id}>
                  <code>{p.sourceCwd}</code>{" "}
                  {t("import.sessionsSuffix", { n: p.sessionCount })}
                </li>
              ))}
            </ul>
          </section>
        )}

        <fieldset style={{ marginTop: "var(--sp-16)", border: 0, padding: 0 }}>
          <legend style={{ marginBottom: "var(--sp-6)" }}>{t("import.conflictLegend")}</legend>
          {(["skip", "merge", "replace"] as const).map((m) => (
            <label
              key={m}
              style={{ display: "inline-block", marginRight: "var(--sp-12)" }}
            >
              <input
                type="radio"
                name="mode"
                checked={mode === m}
                onChange={() => setMode(m)}
              />{" "}
              {modeLabel(m)}
            </label>
          ))}
        </fieldset>

        <fieldset style={{ marginTop: "var(--sp-12)", border: 0, padding: 0 }}>
          <legend style={{ marginBottom: "var(--sp-6)" }}>{t("import.trustLegend")}</legend>
          <label style={{ display: "block" }}>
            <input
              type="checkbox"
              checked={acceptHooks}
              onChange={(e) => setAcceptHooks(e.target.checked)}
            />{" "}
            <Trans
              ns="projects"
              i18nKey="import.acceptHooks"
              components={{ f: <code>proposed-hooks.json</code> }}
            />
          </label>
          <label style={{ display: "block" }}>
            <input
              type="checkbox"
              checked={acceptMcp}
              onChange={(e) => setAcceptMcp(e.target.checked)}
            />{" "}
            {t("import.acceptMcp")}
          </label>
        </fieldset>

        <label style={{ display: "block", marginTop: "var(--sp-12)" }}>
          <input
            type="checkbox"
            checked={dryRun}
            onChange={(e) => setDryRun(e.target.checked)}
          />{" "}
          {t("import.dryRun")}
        </label>
      </ModalBody>
      <ModalFooter>
        <Button onClick={onClose} disabled={submitting}>
          {t("shared.cancel")}
        </Button>
        <Button
          variant="solid"
          onClick={handleImport}
          disabled={submitting || !bundlePath || !plan}
        >
          {submitting
            ? t("import.importing")
            : dryRun
              ? t("import.planImport")
              : t("import.submit")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

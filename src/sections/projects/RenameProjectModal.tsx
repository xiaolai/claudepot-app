import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../api";
import { extractMessage, renderError } from "../../lib/i18n-error";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { IconButton } from "../../components/primitives/IconButton";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "../../components/primitives/Modal";
import {
  FieldBlock,
  GroupCard,
  Hint,
  OptionRow,
} from "../../components/primitives/modalParts";
import { NF } from "../../icons";
import { basename } from "../../lib/paths";
import { DRY_RUN_SUPERSEDED, type DryRunPlan, type MoveArgs } from "../../types";

const DEBOUNCE_MS = 300;

type PreviewState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; plan: DryRunPlan }
  | { kind: "error"; message: string };

type CollisionPolicy = "none" | "merge" | "overwrite";

/**
 * Rename modal. Per plan §7.1:
 * - Path text input is the primary authority. "Browse parent…" helps
 *   but writes into the same field so case-only renames and arbitrary
 *   basenames still work.
 * - Dry-run preview is debounced (300ms) and re-requested on every
 *   change of inputs that affect the plan (new path, collision policy,
 *   flags).
 * - Danger zone visually separates `--force` and
 *   `--ignore-pending-journals` from the collision radio. Each has
 *   explicit consequence copy.
 * - Submit is explicitly not a safety claim — copy below the button
 *   says so verbatim.
 */
export function RenameProjectModal({
  oldPath,
  onClose,
  onSubmit,
}: {
  oldPath: string;
  onClose: () => void;
  /** Called when the user confirms. Parent performs the execution. */
  onSubmit: (args: MoveArgs) => void;
}) {
  const { t } = useTranslation("projects");
  const [newPath, setNewPath] = useState<string>(oldPath);
  const [collision, setCollision] = useState<CollisionPolicy>("none");
  const [force, setForce] = useState(false);
  const [ignorePending, setIgnorePending] = useState(false);
  const [noMove, setNoMove] = useState(false);
  const [preview, setPreview] = useState<PreviewState>({ kind: "idle" });

  const headingId = useId();

  // Used to drop stale preview responses: every keystroke increments
  // the token; on response we check ours still matches. Cheaper than
  // aborting Tauri invokes — which Tauri doesn't support anyway — and
  // it also cheaply drops responses that raced a later keystroke.
  const reqToken = useRef(0);

  const args: MoveArgs = useMemo(
    () => ({
      oldPath,
      newPath,
      noMove,
      merge: collision === "merge",
      overwrite: collision === "overwrite",
      force,
      ignorePendingJournals: ignorePending,
    }),
    [oldPath, newPath, noMove, collision, force, ignorePending],
  );

  const runPreview = useCallback(() => {
    if (!newPath.trim()) {
      // Audit M17: advance the token even on the empty-input branch.
      // Previously the token only incremented inside the non-empty
      // path, so if the user cleared the input while a request was
      // in flight, that in-flight response could still arrive and
      // repopulate the preview for an empty input (stale-data leak).
      ++reqToken.current;
      setPreview({ kind: "idle" });
      return;
    }
    const myToken = ++reqToken.current;
    setPreview({ kind: "loading" });
    // Send the token to the backend so it can short-circuit stale work
    // on its side too (plan §7.1). Monotonic + shared process-wide is
    // fine — the backend's DryRunRegistry uses fetch_max.
    api
      .projectMoveDryRun({ ...args, cancelToken: myToken })
      .then((plan) => {
        if (myToken !== reqToken.current) return; // stale
        setPreview({ kind: "ok", plan });
      })
      .catch((e) => {
        if (myToken !== reqToken.current) return;
        // Raw (untruncated, unredacted) text for the sentinel test —
        // `renderError` caps at 240 chars, which must never decide
        // control flow. Display goes through `renderError` separately.
        const msg = extractMessage(e);
        // Backend sentinel: it noticed we were superseded and bailed.
        // Leave the preview state as-is so the UI doesn't flash an
        // error — a newer call is already in flight.
        if (msg.includes(DRY_RUN_SUPERSEDED)) return;
        setPreview({ kind: "error", message: renderError(e) });
      });
  }, [args, newPath]);

  useEffect(() => {
    const handle = window.setTimeout(runPreview, DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [runPreview]);

  // Audit M17: invalidate the last token when the modal closes so
  // any in-flight dry-run can't call setPreview after unmount.
  useEffect(() => {
    return () => {
      // Bumping the token past any in-flight call's value guarantees
      // the stale-response guard fails for responses that land post-unmount.
      reqToken.current += 1;
    };
  }, []);

  const browseParent = async () => {
    try {
      const result = await openDialog({
        directory: true,
        multiple: false,
        title: t("rename.chooseParentTitle"),
      });
      if (typeof result === "string" && result) {
        const basename = currentBasename(newPath) || currentBasename(oldPath);
        setNewPath(basename ? `${result.replace(/\/$/, "")}/${basename}` : result);
      }
    } catch (e) {
      console.warn("browse dialog failed", e);
    }
  };

  const conflict = preview.kind === "ok" ? preview.plan.conflict : null;
  const conflictNeedsPolicy = Boolean(conflict) && collision === "none";
  const disabledReason: string | null = (() => {
    if (!newPath.trim()) return t("rename.disabledEnterPath");
    if (newPath === oldPath) return t("rename.disabledUnchanged");
    if (preview.kind === "loading") return t("rename.computingPreview");
    if (preview.kind === "error") return t("rename.disabledPreviewFailed");
    if (preview.kind === "idle") return t("rename.disabledPreviewPending");
    if (conflictNeedsPolicy) return t("rename.disabledConflict");
    return null;
  })();
  const submitDisabled = disabledReason !== null;

  return (
    <Modal open onClose={onClose} width="lg" aria-labelledby={headingId}>
      <ModalHeader
        glyph={NF.edit}
        title={t("rename.title")}
        id={headingId}
        onClose={onClose}
      />
      <ModalBody style={{ display: "flex", flexDirection: "column", gap: "var(--sp-16)" }}>
        <FieldBlock label={t("rename.currentPath")}>
          <div
            className="mono selectable"
            style={{
              fontSize: "var(--fs-sm)",
              color: "var(--fg-muted)",
              padding: "var(--sp-6) var(--sp-10)",
              background: "var(--bg-sunken)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {oldPath}
          </div>
        </FieldBlock>

        <FieldBlock label={t("rename.newPath")} htmlFor="rename-new-path">
          <div style={{ display: "flex", gap: "var(--sp-6)", alignItems: "stretch" }}>
            <input
              id="rename-new-path"
              type="text"
              className="mono pm-focus"
              value={newPath}
              spellCheck={false}
              autoCapitalize="off"
              autoComplete="off"
              autoFocus
              onChange={(e) => setNewPath(e.target.value)}
              style={{
                flex: 1,
                padding: "var(--sp-6) var(--sp-10)",
                fontSize: "var(--fs-sm)",
                color: "var(--fg)",
                background: "var(--bg)",
                border: "var(--bw-hair) solid var(--line)",
                borderRadius: "var(--r-2)",
                outline: "none",
              }}
            />
            <IconButton
              glyph={NF.folder}
              title={t("rename.browseParent")}
              aria-label={t("rename.browseParent")}
              onClick={browseParent}
            />
          </div>
          <Hint>
            {t("rename.caseHint")}
          </Hint>
        </FieldBlock>

        <GroupCard label={t("rename.collisionLabel")}>
          <OptionRow
            type="radio"
            name="collision"
            checked={collision === "none"}
            onChange={() => setCollision("none")}
          >
            <strong style={{ fontWeight: 600 }}>{t("rename.collisionNone")}</strong> {t("rename.collisionNoneDesc")}
          </OptionRow>
          <OptionRow
            type="radio"
            name="collision"
            checked={collision === "merge"}
            onChange={() => setCollision("merge")}
          >
            <strong style={{ fontWeight: 600 }}>{t("rename.collisionMerge")}</strong> {t("rename.collisionMergeDesc")}
          </OptionRow>
          <OptionRow
            type="radio"
            name="collision"
            checked={collision === "overwrite"}
            onChange={() => setCollision("overwrite")}
          >
            <strong style={{ fontWeight: 600 }}>{t("rename.collisionOverwrite")}</strong> {t("rename.collisionOverwriteDesc")}
          </OptionRow>
        </GroupCard>

        <OptionRow
          type="checkbox"
          checked={noMove}
          onChange={(e) => setNoMove(e.target.checked)}
        >
          <strong style={{ fontWeight: 600 }}>{t("rename.stateOnly")}</strong> {t("rename.stateOnlyDesc")}
        </OptionRow>

        <GroupCard
          label={
            <span style={{ display: "inline-flex", alignItems: "center", gap: "var(--sp-5)", color: "var(--danger)" }}>
              <Glyph g={NF.warn} size="var(--fs-xs)" /> {t("rename.dangerZone")}
            </span>
          }
          tone="danger"
        >
          <OptionRow
            type="checkbox"
            checked={force}
            onChange={(e) => setForce(e.target.checked)}
          >
            <strong style={{ fontWeight: 600 }}>--force</strong> {t("rename.forceDesc")}
          </OptionRow>
          <OptionRow
            type="checkbox"
            checked={ignorePending}
            onChange={(e) => setIgnorePending(e.target.checked)}
          >
            <strong style={{ fontWeight: 600 }}>--ignore-pending-journals</strong> {t("rename.ignorePendingDesc")}
          </OptionRow>
        </GroupCard>

        <FieldBlock label={t("rename.previewLabel")}>
          <div
            aria-live="polite"
            style={{
              padding: "var(--sp-8) var(--sp-12)",
              background: "var(--bg-sunken)",
              border: "var(--bw-hair) solid var(--line)",
              borderRadius: "var(--r-2)",
              fontSize: "var(--fs-sm)",
              color: "var(--fg-muted)",
            }}
          >
            {preview.kind === "idle" && <span>{t("rename.previewIdle")}</span>}
            {preview.kind === "loading" && <span>{t("rename.computingPreview")}</span>}
            {preview.kind === "error" && (
              <div>
                <strong style={{ color: "var(--fg)", fontWeight: 600 }}>{t("rename.previewInvalid")}</strong>{" "}
                <span className="mono" style={{ fontSize: "var(--fs-xs)" }}>{preview.message}</span>
              </div>
            )}
            {preview.kind === "ok" && (
              <ul
                style={{
                  listStyle: "none",
                  margin: 0,
                  padding: 0,
                  display: "grid",
                  gap: "var(--sp-4)",
                }}
              >
                <li>
                  {preview.plan.would_move_dir
                    ? t("rename.willMoveDir")
                    : t("rename.wontMoveDir")}
                </li>
                <li>
                  {t("rename.ccDir")}{" "}
                  <code className="mono" style={{ fontSize: "var(--fs-xs)" }}>{preview.plan.old_cc_dir}</code>{" "}
                  → <code className="mono" style={{ fontSize: "var(--fs-xs)" }}>{preview.plan.new_cc_dir}</code>
                </li>
                <li>
                  {t("shared.sessions", { count: preview.plan.session_count })}
                  {", "}
                  {t("rename.jsonlToRewrite", {
                    count: preview.plan.estimated_jsonl_files,
                  })}
                </li>
                <li>
                  ~/.claude.json: {preview.plan.would_rewrite_claude_json ? t("shared.rewrite") : t("shared.skip")}
                </li>
                <li>
                  {t("rename.autoMemoryDir")} {preview.plan.would_move_memory_dir ? t("shared.move") : t("shared.skip")}
                </li>
                <li>
                  {t("rename.projectSettings")}{" "}
                  {preview.plan.would_rewrite_project_settings ? t("shared.rewrite") : t("shared.skip")}
                </li>
                <li>
                  {t("rename.pluginBindings")}{" "}
                  {preview.plan.would_rewrite_installed_plugins ? t("shared.repoint") : t("shared.skip")}
                </li>
                {preview.plan.estimated_history_lines > 0 && (
                  <li>
                    {t("rename.historyLines", {
                      n: preview.plan.estimated_history_lines,
                    })}
                  </li>
                )}
                {conflict && (
                  <li style={{ color: "var(--danger)" }}>
                    <strong style={{ fontWeight: 600 }}>{t("rename.conflictLabel")}</strong> {conflict}
                    {collision === "none" && (
                      <>
                        {" "}
                        <Trans
                          ns="projects"
                          i18nKey="rename.conflictPick"
                          components={{ m: <em />, o: <em /> }}
                        />
                      </>
                    )}
                  </li>
                )}
              </ul>
            )}
          </div>
        </FieldBlock>
      </ModalBody>
      <ModalFooter>
        <p
          style={{
            flex: 1,
            margin: 0,
            textAlign: "left",
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
          }}
        >
          {t("rename.approxNote")}
        </p>
        {submitDisabled && disabledReason && (
          <span
            style={{
              fontSize: "var(--fs-xs)",
              color: "var(--fg-faint)",
              fontStyle: "italic",
            }}
          >
            {disabledReason}
          </span>
        )}
        <Button variant="ghost" onClick={onClose}>
          {t("shared.cancel")}
        </Button>
        <Button
          variant="solid"
          disabled={submitDisabled}
          onClick={() => onSubmit(args)}
        >
          {t("rename.submit")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

function currentBasename(path: string): string {
  // Windows-aware via lib/paths — `C:\a\b` must yield `b`, not the
  // whole string (audit 2026-07 F2). Empty input stays "".
  return basename(path);
}

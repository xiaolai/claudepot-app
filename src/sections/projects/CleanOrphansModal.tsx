import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { Icon } from "../../components/Icon";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import { CopyButton } from "../../components/CopyButton";
import { Button } from "../../components/primitives/Button";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "../../components/primitives/Modal";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import type {
  CleanPreview,
  CleanResult,
  OperationProgressEvent,
  ProjectInfo,
} from "../../types";
import { formatSize } from "./format";
import { NF } from "../../icons";

type State =
  | { kind: "loading" }
  | { kind: "preview"; data: CleanPreview }
  | { kind: "running"; opId: string; phase: string; done: number; total: number }
  | { kind: "done"; result: CleanResult }
  | { kind: "error"; message: string };

/**
 * Confirm + execute dialog for `project clean`. Subscribes to
 * `op-progress::<opId>` once the clean task is started so the
 * user sees live "N of M" feedback instead of a mysterious spinner.
 *
 * Lifecycle:
 *   1. loading  — fetch preview on mount. User sees a skeleton.
 *   2. preview  — list of orphan candidates + unreachable skip note.
 *                 Confirm enabled only when the orphan count > 0.
 *   3. running  — progress bar driven by sub_progress events. The
 *                 backend emits two phases: `batch-sibling` (single
 *                 pass through history.jsonl + ~/.claude.json) and
 *                 `remove-dirs` (per-orphan remove_dir_all). We
 *                 surface the currently-active phase's progress.
 *   4. done     — counters panel + recovery snapshot paths.
 *   5. error    — backend error (journal gate, lock race, etc.).
 *
 * The dialog is dismissable in every state EXCEPT running. Running
 * is non-dismissable because the backend is holding the clean lock
 * and actively mutating disk; abandoning mid-run would leave
 * subsequent starts with a stale lock to break.
 */
export function CleanOrphansModal({
  onClose,
  onDone,
}: {
  onClose: () => void;
  onDone: (result: CleanResult) => void;
}) {
  const { t } = useTranslation("projects");
  const [state, setState] = useState<State>({ kind: "loading" });
  const headingId = useId();
  const firedTerminal = useRef(false);

  const loadPreview = useCallback(() => {
    setState({ kind: "loading" });
    firedTerminal.current = false;
    api
      .projectCleanPreview()
      .then((data) => setState({ kind: "preview", data }))
      .catch((e) => setState({ kind: "error", message: renderError(e) }));
  }, []);

  useEffect(() => {
    loadPreview();
  }, [loadPreview]);


  // Subscribe to progress events only while a clean is running.
  const channel =
    state.kind === "running" ? `op-progress::${state.opId}` : null;
  const opIdRef = useRef<string | null>(null);

  const handleEvent = useCallback(
    (event: { payload: OperationProgressEvent }) => {
      const ev = event.payload;
      if (ev.op_id !== opIdRef.current) return;

      if (ev.phase === "op") {
        if (firedTerminal.current) return;
        firedTerminal.current = true;
        const isComplete = ev.status === "complete";
        api
          .projectCleanStatus(ev.op_id)
          .then((info) => {
            if (isComplete && info?.clean_result) {
              setState({ kind: "done", result: info.clean_result });
              onDone(info.clean_result);
            } else if (isComplete) {
              // Terminal complete but poll missed the result — synthesize
              // an empty one so the UI doesn't wedge.
              const empty: CleanResult = {
                orphans_found: 0,
                orphans_removed: 0,
                orphans_skipped_live: 0,
                unreachable_skipped: 0,
                bytes_freed: 0,
                claude_json_entries_removed: 0,
                history_lines_removed: 0,
                claudepot_artifacts_removed: 0,
                snapshot_paths: [],
                protected_paths_skipped: 0,
              };
              setState({ kind: "done", result: empty });
              onDone(empty);
            } else {
              setState({
                kind: "error",
                message: ev.detail ?? info?.last_error ?? t("clean.failed"),
              });
            }
          })
          .catch(() => {
            setState({
              kind: "error",
              message: ev.detail ?? t("clean.failedUnreachable"),
            });
          });
        return;
      }

      // Phase + sub_progress updates. Only advance the surfaced phase
      // when we actually get a sub_progress tuple; pure status events
      // ("batch-sibling complete" without done/total) flip the phase
      // label and reset the counter.
      if (typeof ev.done === "number" && typeof ev.total === "number") {
        setState((prev) =>
          prev.kind === "running"
            ? {
                kind: "running",
                opId: prev.opId,
                phase: ev.phase,
                done: ev.done!,
                total: ev.total!,
              }
            : prev,
        );
      } else if (ev.status === "running") {
        setState((prev) =>
          prev.kind === "running"
            ? { ...prev, phase: ev.phase, done: 0, total: prev.total }
            : prev,
        );
      }
    },
    [onDone, t],
  );

  useTauriEvent<OperationProgressEvent>(channel, handleEvent);

  const runClean = () => {
    firedTerminal.current = false;
    api
      .projectCleanStart()
      .then((opId) => {
        opIdRef.current = opId;
        setState({
          kind: "running",
          opId,
          phase: "batch-sibling",
          done: 0,
          total: 0,
        });
      })
      .catch((e) => setState({ kind: "error", message: renderError(e) }));
  };

  const closeSafe = () => {
    if (state.kind === "running") return;
    onClose();
  };

  return (
    <Modal open onClose={closeSafe} width="lg" aria-labelledby={headingId}>
      <ModalHeader
        title={t("clean.title")}
        id={headingId}
        onClose={closeSafe}
      />
      <ModalBody>
        {state.kind === "loading" && <SkeletonPreview />}

        {state.kind === "preview" && (
          <Preview data={state.data} onRefresh={loadPreview} />
        )}

        {state.kind === "running" && (
          <RunningView
            phase={state.phase}
            done={state.done}
            total={state.total}
          />
        )}

        {state.kind === "done" && <Result result={state.result} />}

        {state.kind === "error" && (
          <div className="clean-error" role="alert">
            <Icon name="alert-triangle" size={14} />
            <div>
              <strong>{t("clean.errorHeading")}</strong>
              <p className="mono small">{state.message}</p>
              <p className="muted small">
                {t("clean.errorHint")}
              </p>
            </div>
          </div>
        )}
      </ModalBody>
      <ModalFooter>
        {state.kind === "done" ? (
          <Button variant="solid" onClick={closeSafe} autoFocus>
            {t("shared.close")}
          </Button>
        ) : (
          <>
            <Button
              variant="ghost"
              onClick={closeSafe}
              disabled={state.kind === "running"}
              title={
                state.kind === "running"
                  ? t("clean.cantCancelTitle")
                  : undefined
              }
            >
              {state.kind === "running"
                ? t("clean.running")
                : state.kind === "error"
                  ? t("shared.close")
                  : t("shared.cancel")}
            </Button>
            <Button
              variant="solid"
              danger
              disabled={
                !(state.kind === "preview" && state.data.orphans_found > 0)
              }
              onClick={runClean}
              glyph={NF.trash}
            >
              {state.kind === "preview" && state.data.orphans_found > 0
                ? t("clean.removeN", { count: state.data.orphans_found })
                : t("clean.remove")}
            </Button>
          </>
        )}
      </ModalFooter>
    </Modal>
  );
}

function SkeletonPreview() {
  return (
    <div className="skeleton-container">
      <div className="skeleton skeleton-header" />
      <div className="skeleton skeleton-card" />
    </div>
  );
}

function Preview({
  data,
  onRefresh,
}: {
  data: CleanPreview;
  onRefresh: () => void;
}) {
  const { t } = useTranslation("projects");
  if (data.orphans_found === 0 && data.unreachable_skipped === 0) {
    return (
      <div className="clean-empty">
        <p>{t("clean.nothing")}</p>
        <p className="muted small">
          {t("clean.nothingHint")}
        </p>
      </div>
    );
  }

  return (
    <>
      <p className="clean-summary">
        <Trans
          ns="projects"
          i18nKey="clean.summary"
          count={data.orphans_found}
          values={{ size: formatSize(data.total_bytes) }}
          components={{ n: <strong /> }}
        />
      </p>

      {data.unreachable_skipped > 0 && (
        <div className="clean-unreachable" role="status">
          <Icon name="wifi-off" size={14} />
          <span>
            <Trans
              ns="projects"
              i18nKey="clean.unreachableNote"
              count={data.unreachable_skipped}
              components={{ n: <strong /> }}
            />{" "}
            <button
              type="button"
              className="link-btn"
              onClick={onRefresh}
              title={t("clean.refreshTitle")}
            >
              {t("clean.refresh")}
            </button>
          </span>
        </div>
      )}

      {data.orphans_found > 0 && (
        <ul className="clean-orphan-list" aria-label={t("clean.toRemoveAria")}>
          {data.orphans.map((p) => (
            <OrphanRow key={p.sanitized_name} info={p} />
          ))}
        </ul>
      )}

      <p className="muted small clean-disclaimer">
        <Trans
          ns="projects"
          i18nKey="clean.disclaimer"
          components={{
            cj: <code>~/.claude.json</code>,
            hj: <code>history.jsonl</code>,
          }}
        />
      </p>

      {data.protected_count > 0 && (
        <p className="muted small clean-disclaimer">
          <Trans
            ns="projects"
            i18nKey="clean.protectedNote"
            count={data.protected_count}
            components={{
              n: <strong />,
              cj: <code>~/.claude.json</code>,
              hj: <code>history.jsonl</code>,
            }}
          />
        </p>
      )}
    </>
  );
}

function OrphanRow({ info }: { info: ProjectInfo }) {
  const { t } = useTranslation("projects");
  return (
    <li className="clean-orphan-row">
      <div className="clean-orphan-main">
        <span className="mono small selectable" title={info.original_path}>
          {info.original_path}
        </span>
        <span className="muted small">
          {t("shared.sessions", { count: info.session_count })} ·{" "}
          {formatSize(info.total_size_bytes)}
        </span>
      </div>
      {info.is_empty && (
        <span className="project-tag empty" title={t("clean.emptyDirTitle")}>
          <Icon name="circle-dashed" size={11} /> {t("status.empty")}
        </span>
      )}
    </li>
  );
}

function RunningView({
  phase,
  done,
  total,
}: {
  phase: string;
  done: number;
  total: number;
}) {
  const { t } = useTranslation("projects");
  const label =
    phase === "batch-sibling"
      ? t("clean.phaseBatchSibling")
      : phase === "remove-dirs"
        ? t("clean.phaseRemoveDirs")
        : t("clean.phaseCleaning");
  const pct =
    total > 0 ? Math.round((Math.min(done, total) / total) * 100) : 0;
  return (
    <div className="clean-running" role="status" aria-live="polite">
      <p>{label}…</p>
      {total > 0 ? (
        <>
          <div className="clean-progress-track" aria-hidden="true">
            <div
              className="clean-progress-fill"
              style={{ width: `${pct}%` }}
            />
          </div>
          <p className="muted small">
            {phase === "remove-dirs"
              ? t("clean.progressProjects", { done, total })
              : t("clean.progressSteps", { done, total })}
          </p>
        </>
      ) : (
        <div className="clean-spinner" aria-hidden="true" />
      )}
    </div>
  );
}

function Result({ result }: { result: CleanResult }) {
  const { t } = useTranslation("projects");
  return (
    <>
      <p className="clean-summary">
        <Trans
          ns="projects"
          i18nKey="clean.resultSummary"
          count={result.orphans_removed}
          values={{ size: formatSize(result.bytes_freed) }}
          components={{ n: <strong />, s: <strong /> }}
        />
      </p>

      {result.orphans_skipped_live > 0 && (
        <div className="clean-unreachable" role="status">
          <Icon name="alert-triangle" size={14} />
          <span>
            <Trans
              ns="projects"
              i18nKey="clean.skippedLive"
              count={result.orphans_skipped_live}
              components={{ n: <strong /> }}
            />
          </span>
        </div>
      )}

      <ul className="clean-result-list">
        {result.claude_json_entries_removed > 0 && (
          <li>
            {t("clean.prunedEntries", {
              count: result.claude_json_entries_removed,
            })}{" "}
            <code>~/.claude.json</code>
          </li>
        )}
        {result.history_lines_removed > 0 && (
          <li>
            {t("clean.removedLines", {
              count: result.history_lines_removed,
            })}{" "}
            <code>history.jsonl</code>
          </li>
        )}
        {result.claudepot_artifacts_removed > 0 && (
          <li>
            {t("clean.removedArtifacts", {
              count: result.claudepot_artifacts_removed,
            })}
          </li>
        )}
        {result.protected_paths_skipped > 0 && (
          <li>
            {t("clean.preservedProtected", {
              count: result.protected_paths_skipped,
            })}
          </li>
        )}
      </ul>

      {result.snapshot_paths.length > 0 && (
        <div className="clean-snapshots">
          <div className="field-label">{t("clean.snapshotsLabel")}</div>
          <p className="muted small">
            {t("clean.snapshotsHint")}
          </p>
          <ul className="clean-snapshot-list">
            {result.snapshot_paths.map((p) => (
              <li key={p}>
                <span className="mono small selectable">{p}</span>
                <CopyButton text={p} />
              </li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}

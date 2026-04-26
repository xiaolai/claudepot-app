import { useEffect, useId, useState } from "react";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import { Input } from "../../components/primitives/Input";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { NF } from "../../icons";
import type { RemoveProjectPreview, RemoveProjectResult } from "../../types";
import { formatRelativeTime, formatSize } from "./format";

/**
 * RemoveProjectModal — typed-confirmation gate for the destructive
 * `project_remove_execute` flow.
 *
 * Modal copy is the design (per the design discussion):
 * three blocks in this order — Removing, Not touching, Recoverable
 * until. The `Not touching` block repeats the cwd verbatim because
 * the user's actual fear is that this command deletes their real
 * project files.
 *
 * Affordance demotion: primary button is `Cancel` (solid). `Remove`
 * is `outline + danger`, disabled until the typed slug matches the
 * target slug. Live-session detection disables Remove with an inline
 * reason rather than offering it.
 */
export function RemoveProjectModal({
  target,
  onClose,
  onCompleted,
  onError,
}: {
  /** Path or slug of the project to remove. */
  target: string;
  onClose: () => void;
  onCompleted: (result: RemoveProjectResult) => void;
  onError: (msg: string) => void;
}) {
  const headingId = useId();
  const inputId = useId();
  const [preview, setPreview] = useState<RemoveProjectPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [confirmInput, setConfirmInput] = useState("");
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .projectRemovePreview(target)
      .then((p) => {
        if (cancelled) return;
        setPreview(p);
      })
      .catch((e) => {
        if (cancelled) return;
        setPreviewError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [target]);

  const matches = preview != null && confirmInput === preview.slug;
  const liveBlocked = preview?.has_live_session === true;
  const canSubmit = matches && !liveBlocked && !submitting;
  const disabledReason = liveBlocked
    ? "Live CC session running — close it first."
    : !matches && preview
      ? `Type ${preview.slug} to confirm.`
      : null;

  const recoverableUntil = (() => {
    const d = new Date();
    d.setDate(d.getDate() + 30);
    return d.toISOString().slice(0, 10);
  })();

  const handleSubmit = async () => {
    if (!canSubmit || !preview) return;
    setSubmitting(true);
    try {
      const result = await api.projectRemoveExecute(target);
      onCompleted(result);
    } catch (e) {
      onError(String(e));
      setSubmitting(false);
    }
  };

  return (
    <Modal open onClose={onClose} width="lg" aria-labelledby={headingId}>
      <ModalHeader
        glyph={NF.trash}
        title="Remove project"
        onClose={onClose}
        id={headingId}
      />
      <ModalBody
        style={{ display: "flex", flexDirection: "column", gap: "var(--sp-16)" }}
      >
        {previewError ? (
          <p
            style={{
              margin: 0,
              color: "var(--danger)",
              fontSize: "var(--fs-sm)",
            }}
          >
            {previewError}
          </p>
        ) : !preview ? (
          <p
            style={{
              margin: 0,
              color: "var(--fg-faint)",
              fontSize: "var(--fs-sm)",
            }}
          >
            Loading…
          </p>
        ) : (
          <>
            <Block label="Removing">
              <code
                style={{
                  fontSize: "var(--fs-sm)",
                  color: "var(--fg-base)",
                  wordBreak: "break-all",
                }}
              >
                {`~/.claude/projects/${preview.slug}/`}
              </code>
              <Meta items={metaItems(preview)} />
            </Block>

            <Block label="Not touching">
              <code
                style={{
                  fontSize: "var(--fs-sm)",
                  color: "var(--fg-base)",
                  wordBreak: "break-all",
                }}
              >
                {preview.original_path ?? "(unknown source path)"}
              </code>
              <p
                style={{
                  margin: 0,
                  fontSize: "var(--fs-xs)",
                  color: "var(--fg-faint)",
                }}
              >
                Your actual project files. Untouched.
              </p>
            </Block>

            <Block label={`Recoverable until ${recoverableUntil}`}>
              <p
                style={{
                  margin: 0,
                  fontSize: "var(--fs-xs)",
                  color: "var(--fg-faint)",
                }}
              >
                Lives in <code>~/.claudepot/trash/projects/</code> for 30 days.
                Restore via <code>claudepot project trash restore &lt;id&gt;</code>.
              </p>
            </Block>

            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: "var(--sp-6)",
              }}
            >
              <label
                htmlFor={inputId}
                style={{
                  fontSize: "var(--fs-xs)",
                  color: "var(--fg-muted)",
                  textTransform: "uppercase",
                  letterSpacing: "var(--tracking-wide)",
                }}
              >
                Type{" "}
                <code style={{ color: "var(--fg-base)" }}>{preview.slug}</code>{" "}
                to confirm
              </label>
              <Input
                value={confirmInput}
                onChange={(e) => setConfirmInput(e.target.value)}
                aria-label="Type project slug to confirm removal"
                autoFocus
                style={{ fontFamily: "var(--ff-mono)" }}
              />
            </div>
          </>
        )}
      </ModalBody>
      <ModalFooter>
        {disabledReason && (
          <span
            style={{
              flex: 1,
              fontSize: "var(--fs-xs)",
              color: liveBlocked ? "var(--danger)" : "var(--fg-faint)",
              fontStyle: "italic",
            }}
          >
            {disabledReason}
          </span>
        )}
        <Button variant="solid" onClick={onClose} autoFocus={!preview}>
          Cancel
        </Button>
        <Button
          variant="outline"
          danger
          disabled={!canSubmit}
          onClick={handleSubmit}
        >
          {submitting ? "Removing…" : "Remove"}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

function Block({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-4)",
        padding: "var(--sp-12)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
      }}
    >
      <div
        style={{
          fontSize: "var(--fs-xs)",
          color: "var(--fg-muted)",
          textTransform: "uppercase",
          letterSpacing: "var(--tracking-wide)",
        }}
      >
        {label}
      </div>
      {children}
    </div>
  );
}

function Meta({ items }: { items: string[] }) {
  if (items.length === 0) return null;
  return (
    <div
      style={{
        fontSize: "var(--fs-xs)",
        color: "var(--fg-faint)",
      }}
    >
      {items.join(" · ")}
    </div>
  );
}

function metaItems(p: RemoveProjectPreview): string[] {
  const items: string[] = [];
  if (p.session_count > 0) {
    items.push(`${p.session_count} session${p.session_count === 1 ? "" : "s"}`);
  }
  if (p.bytes > 0) {
    items.push(formatSize(p.bytes));
  }
  if (p.last_modified_ms != null) {
    items.push(`last touched ${formatRelativeTime(p.last_modified_ms)}`);
  }
  if (p.claude_json_entry_present) {
    items.push("with .claude.json entry");
  }
  if (p.history_lines_count > 0) {
    items.push(
      `${p.history_lines_count} history line${p.history_lines_count === 1 ? "" : "s"}`,
    );
  }
  return items;
}

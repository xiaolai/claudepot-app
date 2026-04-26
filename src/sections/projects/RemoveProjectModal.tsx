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
import type {
  RemoveProjectPreviewBasic,
  RemoveProjectPreviewExtras,
  RemoveProjectResult,
} from "../../types";
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
  const [basic, setBasic] = useState<RemoveProjectPreviewBasic | null>(null);
  const [extras, setExtras] = useState<RemoveProjectPreviewExtras | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [confirmInput, setConfirmInput] = useState("");
  const [submitting, setSubmitting] = useState(false);

  // Two-phase preview: the basic call lands in <50 ms (drives first
  // paint of the disclosure); the extras call backfills the live-
  // session block and the sibling-state annotations (can take 1-3 s
  // when history.jsonl is multi-MB or lsof is slow). Issued in
  // parallel so the slow probe doesn't gate the fast one.
  useEffect(() => {
    let cancelled = false;
    api
      .projectRemovePreviewBasic(target)
      .then((b) => {
        if (cancelled) return;
        setBasic(b);
      })
      .catch((e) => {
        if (cancelled) return;
        setPreviewError(String(e));
      });
    api
      .projectRemovePreviewExtras(target)
      .then((x) => {
        if (cancelled) return;
        setExtras(x);
      })
      .catch(() => {
        // Extras failure isn't fatal — the disclosure is already
        // rendered. The execute path's live-session refusal is the
        // backstop. Swallowing here keeps a slow-host hiccup from
        // blocking the user.
      });
    return () => {
      cancelled = true;
    };
  }, [target]);

  const matches = basic != null && confirmInput === basic.slug;
  const liveBlocked = extras?.has_live_session === true;
  // Submit is gated by the typed-slug match. Live-session detection
  // demotes the button when the slow probe lands; before then, we
  // optimistically allow click-through and rely on the execute path's
  // hard refusal as the safety net (the design's "live + slow probe"
  // race — never make the user wait on lsof).
  const canSubmit = matches && !liveBlocked && !submitting;
  const disabledReason = liveBlocked
    ? "Live CC session running — close it first."
    : !matches && basic
      ? `Type ${basic.slug} to confirm.`
      : null;

  const recoverableUntil = (() => {
    const d = new Date();
    d.setDate(d.getDate() + 30);
    return d.toISOString().slice(0, 10);
  })();

  const handleSubmit = async () => {
    if (!canSubmit || !basic) return;
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
        ) : !basic ? (
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
                {`~/.claude/projects/${basic.slug}/`}
              </code>
              <Meta items={metaItems(basic, extras)} />
            </Block>

            <Block label="Not touching">
              <code
                style={{
                  fontSize: "var(--fs-sm)",
                  color: "var(--fg-base)",
                  wordBreak: "break-all",
                }}
              >
                {basic.original_path ?? "(unknown source path)"}
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
                <code style={{ color: "var(--fg-base)" }}>{basic.slug}</code>{" "}
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
        <Button variant="solid" onClick={onClose} autoFocus={!basic}>
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

function metaItems(
  basic: RemoveProjectPreviewBasic,
  extras: RemoveProjectPreviewExtras | null,
): string[] {
  const items: string[] = [];
  if (basic.session_count > 0) {
    items.push(
      `${basic.session_count} session${basic.session_count === 1 ? "" : "s"}`,
    );
  }
  if (basic.bytes > 0) {
    items.push(formatSize(basic.bytes));
  }
  if (basic.last_modified_ms != null) {
    items.push(`last touched ${formatRelativeTime(basic.last_modified_ms)}`);
  }
  if (extras?.claude_json_entry_present) {
    items.push("with .claude.json entry");
  }
  if (extras && extras.history_lines_count > 0) {
    items.push(
      `${extras.history_lines_count} history line${extras.history_lines_count === 1 ? "" : "s"}`,
    );
  }
  return items;
}

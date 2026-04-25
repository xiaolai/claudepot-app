import { useId, useState } from "react";
import { Button } from "./primitives/Button";
import { Modal, ModalHeader, ModalBody, ModalFooter } from "./primitives/Modal";

/**
 * Consequence-explaining confirm dialog. Two variants:
 *
 * - Standard (default): renders consequence copy + Cancel/Confirm
 *   buttons. Used for Resume, Rollback, Break-lock — all reversible
 *   or auditable.
 * - Type-to-confirm (`typeToConfirm` prop): renders an input field
 *   that must match the expected string before the Confirm button
 *   enables. Used for Abandon, which destroys the audit trail.
 *
 * Built on the paper-mono `<Modal>` primitive. Destructive actions
 * intentionally suppress backdrop-click dismissal — Modal's default
 * scrim click would fire `onClose`, but for a destructive prompt that
 * means a stray click silently abandons the user's intent. Esc still
 * cancels via `onClose`, and the explicit Cancel button is the mouse
 * path. See `closeOnBackdrop={false}` below.
 */
export function ConfirmDangerousAction({
  title,
  consequences,
  confirmLabel,
  onCancel,
  onConfirm,
  typeToConfirm,
  danger = true,
}: {
  title: string;
  /** React node so callers can render lists, code spans, etc. */
  consequences: React.ReactNode;
  confirmLabel: string;
  onCancel: () => void;
  onConfirm: () => void;
  /** When set, user must type this exact string before Confirm enables. */
  typeToConfirm?: string;
  /** Render Confirm in danger style. Defaults to true. */
  danger?: boolean;
}) {
  const [typed, setTyped] = useState("");
  const headingId = useId();
  const confirmDisabled =
    typeToConfirm !== undefined && typed !== typeToConfirm;

  return (
    <Modal
      open
      onClose={onCancel}
      closeOnBackdrop={false}
      aria-labelledby={headingId}
    >
      <ModalHeader title={title} id={headingId} onClose={onCancel} />
      <ModalBody>
        {consequences}
        {typeToConfirm !== undefined && (
          <div
            style={{
              marginTop: "var(--sp-16)",
              display: "flex",
              flexDirection: "column",
              gap: "var(--sp-6)",
            }}
          >
            <label
              htmlFor="type-to-confirm-input"
              style={{
                fontSize: "var(--fs-xs)",
                color: "var(--fg-muted)",
                letterSpacing: "var(--ls-wide)",
                textTransform: "uppercase",
              }}
            >
              Type{" "}
              <code
                style={{
                  fontFamily: "var(--font-mono, var(--font))",
                  color: "var(--fg)",
                  textTransform: "none",
                  letterSpacing: 0,
                  padding: "0 var(--sp-4)",
                  background: "var(--bg-sunken)",
                  border: "var(--bw-hair) solid var(--line)",
                  borderRadius: "var(--r-1)",
                }}
              >
                {typeToConfirm}
              </code>{" "}
              to confirm
            </label>
            <input
              id="type-to-confirm-input"
              type="text"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              autoFocus
              style={{
                height: "var(--input-height)",
                padding: "0 var(--sp-10)",
                background: "var(--bg-raised)",
                border: "var(--bw-hair) solid var(--line)",
                borderRadius: "var(--r-2)",
                fontFamily: "var(--font)",
                fontSize: "var(--fs-sm)",
                color: "var(--fg)",
                outline: "none",
              }}
              className="pm-focus"
            />
          </div>
        )}
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          variant="solid"
          danger={danger}
          disabled={confirmDisabled}
          onClick={onConfirm}
          autoFocus={typeToConfirm === undefined}
        >
          {confirmLabel}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

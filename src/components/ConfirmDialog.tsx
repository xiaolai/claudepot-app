import React, { useId } from "react";
import { Button } from "./primitives/Button";
import { Modal, ModalHeader, ModalBody, ModalFooter } from "./primitives/Modal";

/**
 * Lightweight yes/no confirm. Built on the paper-mono `<Modal>`
 * primitive, which provides the backdrop, Escape-to-close, focus
 * trap, initial-focus, and focus-restore for free. Use for
 * non-destructive confirmations; reach for `ConfirmDangerousAction`
 * when the confirm has a type-to-confirm gate.
 */
export function ConfirmDialog({
  title,
  body,
  confirmLabel = "Confirm",
  confirmDanger = false,
  onCancel,
  onConfirm,
}: {
  title: string;
  body: React.ReactNode;
  confirmLabel?: string;
  confirmDanger?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const titleId = useId();
  return (
    <Modal open onClose={onCancel} aria-labelledby={titleId}>
      <ModalHeader title={title} id={titleId} onClose={onCancel} />
      <ModalBody>{body}</ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          variant="solid"
          danger={confirmDanger}
          onClick={onConfirm}
          autoFocus
        >
          {confirmLabel}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

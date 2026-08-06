import React, { useId } from "react";
import { useTranslation } from "react-i18next";
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
  confirmLabel,
  confirmDanger = false,
  onCancel,
  onConfirm,
}: {
  title: string;
  body: React.ReactNode;
  /** Defaults to the localized "Confirm" — resolved at render so a
   *  language switch applies without remounting the dialog. */
  confirmLabel?: string;
  confirmDanger?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { t } = useTranslation("components");
  const titleId = useId();
  return (
    <Modal open onClose={onCancel} aria-labelledby={titleId}>
      <ModalHeader title={title} id={titleId} onClose={onCancel} />
      <ModalBody>{body}</ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onCancel}>
          {t("modals.cancel")}
        </Button>
        <Button
          variant="solid"
          danger={confirmDanger}
          onClick={onConfirm}
          autoFocus
        >
          {confirmLabel ?? t("modals.confirm")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

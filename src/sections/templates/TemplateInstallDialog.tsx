import { Modal } from "../../components/primitives/Modal";
import { TemplateInstallView } from "./TemplateInstallView";

interface Props {
  open: boolean;
  templateId: string | null;
  onClose: () => void;
  onInstalled: () => void;
  onError: (msg: string) => void;
  onOpenThirdParties: () => void;
}

/**
 * Standalone install-template dialog. Thin wrapper around
 * `Modal` + `TemplateInstallView`. Used when an install flow
 * is invoked outside the gallery (e.g. a future "from a deep
 * link" path).
 *
 * Inside the gallery, `TemplateGallery` mounts
 * `TemplateInstallView` directly inside its own Modal so the
 * gallery → install transition is a content swap, not a
 * close-then-open of two separate Modals (which caused a
 * visible backdrop flash).
 */
export function TemplateInstallDialog({
  open,
  templateId,
  onClose,
  onInstalled,
  onError,
  onOpenThirdParties,
}: Props) {
  if (!open || !templateId) return null;
  return (
    <Modal
      open={open}
      onClose={onClose}
      width="lg"
      aria-labelledby="template-install-title"
    >
      <TemplateInstallView
        templateId={templateId}
        onBack={onClose}
        onInstalled={() => {
          onInstalled();
          onClose();
        }}
        onError={onError}
        onOpenThirdParties={onOpenThirdParties}
      />
    </Modal>
  );
}

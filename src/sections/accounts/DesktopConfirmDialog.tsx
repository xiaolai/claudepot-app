import { ConfirmDialog } from "../../components/ConfirmDialog";
import type { DesktopConfirmRequest } from "../../providers/AppStateProvider";

interface Props {
  request: DesktopConfirmRequest;
  onCancel: () => void;
  onConfirm: () => void;
}

/**
 * Destructive-Desktop-action confirmations. Codex follow-up review
 * (thread 019db814-a45b-7fa3-a280-80b1f20e1149) flagged that
 * "Sign Desktop out" and adopt-overwrite were one-click destructive
 * with no confirmation. The same copy runs through the tray, context
 * menu, and palette surfaces so the user sees the same trade-off
 * regardless of entry point.
 */
export function DesktopConfirmDialog({ request, onCancel, onConfirm }: Props) {
  switch (request.kind) {
    case "sign_out":
      return (
        <ConfirmDialog
          title="Sign Claude Desktop out?"
          confirmLabel="Sign out"
          confirmDanger
          body={
            <>
              <p>
                Claudepot will quit Claude Desktop and delete the live
                session items. The current snapshot is preserved, so you
                can swap this account back in later.
              </p>
              <ul className="muted small" style={{ paddingLeft: 18 }}>
                <li>Any unsent Desktop chat drafts will be lost.</li>
                <li>
                  Next Desktop launch will open to the login screen until
                  you sign in again.
                </li>
                <li>
                  The snapshot under <code>~/.claudepot/desktop/</code>
                  {" "}stays intact.
                </li>
              </ul>
            </>
          }
          onCancel={onCancel}
          onConfirm={onConfirm}
        />
      );
    case "overwrite_profile":
      return (
        <ConfirmDialog
          title={`Replace Desktop profile for ${request.account.email}?`}
          confirmLabel="Replace"
          confirmDanger
          body={
            <>
              <p>
                <strong>{request.account.email}</strong> already has a
                stored Desktop snapshot. Binding the live session will
                overwrite it with the current state.
              </p>
              <p className="muted small">
                The previous snapshot is stashed to a temporary directory
                during the copy; if the new snapshot fails, the old one
                is rolled back. On success, the old snapshot is
                permanently discarded.
              </p>
            </>
          }
          onCancel={onCancel}
          onConfirm={onConfirm}
        />
      );
  }
}

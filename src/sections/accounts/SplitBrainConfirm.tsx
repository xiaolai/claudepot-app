import { ConfirmDialog } from "../../components/ConfirmDialog";
import type { AccountSummary } from "../../types";

interface Props {
  account: AccountSummary;
  onCancel: () => void;
  onConfirm: () => void;
}

/**
 * Pre-swap split-brain warning. Surfaces the same three-bullet
 * advisory the CLI prints *after* a `--force` swap when CC is
 * running — except the GUI raises it *before* the swap so the user
 * makes the trade-off knowingly rather than recovering from it.
 */
export function SplitBrainConfirm({ account, onCancel, onConfirm }: Props) {
  return (
    <ConfirmDialog
      title="Claude Code is running"
      confirmLabel={`Swap to ${account.email} anyway`}
      confirmDanger
      body={
        <>
          <p>
            A running Claude Code session is using the current account.
            Until you quit it you'll see split-brain state:
          </p>
          <ul className="muted small" style={{ paddingLeft: 18 }}>
            <li>
              Session identity (header, org name) stays as the old
              account — cached at startup.
            </li>
            <li>
              API calls (/usage, completions, billing) switch to{" "}
              <strong>{account.email}</strong> immediately.
            </li>
            <li>
              The next OAuth refresh (typically within the hour) may
              overwrite the keychain back to the old account, silently
              reverting this swap.
            </li>
          </ul>
          <p className="muted small">
            Safest: quit Claude Code first, then swap. This action
            proceeds with <code>--force</code>.
          </p>
        </>
      }
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}

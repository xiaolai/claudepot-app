import { CommandPalette } from "./CommandPalette";
import { ErrorBoundary } from "../ErrorBoundary";
import { api } from "../api";
import { toastError } from "../lib/toastError";
import { useAppState } from "../providers/AppStateProvider";

/**
 * Shell mount point for the ⌘K command palette, extracted from
 * AppShell. Reads account state + action helpers straight from
 * AppStateProvider; the shell passes only its own concerns (open
 * state, navigation, the shortcuts modal trigger).
 *
 * Wrapped in a scoped ErrorBoundary: the palette is fed by backend
 * data (accounts, status, session search), making it a realistic
 * crash surface — without the boundary a render crash here would
 * bubble to main.tsx's full-takeover reload panel.
 */
export function ShellCommandPalette(props: {
  open: boolean;
  onClose: () => void;
  onNavigate: (id: string) => void;
  onShowShortcuts: () => void;
}) {
  const {
    accounts,
    status: appStatus,
    refresh: refreshAccounts,
    pushToast,
    actions,
    requestCliSwap,
    requestDesktopSignOut,
    requestDesktopOverwrite,
    requestRemoveAccount,
  } = useAppState();

  if (!props.open || !appStatus) return null;

  return (
    <ErrorBoundary label="Command palette">
      <CommandPalette
        accounts={accounts}
        status={appStatus}
        onClose={props.onClose}
        onSwitchCli={(a) => void requestCliSwap(a)}
        onSwitchDesktop={(a) => void actions.useDesktop(a)}
        onAdd={() => {
          props.onNavigate("accounts");
          window.dispatchEvent(new CustomEvent("cp-open-add"));
        }}
        onRefresh={() => void refreshAccounts()}
        onRemove={(a) => requestRemoveAccount(a)}
        onAdoptDesktop={(a) => {
          if (a.desktop_profile_on_disk) requestDesktopOverwrite(a);
          else void actions.adoptDesktop(a);
        }}
        onClearDesktop={requestDesktopSignOut}
        onLaunchDesktop={() => {
          api.desktopLaunch().catch((e) => {
            toastError(pushToast, "Desktop launch failed", e);
          });
        }}
        onNavigate={props.onNavigate}
        onShowShortcuts={props.onShowShortcuts}
      />
    </ErrorBoundary>
  );
}

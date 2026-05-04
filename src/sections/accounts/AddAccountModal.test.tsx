import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";

import { AddAccountModal } from "./AddAccountModal";
import {
  OperationsContext,
  OperationsProvider,
} from "../../hooks/useOperations";

// Mock the api module — we only exercise the calls this modal uses.
// `accountRegisterFromBrowserStart` replaces the old synchronous
// `accountRegisterFromBrowser` (C-1 fix wave); the legacy entry point
// stays mocked so any unmigrated callsites still resolve.
vi.mock("../../api", () => {
  return {
    api: {
      currentCcIdentity: vi.fn(),
      accountAddFromCurrent: vi.fn(),
      accountRegisterFromBrowser: vi.fn(),
      accountRegisterFromBrowserStart: vi.fn(),
      accountLoginStatus: vi.fn(),
      accountLoginCancel: vi.fn(),
    },
  };
});

// Mock the Tauri event bus so the OperationProgressModal subscribed by
// the shell doesn't error inside jsdom when the start path fires.
vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

// Re-import after the mock is set up so test bodies can poke at the mocks.
// eslint-disable-next-line import/first
import { api } from "../../api";

const mockApi = api as unknown as {
  currentCcIdentity: ReturnType<typeof vi.fn>;
  accountAddFromCurrent: ReturnType<typeof vi.fn>;
  accountRegisterFromBrowser: ReturnType<typeof vi.fn>;
  accountRegisterFromBrowserStart: ReturnType<typeof vi.fn>;
  accountLoginStatus: ReturnType<typeof vi.fn>;
  accountLoginCancel: ReturnType<typeof vi.fn>;
};

beforeEach(() => {
  mockApi.currentCcIdentity.mockReset();
  mockApi.accountAddFromCurrent.mockReset();
  mockApi.accountRegisterFromBrowser.mockReset();
  mockApi.accountRegisterFromBrowserStart.mockReset();
  mockApi.accountLoginStatus.mockReset();
  mockApi.accountLoginCancel.mockReset();
  // Neutral preflight — no current CC session, so Import is disabled and
  // the browser login is the only active button.
  mockApi.currentCcIdentity.mockResolvedValue({ email: null, error: null });
});

/** Render `AddAccountModal` inside the operations provider so the
 *  `useOperations` call in the new browser-login start flow has a
 *  context to read from. */
function renderWithOps(ui: ReactElement) {
  return render(<OperationsProvider>{ui}</OperationsProvider>);
}

describe("AddAccountModal — browser login (async start)", () => {
  it("hands off to the shell op modal once the start call returns", async () => {
    // The new flow is non-blocking: `accountRegisterFromBrowserStart`
    // returns an op_id immediately; phase events flow on
    // `op-progress::<op_id>` and the shell-level OperationProgressModal
    // owns the user-visible surface from there. The AddAccountModal
    // dismisses itself.
    mockApi.accountRegisterFromBrowserStart.mockResolvedValue("op-fake");
    const onClose = vi.fn();

    renderWithOps(
      <AddAccountModal
        open
        onClose={onClose}
        onAdded={() => {}}
        onError={() => {}}
        accounts={[]}
        onAdoptDesktop={() => Promise.resolve(true)}
      />,
    );

    const loginBtn = await screen.findByRole("button", { name: /log in/i });
    await userEvent.click(loginBtn);

    await waitFor(() =>
      expect(mockApi.accountRegisterFromBrowserStart).toHaveBeenCalledOnce(),
    );
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("wires onCancel through the op handle so the shell modal can cancel the login", async () => {
    // The shell `OperationProgressModal` owns the Cancel button and
    // calls `onCancel` when the user clicks it. AddAccountModal must
    // pass a handler that hits `accountLoginCancel` so the backend
    // unwinds the in-flight `claude auth login`.
    mockApi.accountRegisterFromBrowserStart.mockResolvedValue("op-fake");
    mockApi.accountLoginCancel.mockResolvedValue(undefined);

    const captured: { onCancel?: () => void; cancelLabel?: string } = {};
    const TestProvider = ({ children }: { children: ReactElement }) => {
      const value = {
        active: null,
        open: (handle: {
          onCancel?: () => void;
          cancelLabel?: string;
        }) => {
          captured.onCancel = handle.onCancel;
          captured.cancelLabel = handle.cancelLabel;
        },
        close: () => {},
      };
      return (
        <OperationsContext.Provider value={value as never}>
          {children}
        </OperationsContext.Provider>
      );
    };

    render(
      <TestProvider>
        <AddAccountModal
          open
          onClose={() => {}}
          onAdded={() => {}}
          onError={() => {}}
          accounts={[]}
          onAdoptDesktop={() => Promise.resolve(true)}
        />
      </TestProvider>,
    );

    const loginBtn = await screen.findByRole("button", { name: /log in/i });
    await userEvent.click(loginBtn);

    await waitFor(() => expect(captured.onCancel).toBeTypeOf("function"));
    expect(captured.cancelLabel).toBe("Cancel login");

    captured.onCancel?.();
    await waitFor(() =>
      expect(mockApi.accountLoginCancel).toHaveBeenCalledOnce(),
    );
  });

  it("surfaces non-cancel errors via onError when the start call rejects", async () => {
    mockApi.accountRegisterFromBrowserStart.mockRejectedValue(
      new Error("register failed: claude binary not found"),
    );
    const onError = vi.fn();

    renderWithOps(
      <AddAccountModal
        open
        onClose={() => {}}
        onAdded={() => {}}
        onError={onError}
        accounts={[]}
        onAdoptDesktop={() => Promise.resolve(true)}
      />,
    );

    const loginBtn = await screen.findByRole("button", { name: /log in/i });
    await userEvent.click(loginBtn);

    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(
        expect.stringMatching(/claude binary not found/),
      ),
    );
  });

  it("swallows the cancelled-error toast when the start call reports cancellation", async () => {
    mockApi.accountRegisterFromBrowserStart.mockRejectedValue(
      new Error(
        "register failed: claude auth login was cancelled by the user",
      ),
    );
    const onError = vi.fn();

    renderWithOps(
      <AddAccountModal
        open
        onClose={() => {}}
        onAdded={() => {}}
        onError={onError}
        accounts={[]}
        onAdoptDesktop={() => Promise.resolve(true)}
      />,
    );

    const loginBtn = await screen.findByRole("button", { name: /log in/i });
    await userEvent.click(loginBtn);

    await waitFor(() =>
      expect(mockApi.accountRegisterFromBrowserStart).toHaveBeenCalledOnce(),
    );
    expect(onError).not.toHaveBeenCalled();
  });
});
